//! Update-service discovery, health probing and status sampling.
//!
//! The launcher never talks to the update service from the webview: the
//! capability whitelist for `http:` is a compile-time list of hosts, and the
//! service address is resolved at runtime from a DNS SRV record. Everything
//! therefore lives here, behind three thin Tauri commands.
//!
//! Rules this module keeps:
//!
//! * Every remote failure degrades to `Ok(None)` / an empty list. Nothing here
//!   panics, returns an error to the caller, or blocks start-up: local features
//!   (launching, instances, downloads, skins) never go through this path.
//! * DNS answers are cached, 300 s on success and 30 s on failure, matching the
//!   download resolver's policy in [`crate::util::download_dns`].
//! * Two failures in a row are needed before the status reads offline; a single
//!   success brings it back, so a flapping link does not make the indicator
//!   blink.
//! * Only `https` is accepted for the resolved address. There is no `http`
//!   downgrade: a plaintext update feed can be rewritten in transit, and the
//!   savings are not worth it. The one exception is the loopback interface,
//!   which is where the local stub server for manual testing runs.
//! * Certificates are validated strictly and there is no way to relax that.
//!   An untrusted chain is a hard failure, reported once per
//!   [`CERT_WARNING_INTERVAL`] instead of on every probe, so a self-signed
//!   deployment shows up in the log rather than failing silently.
//! * Address-family selection is left to the HTTP client. The server is
//!   dual-stack but its DNS currently publishes only `A` records, so "no
//!   `AAAA`, use `A`" is the ordinary path, not an anomaly: nothing here warns
//!   about it, and a missing `AAAA` never lowers the status or forces a retry.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use url::Url;

pub use crate::brand::{
    UPDATE_SERVICE_BASE_ENV, UPDATE_SERVICE_CLOUDFLARE_BASE,
    UPDATE_SERVICE_FALLBACK_BASE, UPDATE_SERVICE_HEALTH_PATH,
    UPDATE_SERVICE_HOST, UPDATE_SERVICE_IPV6_PORT, UPDATE_SERVICE_SRV,
};

/// Log target for everything this module reports.
///
/// The module path would be `theseus::api::update_service`; the shorter target
/// is what the logging filter and the support docs use.
pub const LOG_TARGET: &str = "theseus::update_service";

/// How long a successful SRV answer stays usable.
const SRV_SUCCESS_TTL: Duration = Duration::from_secs(300);
/// How long a failed lookup is remembered before DNS is asked again.
const SRV_FAILURE_TTL: Duration = Duration::from_secs(30);
/// Upper bound for one SRV lookup, so a silent resolver cannot stall a probe.
const SRV_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// A `/health` request gives up on the TCP/TLS handshake after this long.
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// ...and on the whole request after this long.
const PROBE_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);

/// Timeouts for the business endpoints proxied to the frontend.
///
/// Slightly more generous than the health probe: these responses carry real
/// payloads (a moderation feed, an update manifest), so a slow link should be
/// given a little room, while still finishing well inside a user's patience.
const API_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const API_TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

/// Retry delays inside one probe round. The round is declared failed after the
/// last one, which is what turns two failures in a row into an offline status.
const PROBE_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_secs(1), Duration::from_secs(2)];

/// Failures in a row before the status flips to offline.
const FAILURES_BEFORE_OFFLINE: u8 = 2;
/// Number of recent round-trip times the displayed latency is taken from.
const RTT_WINDOW: usize = 5;

/// How often an unverifiable certificate may be reported.
///
/// A wrong chain fails every probe, and the probe runs every 30 s: without this
/// the log would carry 120 identical warnings an hour and bury everything else.
const CERT_WARNING_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// A new latency is published only when it differs by at least this much.
///
/// Without a threshold the sampler would push an event every 30 s for a couple
/// of milliseconds of jitter, and each event re-renders the title bar.
pub const RTT_PUBLISH_THRESHOLD_MS: u64 = 5;

/// Capacity of the status channel; only the newest values matter.
const STATUS_CHANNEL_CAPACITY: usize = 32;

// ── Public types ────────────────────────────────────────────────

/// What the launcher currently believes about the update service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateServiceState {
    /// No probe has succeeded or failed enough times to decide yet.
    #[default]
    Unknown,
    /// The last `/health` request (or the one before it) succeeded.
    Online,
    /// At least two `/health` requests in a row failed.
    Offline,
}

/// Why the last probe failed, in the only three categories the UI shows.
///
/// The frontend never receives an error string: a raw TLS/DNS message is
/// unreadable in a tooltip and leaks network details, so the classification
/// happens here and the shell only translates a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeFailure {
    /// The certificate chain could not be verified (strict validation failed).
    Certificate,
    /// The request did not finish inside the probe budget.
    Timeout,
    /// Anything else: refused, reset, DNS, non-2xx, unreadable body.
    Unreachable,
}


// ── Connection tiers ────────────────────────────────────────────

/// Failures on the active tier before the next one is tried.
///
/// Two, not one: a single dropped packet must not move the launcher onto a
/// slower path (the Cloudflare tunnel is an order of magnitude behind a direct
/// connection).
const FAILURES_BEFORE_SWITCH: u8 = 2;

/// Upper bound on how long an SRV answer is reused.
///
/// The record carries a STUN-derived port that changes whenever the tunnel is
/// re-established, so it is refreshed far more often than the 300 s cap the
/// design allows: two minutes keeps a stale port from costing more than a couple
/// of probes.
const SRV_SUCCESS_TTL_CAP: Duration = Duration::from_secs(300);

/// How long a resolved IPv6 address is reused.
const IPV6_SUCCESS_TTL: Duration = Duration::from_secs(60);

/// How often the best tier is re-tested while a lower one is in use.
const SRV_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Redirects followed before a tier is declared unusable.
const MAX_REDIRECT_HOPS: usize = 5;

/// Markers that identify a Cloudflare Access gate rather than the service.
const ACCESS_MARKERS: [&str; 4] = [
    "cloudflareaccess.com",
    "/cdn-cgi/access/",
    "cf-access",
    // The login page can answer 200 with the gate rendered in the body.
    "cloudflare access",
];

/// TLS failures that mean "this listener does not speak what we speak".
///
/// Deliberately excludes anything certificate-shaped: a broken chain must never
/// be retried over plaintext.
const TLS_PROTOCOL_HINTS: [&str; 4] = [
    "tls alert",
    "handshake failure",
    "protocol version",
    "unsupported protocol",
];

/// Which connection path is in use, highest priority first.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum UpdateServiceTier {
    /// Nothing is reachable (or nothing has been resolved yet).
    #[default]
    None,
    /// DNS SRV record: reverse proxy on the STUN-derived port.
    Srv,
    /// IPv6 literal on the fixed port.
    Ipv6,
    /// Cloudflare tunnel, the last resort.
    Cloudflare,
    /// `HAYENAI_UPDATE_BASE` pinned the address (stub server / debugging).
    ///
    /// Reported as its own value instead of being folded into one of the three:
    /// "why is it online when everything is down" must have an obvious answer
    /// in the tooltip.
    Override,
}

impl UpdateServiceTier {
    /// Stable name for logs and the wire format.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Srv => "srv",
            Self::Ipv6 => "ipv6",
            Self::Cloudflare => "cloudflare",
            Self::Override => "override",
        }
    }
}

/// One resolved base URL, in the order the tiers should be tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierCandidate {
    /// Which path this base belongs to.
    pub tier: UpdateServiceTier,
    /// Base URL, always ending with `/`.
    pub base: String,
    /// Set when the base was downgraded to plaintext after a TLS refusal.
    pub insecure: bool,
}

/// Builds the base URL for an IPv6 endpoint.
pub fn ipv6_base(address: &std::net::Ipv6Addr) -> String {
    format!("https://[{address}]:{UPDATE_SERVICE_IPV6_PORT}/")
}

/// The ordered candidate list: SRV, IPv6, Cloudflare.
///
/// An override short-circuits the list with a single locked entry, because a
/// pointed-at stub server must not be replaced by a real one mid-test.
pub fn build_candidates(
    override_base: Option<&str>,
    srv: Option<&ServiceEndpoint>,
    ipv6: Option<std::net::Ipv6Addr>,
) -> Vec<TierCandidate> {
    if let Some(raw) = override_base {
        match normalize_base(raw) {
            Some(base) => {
                return vec![TierCandidate {
                    tier: UpdateServiceTier::Override,
                    base,
                    insecure: false,
                }];
            }
            None => {
                tracing::warn!(
                    target: LOG_TARGET,
                    "Ignoring {UPDATE_SERVICE_BASE_ENV}: it is not a usable https base URL"
                );
            }
        }
    }

    let mut candidates = Vec::with_capacity(3);
    if let Some(endpoint) = srv {
        candidates.push(TierCandidate {
            tier: UpdateServiceTier::Srv,
            base: base_url_for(Some(endpoint)),
            insecure: false,
        });
    }
    if let Some(address) = ipv6 {
        candidates.push(TierCandidate {
            tier: UpdateServiceTier::Ipv6,
            base: ipv6_base(&address),
            insecure: false,
        });
    }
    // Always last, always present: something has to answer when the direct
    // paths are unreachable, and a failed tier is cheap to detect.
    candidates.push(TierCandidate {
        tier: UpdateServiceTier::Cloudflare,
        base: UPDATE_SERVICE_CLOUDFLARE_BASE.to_string(),
        insecure: false,
    });
    candidates
}

/// Index of the tier to probe next.
///
/// Staying on the current index until [`FAILURES_BEFORE_SWITCH`] failures is
/// what keeps the indicator from bouncing; once the last tier is reached there
/// is nowhere left to go, so the probe keeps reporting the failure.
pub fn advance_tier(current: usize, failures: u8, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let current = current.min(len - 1);
    if failures >= FAILURES_BEFORE_SWITCH && current + 1 < len {
        current + 1
    } else {
        current
    }
}

/// Whether a response is a Cloudflare Access gate instead of the service.
///
/// Access answers with a redirect to `*.cloudflareaccess.com`, and a tunnel
/// without an ingress rule answers 404 — both look like "the host is up". Only
/// an actual health payload counts, which is why this is checked before the
/// status code is trusted.
pub fn is_access_gate(location: Option<&str>, body: &str) -> bool {
    if let Some(location) = location {
        let lowered = location.to_ascii_lowercase();
        if ACCESS_MARKERS.iter().any(|marker| lowered.contains(marker)) {
            return true;
        }
    }
    let lowered = body.to_ascii_lowercase();
    ACCESS_MARKERS.iter().any(|marker| lowered.contains(marker))
}

/// Whether an error message describes a TLS protocol refusal, not a bad chain.
pub fn is_tls_protocol_failure(message: &str) -> bool {
    if looks_like_certificate_failure(message) {
        return false;
    }
    let lowered = message.to_ascii_lowercase();
    TLS_PROTOCOL_HINTS
        .iter()
        .any(|hint| lowered.contains(hint))
}

/// The plaintext twin of an `https` base, or `None` when it already is one.
pub fn insecure_variant(base: &str) -> Option<String> {
    base.strip_prefix("https://")
        .map(|rest| format!("http://{rest}"))
}

/// Resolves a `Location` header into the base the next request should use.
///
/// The health path is stripped when it is part of the target, so a 307 that
/// points straight at `/api/health` on another host yields that host's root
/// rather than a base that would double the path.
pub fn redirect_base(current_base: &str, location: &str) -> Option<String> {
    let base = Url::parse(current_base).ok()?;
    let target = base.join(location.trim()).ok()?;
    if !matches!(target.scheme(), "http" | "https") {
        return None;
    }
    let host = target.host_str()?;
    let authority_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };

    let mut path = target.path().to_string();
    if let Some(prefix) = path.strip_suffix(UPDATE_SERVICE_HEALTH_PATH) {
        path = prefix.to_string();
    }
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    if !path.ends_with('/') {
        path.push('/');
    }

    let mut next = String::from(target.scheme());
    next.push_str("://");
    next.push_str(&authority_host);
    if let Some(port) = target.port() {
        next.push_str(&format!(":{port}"));
    }
    next.push_str(&path);
    Some(next)
}

/// A successful health probe.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeOutcome {
    /// Base actually used after redirects; adopted for later requests.
    base: String,
    /// Full URL that answered, for diagnostics only.
    final_url: String,
    rtt_ms: u64,
    /// True when the tier had to fall back to plaintext.
    insecure: bool,
}

/// Internal probe failure, one step finer than [`ProbeFailure`].
///
/// `TlsProtocol` is the only category that is retried over plaintext, so it must
/// stay distinguishable from the categories the UI sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseProbeError {
    Certificate,
    TlsProtocol,
    Timeout,
    Unreachable,
}

impl BaseProbeError {
    /// The category reported to the tracker and the frontend.
    fn as_failure(self) -> ProbeFailure {
        match self {
            Self::Certificate => ProbeFailure::Certificate,
            Self::Timeout => ProbeFailure::Timeout,
            Self::TlsProtocol | Self::Unreachable => ProbeFailure::Unreachable,
        }
    }
}

/// In-memory snapshot handed to the frontend.
///
/// This is a copy, never a handle: the command that returns it does not perform
/// any network request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateServiceStatus {
    /// Current state of the service as far as the sampler can tell.
    pub state: UpdateServiceState,
    /// Median of the recent successful round-trip times, in milliseconds.
    pub rtt_ms: Option<u64>,
    /// Base URL in use, or `None` before the first resolution.
    pub base_url: Option<String>,
    /// Unix timestamp (seconds) of the last successful probe.
    pub last_ok_at: Option<u64>,
    /// Category of the last failure, cleared by the next success.
    pub last_failure: Option<ProbeFailure>,
    /// Which connection path is in use.
    pub tier: UpdateServiceTier,
    /// True when the tier had to fall back to plaintext.
    pub insecure: bool,
}

/// One resolved endpoint of the update service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEndpoint {
    /// Host name, without the trailing dot DNS uses for absolute names.
    pub target: String,
    /// Port the reverse proxy listens on.
    pub port: u16,
}

/// One SRV answer, reduced to the fields the selection rule needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvAnswer {
    /// Lower wins; see RFC 2782.
    pub priority: u16,
    /// Higher wins among equal priorities.
    pub weight: u16,
    /// Host name the record points at.
    pub target: String,
    /// Port the record points at; `0` means "service not available".
    pub port: u16,
}

// ── SRV selection and base URL building ─────────────────────────

/// Strips the trailing dot from an absolute DNS name.
///
/// `hickory-resolver` formats absolute names with a trailing dot, which would
/// end up in the URL as `https://host.:5443/`.
fn normalize_target(target: &str) -> String {
    target.trim().trim_end_matches('.').to_string()
}

/// Picks the endpoint to use out of a set of SRV answers.
///
/// Selection follows the design contract: lowest priority first, then highest
/// weight, then the order the record set arrived in. Answers that mark the
/// service as unavailable (target `.` or port `0`, RFC 2782) are dropped before
/// selecting, so an explicit "gone" answer falls back to the constant base
/// rather than being dialled.
pub fn select_endpoint(answers: &[SrvAnswer]) -> Option<ServiceEndpoint> {
    let mut candidates: Vec<&SrvAnswer> = answers
        .iter()
        .filter(|answer| {
            let target = normalize_target(&answer.target);
            !target.is_empty() && answer.port != 0
        })
        .collect();

    // `sort_by` keeps the original order for equal keys, which is the round
    // robin tie-breaker the contract asks for.
    candidates.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| right.weight.cmp(&left.weight))
    });

    candidates.first().map(|answer| ServiceEndpoint {
        target: normalize_target(&answer.target),
        port: answer.port,
    })
}

/// Builds the base URL for an endpoint, or the fallback constant without one.
///
/// The result always ends with `/` so endpoints can be appended by plain
/// concatenation, and the port is omitted only when it is the HTTPS default.
pub fn base_url_for(endpoint: Option<&ServiceEndpoint>) -> String {
    let Some(endpoint) = endpoint else {
        return UPDATE_SERVICE_FALLBACK_BASE.to_string();
    };

    // A literal IPv6 target has to be bracketed to be a valid URL authority.
    let host = if endpoint.target.contains(':')
        && !endpoint.target.starts_with('[')
    {
        format!("[{}]", endpoint.target)
    } else {
        endpoint.target.clone()
    };

    if endpoint.port == 443 {
        format!("https://{host}/")
    } else {
        format!("https://{host}:{}/", endpoint.port)
    }
}

/// Validates and normalizes a base URL from the environment.
///
/// Returns `None` for anything that is not a usable base: an unparsable URL, a
/// scheme other than `https`, or `http` on a non-loopback host. Loopback is
/// allowed because the stub server used for manual testing speaks plain HTTP.
/// The result always ends with `/` and never carries a query or a fragment.
pub fn normalize_base(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    };
    let parsed = Url::parse(&candidate).ok()?;

    let host = parsed.host()?;
    let host_text = match &host {
        url::Host::Ipv6(address) => address.to_string(),
        other => other.to_string(),
    };
    let is_loopback =
        matches!(host_text.as_str(), "127.0.0.1" | "localhost" | "::1");
    let scheme = parsed.scheme();
    if scheme != "https" && !(scheme == "http" && is_loopback) {
        return None;
    }

    let mut base = String::new();
    base.push_str(scheme);
    base.push_str("://");
    match host {
        url::Host::Ipv6(address) => base.push_str(&format!("[{address}]")),
        other => base.push_str(&other.to_string()),
    }
    if let Some(port) = parsed.port() {
        base.push_str(&format!(":{port}"));
    }
    let path = parsed.path();
    base.push_str(path);
    if !path.ends_with('/') {
        base.push('/');
    }

    Some(base)
}

/// The base URL to use, in priority order: override, SRV answer, fallback.
///
/// An unusable override is logged and ignored; it must never take the service
/// offline on its own, because the override is a developer convenience and not
/// part of the runtime configuration.
pub fn resolve_base(
    override_base: Option<&str>,
    endpoint: Option<&ServiceEndpoint>,
) -> String {
    if let Some(raw) = override_base {
        match normalize_base(raw) {
            Some(base) => return base,
            None => {
                tracing::warn!(
                    target: LOG_TARGET,
                    "Ignoring {UPDATE_SERVICE_BASE_ENV}: it is not a usable https base URL"
                );
            }
        }
    }

    base_url_for(endpoint)
}

// ── Latency and state bookkeeping ───────────────────────────────

/// Median of a set of samples.
///
/// With the five-sample window the result is the exact middle value. An even
/// count (only reachable from tests) takes the upper middle, which is
/// deliberately conservative for a latency figure.
fn median(samples: &[u64]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

/// Turns probe outcomes into a published state plus a latency figure.
///
/// The tracker is deliberately independent of the network so the transition
/// rules can be tested without a server: [`Self::record_success`] and
/// [`Self::record_failure`] are the only entry points.
#[derive(Debug, Default)]
pub struct HealthTracker {
    state: UpdateServiceState,
    consecutive_failures: u8,
    samples: VecDeque<u64>,
    last_failure: Option<ProbeFailure>,
}

impl HealthTracker {
    /// A tracker that has not seen a probe yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The state to display.
    pub fn state(&self) -> UpdateServiceState {
        self.state
    }

    /// Failures recorded since the last success.
    pub fn consecutive_failures(&self) -> u8 {
        self.consecutive_failures
    }

    /// Median of the retained round-trip times; `None` before the first one.
    pub fn rtt_ms(&self) -> Option<u64> {
        median(&self.samples.iter().copied().collect::<Vec<_>>())
    }

    /// Category of the last failure; cleared by the next success.
    pub fn last_failure(&self) -> Option<ProbeFailure> {
        self.last_failure
    }

    /// Records a successful probe and returns whether the state changed.
    ///
    /// A single success is enough to come back online: waiting for a
    /// confirmation would keep the indicator red for another sampling interval
    /// after the link is demonstrably working.
    pub fn record_success(&mut self, rtt_ms: u64) -> bool {
        self.consecutive_failures = 0;
        self.last_failure = None;
        if self.samples.len() == RTT_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(rtt_ms);

        let previous = self.state;
        self.state = UpdateServiceState::Online;
        previous != self.state
    }

    /// Records a failed probe of the given category.
    pub fn record_failure_with(&mut self, kind: ProbeFailure) -> bool {
        self.last_failure = Some(kind);
        self.record_failure()
    }

    /// Records a failed probe and returns whether the state changed.
    ///
    /// The first failure keeps the current state so a single stalled packet
    /// does not flip the indicator; the second one in a row goes offline.
    pub fn record_failure(&mut self) -> bool {
        self.consecutive_failures =
            self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures < FAILURES_BEFORE_OFFLINE {
            return false;
        }

        let previous = self.state;
        self.state = UpdateServiceState::Offline;
        previous != self.state
    }
}

/// Whether a snapshot is worth pushing to subscribers.
///
/// State and base URL changes always are; a latency change only when it is
/// visible, which is what keeps the event rate at the sampling rate instead of
/// at the request rate.
fn status_is_publishable(
    previous: &UpdateServiceStatus,
    next: &UpdateServiceStatus,
) -> bool {
    if previous.state != next.state
        || previous.base_url != next.base_url
        || previous.last_failure != next.last_failure
        || previous.tier != next.tier
        || previous.insecure != next.insecure
    {
        return true;
    }

    match (previous.rtt_ms, next.rtt_ms) {
        (Some(previous), Some(next)) => {
            previous.abs_diff(next) >= RTT_PUBLISH_THRESHOLD_MS
        }
        (None, None) => false,
        _ => true,
    }
}

// ── DNS cache ───────────────────────────────────────────────────

/// Cached outcome of the last SRV lookup.
#[derive(Debug, Default)]
struct SrvCache {
    endpoint: Option<ServiceEndpoint>,
    resolved_at: Option<Instant>,
    lookup_failed: bool,
}

impl SrvCache {
    /// Whether the cached value may still be used without asking DNS.
    fn is_fresh(&self) -> bool {
        let Some(resolved_at) = self.resolved_at else {
            return false;
        };
        let ttl = if self.lookup_failed {
            SRV_FAILURE_TTL
        } else {
            SRV_SUCCESS_TTL
        };
        resolved_at.elapsed() < ttl
    }

    /// Stores a lookup outcome; `endpoint` is `None` when nothing usable was
    /// returned, and `lookup_failed` distinguishes "no record" from "the
    /// resolver itself failed".
    fn store(&mut self, endpoint: Option<ServiceEndpoint>, lookup_failed: bool) {
        self.endpoint = endpoint;
        self.resolved_at = Some(Instant::now());
        self.lookup_failed = lookup_failed;
    }
}

// ── The service ─────────────────────────────────────────────────

/// Mutable state guarded by one mutex; never held across an `await`.
#[derive(Debug, Default)]
struct Inner {
    cache: SrvCache,
    tracker: HealthTracker,
    base_url: Option<String>,
    last_ok_at: Option<u64>,
    published: UpdateServiceStatus,
    /// When the last "certificate could not be verified" warning was logged.
    certificate_warning_at: Option<Instant>,
    /// When the last plaintext-fallback warning was logged.
    plaintext_warning_at: Option<Instant>,
    /// Tier in use.
    tier: UpdateServiceTier,
    /// True when the active base is plaintext.
    insecure: bool,
    /// Failed probes on the tier in use.
    tier_failures: u8,
    /// Cached IPv6 address and when it was resolved.
    ipv6: Option<std::net::Ipv6Addr>,
    ipv6_at: Option<Instant>,
    /// When the best tier was last re-tested while a lower tier was active.
    higher_tier_probe_at: Option<Instant>,
}

impl Inner {
    /// Snapshots the state in the shape the frontend consumes.
    fn snapshot(&self) -> UpdateServiceStatus {
        UpdateServiceStatus {
            state: self.tracker.state(),
            rtt_ms: self.tracker.rtt_ms(),
            base_url: self.base_url.clone(),
            last_ok_at: self.last_ok_at,
            last_failure: self.tracker.last_failure(),
            tier: self.tier,
            insecure: self.insecure,
        }
    }
}

/// Process-wide update service: address resolution, probing and sampling.
pub struct UpdateService {
    inner: Mutex<Inner>,
    statuses: broadcast::Sender<UpdateServiceStatus>,
    client: OnceLock<Option<reqwest::Client>>,
    api_client: OnceLock<Option<reqwest::Client>>,
    paused: AtomicBool,
    probing: AtomicBool,
}

impl std::fmt::Debug for UpdateService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateService")
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for UpdateService {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateService {
    /// Creates a service that has not resolved or probed anything yet.
    pub fn new() -> Self {
        let (statuses, _) = broadcast::channel(STATUS_CHANNEL_CAPACITY);
        Self {
            inner: Mutex::new(Inner::default()),
            statuses,
            client: OnceLock::new(),
            api_client: OnceLock::new(),
            paused: AtomicBool::new(false),
            probing: AtomicBool::new(false),
        }
    }

    /// The process-wide instance, created on first use.
    pub fn shared() -> &'static Arc<Self> {
        static SHARED: OnceLock<Arc<UpdateService>> = OnceLock::new();
        SHARED.get_or_init(|| Arc::new(UpdateService::new()))
    }

    /// Subscribes to status changes. Every subscriber gets its own cursor, so
    /// the Tauri shell and an in-process consumer do not steal from each other.
    pub fn subscribe(&self) -> broadcast::Receiver<UpdateServiceStatus> {
        self.statuses.subscribe()
    }

    /// The current snapshot, without any network access.
    pub fn status(&self) -> UpdateServiceStatus {
        self.inner.lock().snapshot()
    }

    /// The base URL from the last resolution, without any network access.
    pub fn base_url(&self) -> Option<String> {
        self.inner.lock().base_url.clone()
    }

    /// Suspends or resumes sampling.
    ///
    /// Used while the window is minimised: a hidden window has no indicator to
    /// update, so spending probes on it would be pure waste.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
        if !paused {
            tracing::debug!(target: LOG_TARGET, "Sampling resumed");
        }
    }

    /// Whether sampling is currently suspended.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// The base URL to use right now, without probing.
    ///
    /// Keeps the old contract (`updater_impl` and `get_json` call it): the value
    /// is the highest-priority candidate that has been resolved so far, and it
    /// never triggers a health request of its own.
    pub async fn resolve_base_url(&self) -> Option<String> {
        // 开源版没有自建更新服务（`brand::UPDATE_SERVICE_SRV` 是空串）。
        // 此时再走 SRV → AAAA → 隧道三级探测纯属空转：空 SRV 要白等 1.5s 超时，
        // 之后还会对 `placeholder.invalid` 发健康请求，每轮往日志里灌两行
        // warn（实测两小时涨 4MB），而结果永远是兜底常量。直接短路。
        if crate::brand::UPDATE_SERVICE_SRV.trim().is_empty() {
            return Some(UPDATE_SERVICE_FALLBACK_BASE.to_string());
        }

        let candidate = self.ensure_candidate().await?;
        Some(candidate.base)
    }

    /// The candidate in use, resolving the tier list if nothing is active yet.
    async fn ensure_candidate(&self) -> Option<TierCandidate> {
        if let Some(locked) = self.locked_candidate() {
            self.set_active(&locked);
            return Some(locked);
        }

        // An already-probed tier is reused until a probe says otherwise.
        let active = {
            let inner = self.inner.lock();
            match (&inner.base_url, inner.tier) {
                (Some(base), tier) if tier != UpdateServiceTier::None => {
                    Some(TierCandidate {
                        tier,
                        base: base.clone(),
                        insecure: inner.insecure,
                    })
                }
                _ => None,
            }
        };
        if let Some(active) = active {
            return Some(active);
        }

        let candidates = self.candidates().await;
        let first = candidates.into_iter().next();
        if let Some(candidate) = &first {
            self.set_active(candidate);
        }
        first
    }

    /// The env override, when it is usable.
    ///
    /// A locked address never participates in failover: a developer pointing the
    /// launcher at a stub server expects that server and nothing else.
    fn locked_candidate(&self) -> Option<TierCandidate> {
        let raw = std::env::var(UPDATE_SERVICE_BASE_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        build_candidates(Some(raw.as_str()), None, None)
            .into_iter()
            .next()
    }

    /// The ordered candidate list for this probe.
    ///
    /// Order is the user-facing priority: SRV, then IPv6, then the Cloudflare
    /// tunnel. Each step only appears when it resolves, except Cloudflare which
    /// is always present as the last resort.
    async fn candidates(&self) -> Vec<TierCandidate> {
        if let Some(locked) = self.locked_candidate() {
            return vec![locked];
        }

        let srv = self.srv_endpoint().await;
        let ipv6 = self.ipv6_address().await;
        build_candidates(None, srv.as_ref(), ipv6)
    }

    /// Cached SRV endpoint, refreshed on a short TTL.
    ///
    /// The record points at a STUN-derived port that changes whenever the
    /// tunnel is re-established, so the success TTL is deliberately half the
    /// allowed maximum: a stale port costs two failed probes before failover.
    async fn srv_endpoint(&self) -> Option<ServiceEndpoint> {
        let cached = {
            let inner = self.inner.lock();
            if inner.cache.is_fresh() {
                Some(inner.cache.endpoint.clone())
            } else {
                None
            }
        };
        match cached {
            Some(endpoint) => endpoint,
            None => self.lookup_srv().await,
        }
    }

    /// Cached IPv6 address of the service host, refreshed every minute.
    ///
    /// A missing `AAAA` is the normal state today, so failures are cached too
    /// (a minute) and never logged above `debug`.
    async fn ipv6_address(&self) -> Option<std::net::Ipv6Addr> {
        {
            let inner = self.inner.lock();
            if let Some(at) = inner.ipv6_at
                && at.elapsed() < IPV6_SUCCESS_TTL
            {
                return inner.ipv6;
            }
        }

        let resolved = {
            let lookup = async {
                let Ok(builder) = hickory_resolver::TokioResolver::builder_tokio()
                else {
                    return None;
                };
                let resolver = builder.build();
                match resolver.lookup_ip(UPDATE_SERVICE_HOST).await {
                    Ok(answer) => answer.iter().find_map(|address| match address {
                        std::net::IpAddr::V6(address) => Some(address),
                        std::net::IpAddr::V4(_) => None,
                    }),
                    Err(error) => {
                        tracing::debug!(
                            target: LOG_TARGET,
                            "AAAA lookup for {UPDATE_SERVICE_HOST} failed: {error}"
                        );
                        None
                    }
                }
            };
            match tokio::time::timeout(SRV_LOOKUP_TIMEOUT, lookup).await {
                Ok(address) => address,
                Err(_) => {
                    tracing::debug!(
                        target: LOG_TARGET,
                        "AAAA lookup for {UPDATE_SERVICE_HOST} timed out"
                    );
                    None
                }
            }
        };

        {
            let mut inner = self.inner.lock();
            inner.ipv6 = resolved;
            inner.ipv6_at = Some(Instant::now());
        }
        resolved
    }

    /// Stores the tier in use and publishes the change.
    fn set_active(&self, candidate: &TierCandidate) {
        let changed = {
            let mut inner = self.inner.lock();
            let changed = inner.tier != candidate.tier
                || inner.base_url.as_deref() != Some(candidate.base.as_str())
                || inner.insecure != candidate.insecure;
            inner.tier = candidate.tier;
            inner.base_url = Some(candidate.base.clone());
            inner.insecure = candidate.insecure;
            changed
        };
        if changed {
            self.publish_snapshot();
        }
    }

    /// Asks DNS for the SRV record and caches the outcome.
    ///
    /// Three failures are distinguished: a record set (used), an empty answer or
    /// `NXDOMAIN` (no record — a normal state, given the tunnel port moves), and
    /// a resolver failure (retried sooner).
    async fn lookup_srv(&self) -> Option<ServiceEndpoint> {
        let lookup = async {
            let Ok(builder) = hickory_resolver::TokioResolver::builder_tokio()
            else {
                tracing::warn!(
                    target: LOG_TARGET,
                    "System resolver unavailable; using the next tier"
                );
                return None;
            };
            let resolver = builder.build();

            match resolver.srv_lookup(UPDATE_SERVICE_SRV).await {
                Ok(answer) => Some(
                    answer
                        .iter()
                        .map(|record| SrvAnswer {
                            priority: record.priority(),
                            weight: record.weight(),
                            target: record.target().to_string(),
                            port: record.port(),
                        })
                        .collect::<Vec<_>>(),
                ),
                Err(error)
                    if error
                        .proto()
                        .as_ref()
                        .is_some_and(|proto| proto.kind().is_no_records_found()) =>
                {
                    // An empty answer is a normal outcome: the record set may
                    // legitimately be absent while the tunnel is down.
                    Some(Vec::new())
                }
                Err(error) => {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "SRV lookup for {UPDATE_SERVICE_SRV} failed: {error}"
                    );
                    None
                }
            }
        };

        let answers = match tokio::time::timeout(SRV_LOOKUP_TIMEOUT, lookup)
            .await
        {
            Ok(answers) => answers,
            Err(_) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    "SRV lookup for {UPDATE_SERVICE_SRV} timed out"
                );
                None
            }
        };

        let lookup_failed = answers.is_none();
        let endpoint = answers.as_deref().and_then(select_endpoint);

        {
            let mut inner = self.inner.lock();
            inner.cache.store(endpoint.clone(), lookup_failed);
        }

        if let Some(endpoint) = &endpoint {
            tracing::debug!(
                target: LOG_TARGET,
                "Update service SRV resolved to {}:{}",
                endpoint.target,
                endpoint.port
            );
        }

        endpoint
    }

    /// Runs one probe round and returns the resulting snapshot.
    ///
    /// One round is one `/health` request against one tier. Failover is a
    /// property of rounds, not of attempts: two consecutive failing rounds move
    /// the service one tier down, and the highest tier is re-tested every
    /// [`SRV_REFRESH_INTERVAL`] so a recovered tunnel is preferred again.
    pub async fn probe_once(&self) -> UpdateServiceStatus {
        // A locked address has no tiers to fail over to.
        if let Some(locked) = self.locked_candidate() {
            self.set_active(&locked);
            return self.probe_candidate(&locked).await;
        }

        let candidates = self.candidates().await;
        let Some(highest) = candidates.first().cloned() else {
            return self.status();
        };

        let active = {
            let inner = self.inner.lock();
            (inner.tier, inner.tier_failures)
        };

        // Recovery: while a lower tier is in use, retry the best one every few
        // minutes instead of waiting for the user to restart.
        if active.0 != highest.tier && self.higher_tier_probe_due() {
            self.mark_higher_tier_probe();
            match self.probe_base(&highest.base).await {
                Ok(outcome) => {
                    tracing::info!(
                        target: LOG_TARGET,
                        tier = highest.tier.as_str(),
                        reason = "preferred-tier-recovered",
                        "Switching update service tier"
                    );
                    self.record_success_on(&highest, &outcome);
                    return self.status();
                }
                Err(kind) => {
                    tracing::debug!(
                        target: LOG_TARGET,
                        tier = highest.tier.as_str(),
                        "Preferred tier still unavailable: {kind:?}"
                    );
                }
            }
        }

        let index = candidates
            .iter()
            .position(|candidate| candidate.tier == active.0)
            .unwrap_or(0);
        let index = advance_tier(index, active.1, candidates.len());
        let candidate = candidates[index].clone();

        if candidate.tier != active.0 {
            tracing::info!(
                target: LOG_TARGET,
                tier = candidate.tier.as_str(),
                reason = "consecutive-failures",
                "Switching update service tier"
            );
        }

        self.probe_candidate(&candidate).await
    }

    /// Probes one candidate and folds the result into the shared state.
    async fn probe_candidate(&self, candidate: &TierCandidate) -> UpdateServiceStatus {
        match self.probe_base(&candidate.base).await {
            Ok(outcome) => {
                self.record_success_on(candidate, &outcome);
            }
            Err(kind) => {
                self.record_tier_failure(kind);
            }
        }
        self.status()
    }

    /// Whether the recovery probe of the best tier is due.
    fn higher_tier_probe_due(&self) -> bool {
        let inner = self.inner.lock();
        match inner.higher_tier_probe_at {
            Some(at) => at.elapsed() >= SRV_REFRESH_INTERVAL,
            None => true,
        }
    }

    /// Starts the recovery-probe window again.
    fn mark_higher_tier_probe(&self) {
        self.inner.lock().higher_tier_probe_at = Some(Instant::now());
    }

    /// Probes a base, downgrading to plaintext only for a TLS *protocol* refusal.
    ///
    /// A certificate problem is never retried over `http`: that would turn a
    /// mis-issued chain into a silent plaintext update feed. A protocol mismatch
    /// (an old proxy that only speaks TLS 1.0, or a listener that wants plain
    /// HTTP on the same port) is a deployment issue, so it is retried once and
    /// reported as `insecure`.
    async fn probe_base(
        &self,
        base: &str,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        match self.probe_base_once(base).await {
            Ok(outcome) => Ok(outcome),
            Err(BaseProbeError::TlsProtocol) => {
                let Some(plaintext) = insecure_variant(base) else {
                    return Err(ProbeFailure::Unreachable);
                };
                self.warn_about_plaintext_fallback(base, &plaintext);
                self.probe_base_once(&plaintext)
                    .await
                    .map_err(BaseProbeError::as_failure)
            }
            Err(error) => Err(error.as_failure()),
        }
    }

    /// One `/health` request chain: follow redirects, refuse Access gates.
    async fn probe_base_once(
        &self,
        base: &str,
    ) -> Result<ProbeOutcome, BaseProbeError> {
        let Some(client) = self.client() else {
            return Err(BaseProbeError::Unreachable);
        };

        let mut current = base.to_string();
        let mut insecure = base.starts_with("http://");
        let started = Instant::now();
        let mut hops = 0usize;

        loop {
            let url = format!("{current}{UPDATE_SERVICE_HEALTH_PATH}");
            let response = match tokio::time::timeout(
                PROBE_TOTAL_TIMEOUT,
                client.get(&url).send(),
            )
            .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    if is_certificate_failure(&error) {
                        self.warn_about_untrusted_certificate(&url);
                        return Err(BaseProbeError::Certificate);
                    }
                    if is_tls_protocol_failure(&error.to_string()) {
                        return Err(BaseProbeError::TlsProtocol);
                    }
                    tracing::debug!(
                        target: LOG_TARGET,
                        "Health request to {url} failed: {error}"
                    );
                    return Err(BaseProbeError::Unreachable);
                }
                Err(_) => {
                    tracing::debug!(
                        target: LOG_TARGET,
                        "Health request to {url} timed out"
                    );
                    return Err(BaseProbeError::Timeout);
                }
            };

            let rtt_ms = started.elapsed().as_millis() as u64;
            let status = response.status();

            if status.is_redirection() {
                hops += 1;
                if hops > MAX_REDIRECT_HOPS {
                    tracing::debug!(
                        target: LOG_TARGET,
                        "Health request to {url} exceeded {MAX_REDIRECT_HOPS} redirects"
                    );
                    return Err(BaseProbeError::Unreachable);
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                if is_access_gate(location.as_deref(), "") {
                    tracing::info!(
                        target: LOG_TARGET,
                        "Health request to {url} was redirected to a Cloudflare Access login; treating the tier as unavailable"
                    );
                    return Err(BaseProbeError::Unreachable);
                }
                let Some(location) = location else {
                    return Err(BaseProbeError::Unreachable);
                };
                let Some(next) = redirect_base(&current, &location) else {
                    return Err(BaseProbeError::Unreachable);
                };
                // Adopt the redirect target: the next probe and every proxied
                // request should use the host the service actually answered from.
                insecure = next.starts_with("http://");
                current = next;
                continue;
            }

            if !status.is_success() {
                tracing::debug!(
                    target: LOG_TARGET,
                    "Health endpoint at {url} answered {status}"
                );
                return Err(BaseProbeError::Unreachable);
            }

            // `text()` consumes the response, so take the peer address first.
            let peer = response.remote_addr();
            let body = match response.text().await {
                Ok(body) => body,
                Err(error) => {
                    tracing::debug!(
                        target: LOG_TARGET,
                        "Health endpoint at {url} returned an unreadable body: {error}"
                    );
                    return Err(BaseProbeError::Unreachable);
                }
            };

            // A tunnel that answers 200 with a login page is not healthy.
            if is_access_gate(None, &body) {
                tracing::info!(
                    target: LOG_TARGET,
                    "Health endpoint at {url} returned a Cloudflare Access page; treating the tier as unavailable"
                );
                return Err(BaseProbeError::Unreachable);
            }

            let ok = serde_json::from_str::<HealthResponse>(&body)
                .map(|body| body.ok)
                .unwrap_or(false);
            if !ok {
                tracing::debug!(
                    target: LOG_TARGET,
                    "Health endpoint at {url} did not report ok"
                );
                return Err(BaseProbeError::Unreachable);
            }

            if let Some(peer) = peer {
                tracing::debug!(
                    target: LOG_TARGET,
                    "Health probe connected to {peer}"
                );
            }

            return Ok(ProbeOutcome {
                base: current,
                final_url: url,
                rtt_ms,
                insecure,
            });
        }
    }

    /// Records a successful probe and publishes the result.
    fn record_success(&self, rtt_ms: u64) {
        // `timestamp()` is signed and can be negative for a clock before 1970;
        // the wire format is an unsigned unix timestamp, so clamp instead of
        // wrapping.
        let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
        {
            let mut inner = self.inner.lock();
            inner.tracker.record_success(rtt_ms);
            inner.last_ok_at = Some(now);
        }
        self.publish_snapshot();
    }

    /// Records a failed probe and publishes the result.
    fn record_failure(&self, kind: ProbeFailure) {
        self.record_tier_failure(kind);
    }

    /// Records a successful probe on a specific candidate.
    ///
    /// Adopts the base the probe actually ended on: after a redirect chain that
    /// is the host which answered, not the one that was dialled.
    fn record_success_on(&self, candidate: &TierCandidate, outcome: &ProbeOutcome) {
        let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
        {
            let mut inner = self.inner.lock();
            inner.tracker.record_success(outcome.rtt_ms);
            inner.last_ok_at = Some(now);
            inner.tier = candidate.tier;
            inner.base_url = Some(outcome.base.clone());
            inner.insecure = outcome.insecure;
            inner.tier_failures = 0;
            inner.higher_tier_probe_at = None;
        }
        tracing::debug!(
            target: LOG_TARGET,
            tier = candidate.tier.as_str(),
            base = outcome.base.as_str(),
            final = outcome.final_url.as_str(),
            rtt_ms = outcome.rtt_ms,
            insecure = outcome.insecure,
            "Update service probe succeeded"
        );
        self.publish_snapshot();
    }

    /// Counts a failure against the active tier and publishes the result.
    fn record_tier_failure(&self, kind: ProbeFailure) {
        {
            let mut inner = self.inner.lock();
            inner.tracker.record_failure_with(kind);
            inner.tier_failures = inner.tier_failures.saturating_add(1);
        }
        self.publish_snapshot();
    }

    /// Warns once per interval that a tier only answered over plaintext.
    fn warn_about_plaintext_fallback(&self, base: &str, fallback: &str) {
        let should_warn = {
            let mut inner = self.inner.lock();
            match inner.plaintext_warning_at {
                Some(at) if at.elapsed() < CERT_WARNING_INTERVAL => false,
                _ => {
                    inner.plaintext_warning_at = Some(Instant::now());
                    true
                }
            }
        };
        if should_warn {
            tracing::warn!(
                target: LOG_TARGET,
                "TLS was refused by {base}; retrying the same tier over plaintext at {fallback}. The update feed is unauthenticated on this path"
            );
        }
    }

    /// Answers "is the service reachable right now" within `budget`.
    ///
    /// This is the fast path the updater uses before it hands over to the
    /// updater plugin, whose own requests have no timeout: without a bounded
    /// preflight, an offline check hangs for the plugin's full download budget.
    /// The outcome is folded into the shared tracker so the indicator reflects
    /// what the user just did.
    pub async fn is_reachable(&self, budget: Duration) -> bool {
        let work = async {
            let candidate = self
                .ensure_candidate()
                .await
                .ok_or(ProbeFailure::Unreachable)?;
            self.probe_base(&candidate.base).await
        };

        match tokio::time::timeout(budget, work).await {
            Ok(Ok(outcome)) => {
                self.record_success(outcome.rtt_ms);
                true
            }
            Ok(Err(kind)) => {
                self.record_failure(kind);
                false
            }
            Err(_) => {
                tracing::debug!(
                    target: LOG_TARGET,
                    "Health preflight exceeded {budget:?}"
                );
                self.record_failure(ProbeFailure::Timeout);
                false
            }
        }
    }

    /// Reports an untrusted certificate at most once per interval.
    ///
    /// Validation stays strict: the point of this warning is to make a
    /// mis-issued or self-signed deployment visible in the log, not to fall back
    /// to an unverified connection.
    fn warn_about_untrusted_certificate(&self, url: &str) {
        let should_warn = {
            let mut inner = self.inner.lock();
            match inner.certificate_warning_at {
                Some(at) if at.elapsed() < CERT_WARNING_INTERVAL => false,
                _ => {
                    inner.certificate_warning_at = Some(Instant::now());
                    true
                }
            }
        };

        if should_warn {
            tracing::warn!(
                target: LOG_TARGET,
                "The certificate of {url} could not be verified. Validation is strict by design, so the update service stays offline until the chain is trusted by the system store"
            );
        }
    }

    /// Fetches one JSON document from the update service.
    ///
    /// The frontend cannot call these endpoints itself: the webview's `http:`
    /// capability is a compile-time host list, and the service address is only
    /// known after the SRV lookup. Routing the business calls through here also
    /// means they use the same resolved host, port and TLS policy as the health
    /// probe, instead of a compiled-in constant.
    ///
    /// Every failure — offline, non-2xx, unparsable body — returns `None`, which
    /// the caller renders as its empty state. Nothing is thrown at the user.
    pub async fn get_json(&self, path: &str) -> Option<serde_json::Value> {
        // While the sampler says the service is down the answer is the same
        // either way, so do not make the user wait for a timeout to see it.
        if self.inner.lock().tracker.state() == UpdateServiceState::Offline {
            tracing::debug!(
                target: LOG_TARGET,
                "Skipping /{path}: the service is offline"
            );
            return None;
        }

        let base = self.resolve_base_url().await?;
        let client = self.api_client()?;
        let url = format!("{base}{path}");
        let started = Instant::now();

        let response = match tokio::time::timeout(
            API_TOTAL_TIMEOUT,
            client.get(&url).send(),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) if is_certificate_failure(&error) => {
                self.warn_about_untrusted_certificate(&url);
                return None;
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    target: LOG_TARGET,
                    "GET {url} failed: {error}"
                );
                return None;
            }
            Err(_) => {
                tracing::debug!(target: LOG_TARGET, "GET {url} timed out");
                return None;
            }
        };

        if !response.status().is_success() {
            tracing::debug!(
                target: LOG_TARGET,
                "GET {url} answered {}",
                response.status()
            );
            return None;
        }

        let rtt_ms = started.elapsed().as_millis() as u64;
        match response.json::<serde_json::Value>().await {
            Ok(value) => {
                // A served business request is proof the service is up, so let
                // it clear an offline indicator the health probe may still be
                // one interval behind on.
                self.record_success(rtt_ms);
                Some(value)
            }
            Err(error) => {
                tracing::debug!(
                    target: LOG_TARGET,
                    "GET {url} returned an unreadable body: {error}"
                );
                None
            }
        }
    }

    /// The HTTP client used for business requests (longer budget than a probe).
    fn api_client(&self) -> Option<&reqwest::Client> {
        self.api_client
            .get_or_init(|| {
                match reqwest::Client::builder()
                    .user_agent(crate::launcher_user_agent())
                    .connect_timeout(API_CONNECT_TIMEOUT)
                    .timeout(API_TOTAL_TIMEOUT)
                    .build()
                {
                    Ok(client) => Some(client),
                    Err(error) => {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Could not build the update service client: {error}"
                        );
                        None
                    }
                }
            })
            .as_ref()
    }

    /// The shared HTTP client, built once with the probe timeouts.
    ///
    /// Certificate validation is deliberately left at the rustls/webpki default:
    /// `danger_accept_invalid_certs` and `danger_accept_invalid_hostnames` must
    /// never be enabled here, because an update feed is exactly the kind of
    /// endpoint where a downgrade turns into remote code execution.
    fn client(&self) -> Option<&reqwest::Client> {
        self.client
            .get_or_init(|| {
                match reqwest::Client::builder()
                    .user_agent(crate::launcher_user_agent())
                    .connect_timeout(PROBE_CONNECT_TIMEOUT)
                    .timeout(PROBE_TOTAL_TIMEOUT)
                    // Redirects are followed by `probe_base_once`, which has to
                    // count hops and spot Access gates; letting reqwest follow
                    // them would hide both.
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                {
                    Ok(client) => Some(client),
                    Err(error) => {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Could not build the health-check client: {error}"
                        );
                        None
                    }
                }
            })
            .as_ref()
    }

    /// Publishes the current snapshot when it is worth an event.
    fn publish_snapshot(&self) {
        let next = {
            let mut inner = self.inner.lock();
            let next = inner.snapshot();
            if !status_is_publishable(&inner.published, &next) {
                return;
            }
            inner.published = next.clone();
            next
        };

        if let Err(error) = self.statuses.send(next) {
            tracing::trace!(
                target: LOG_TARGET,
                "No update service status subscriber: {error}"
            );
        }
    }

    /// Runs one probe in the background unless one is already running.
    ///
    /// The "refresh" command returns the cached snapshot immediately and lets
    /// this do the network work, so a user pressing retry twice cannot stack
    /// probes on top of each other.
    pub fn spawn_probe(self: &Arc<Self>) {
        if self
            .probing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            tracing::trace!(target: LOG_TARGET, "A probe is already running");
            return;
        }

        let service = Arc::clone(self);
        tokio::spawn(async move {
            service.probe_once().await;
            service.probing.store(false, Ordering::Release);
        });
    }

}

/// Body of a `/health` response. Only `ok` is interpreted; the timestamp and
/// the minimum launcher version are informational.
#[derive(Debug, Deserialize)]
struct HealthResponse {
    #[serde(default)]
    ok: bool,
}

/// Keywords that identify certificate validation trouble in an error chain.
///
/// `reqwest` does not expose a typed TLS-error kind, so the message text of the
/// whole source chain is the only portable signal. The tokens are specific
/// enough that a DNS or connection failure cannot match them.
const CERTIFICATE_ERROR_HINTS: [&str; 7] = [
    "certificate",
    "unknownissuer",
    "unknown issuer",
    "notvalidforhostname",
    "invalid peer certificate",
    "self-signed",
    "self signed",
];

/// Whether an error message looks like a certificate validation failure.
fn looks_like_certificate_failure(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    CERTIFICATE_ERROR_HINTS
        .iter()
        .any(|hint| lowered.contains(hint))
}

/// Whether a request error was caused by the TLS certificate not validating.
fn is_certificate_failure(error: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if looks_like_certificate_failure(&current.to_string()) {
            return true;
        }
        source = current.source();
    }
    false
}

/// Adds ±20 % of jitter to a delay.
///
/// Several launchers on one network must not retry in lockstep after a shared
/// outage, which is what a fixed delay would make them do.
fn jitter(delay: Duration) -> Duration {
    use rand::Rng as _;

    let millis = delay.as_millis() as u64;
    if millis == 0 {
        return delay;
    }
    let spread = millis / 5;
    let offset = rand::thread_rng().gen_range(0..=spread * 2);
    Duration::from_millis(millis - spread + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one SRV answer for the selection tests.
    fn answer(priority: u16, weight: u16, target: &str, port: u16) -> SrvAnswer {
        SrvAnswer {
            priority,
            weight,
            target: target.to_string(),
            port,
        }
    }

    #[test]
    fn srv_record_builds_the_base_url_with_its_port() {
        // The measured production record: host on the default proxy port.
        let answers = [answer(10, 0, "update.example.com.", 5443)];
        let endpoint = select_endpoint(&answers);

        assert_eq!(
            endpoint,
            Some(ServiceEndpoint {
                target: "update.example.com".to_string(),
                port: 5443,
            })
        );
        assert_eq!(
            resolve_base(None, endpoint.as_ref()),
            "https://update.example.com:5443/"
        );
    }

    #[test]
    fn srv_answer_without_records_falls_back_to_the_constant_base() {
        // NXDOMAIN surfaces as an empty answer set.
        let endpoint = select_endpoint(&[]);

        assert_eq!(endpoint, None);
        assert_eq!(
            resolve_base(None, endpoint.as_ref()),
            UPDATE_SERVICE_FALLBACK_BASE
        );
    }

    #[test]
    fn srv_target_dot_marks_the_service_unavailable() {
        // RFC 2782: a single dot means "the service is decidedly not available".
        let answers = [answer(0, 0, ".", 5443)];

        assert_eq!(select_endpoint(&answers), None);
        assert_eq!(
            resolve_base(None, select_endpoint(&answers).as_ref()),
            UPDATE_SERVICE_FALLBACK_BASE
        );
    }

    #[test]
    fn srv_port_zero_marks_the_service_unavailable() {
        let answers = [answer(10, 5, "update.example.com.", 0)];

        assert_eq!(select_endpoint(&answers), None);
        assert_eq!(
            resolve_base(None, select_endpoint(&answers).as_ref()),
            UPDATE_SERVICE_FALLBACK_BASE
        );
    }

    #[test]
    fn srv_port_is_kept_unless_it_is_the_https_default() {
        let alternate = [answer(10, 0, "update.example.com.", 8443)];
        assert_eq!(
            resolve_base(None, select_endpoint(&alternate).as_ref()),
            "https://update.example.com:8443/"
        );

        let default = [answer(10, 0, "update.example.com.", 443)];
        assert_eq!(
            resolve_base(None, select_endpoint(&default).as_ref()),
            "https://update.example.com/"
        );
    }

    #[test]
    fn base_override_wins_over_the_srv_answer() {
        let answers = [answer(10, 0, "update.example.com.", 5443)];

        assert_eq!(
            resolve_base(
                Some("http://127.0.0.1:8787"),
                select_endpoint(&answers).as_ref()
            ),
            "http://127.0.0.1:8787/"
        );
        // An unusable override is ignored instead of taking the service down.
        assert_eq!(
            resolve_base(
                Some("ftp://example.invalid"),
                select_endpoint(&answers).as_ref()
            ),
            "https://update.example.com:5443/"
        );
        // ...and a remote http base is refused, so the feed cannot be downgraded.
        assert_eq!(
            resolve_base(
                Some("http://update.example.com"),
                select_endpoint(&answers).as_ref()
            ),
            "https://update.example.com:5443/"
        );
    }

    #[test]
    fn srv_selection_prefers_priority_then_weight() {
        let answers = [
            answer(20, 100, "secondary.example.", 5443),
            answer(10, 0, "quiet.example.", 5443),
            answer(10, 60, "preferred.example.", 5443),
            answer(10, 60, "preferred-later.example.", 5443),
        ];

        // Lowest priority wins, then the highest weight, then arrival order.
        assert_eq!(
            select_endpoint(&answers),
            Some(ServiceEndpoint {
                target: "preferred.example".to_string(),
                port: 5443,
            })
        );
    }

    #[test]
    fn first_success_moves_unknown_to_online() {
        let mut tracker = HealthTracker::new();
        assert_eq!(tracker.state(), UpdateServiceState::Unknown);

        assert!(tracker.record_success(80));
        assert_eq!(tracker.state(), UpdateServiceState::Online);
    }

    #[test]
    fn one_failure_keeps_the_status_online() {
        let mut tracker = HealthTracker::new();
        tracker.record_success(80);

        assert!(!tracker.record_failure());
        assert_eq!(tracker.state(), UpdateServiceState::Online);
        assert_eq!(tracker.consecutive_failures(), 1);
    }

    #[test]
    fn two_failures_move_the_status_offline() {
        let mut tracker = HealthTracker::new();
        tracker.record_success(80);

        assert!(!tracker.record_failure());
        assert!(tracker.record_failure());
        assert_eq!(tracker.state(), UpdateServiceState::Offline);
    }

    #[test]
    fn one_success_moves_offline_back_to_online() {
        let mut tracker = HealthTracker::new();
        tracker.record_failure();
        tracker.record_failure();
        assert_eq!(tracker.state(), UpdateServiceState::Offline);

        assert!(tracker.record_success(120));
        assert_eq!(tracker.state(), UpdateServiceState::Online);
        assert_eq!(tracker.consecutive_failures(), 0);
    }

    #[test]
    fn latency_is_the_median_of_the_last_five_samples() {
        let mut tracker = HealthTracker::new();
        assert_eq!(tracker.rtt_ms(), None);

        // A single spike must not move the displayed figure.
        for sample in [100, 40, 60, 5_000, 50] {
            tracker.record_success(sample);
        }
        assert_eq!(tracker.rtt_ms(), Some(60));

        // The window slides: the spike drops out once five newer samples exist.
        for sample in [70, 72, 68, 71, 69] {
            tracker.record_success(sample);
        }
        assert_eq!(tracker.rtt_ms(), Some(70));
    }

    #[test]
    fn status_is_published_only_when_it_visibly_changed() {
        let previous = UpdateServiceStatus {
            state: UpdateServiceState::Online,
            rtt_ms: Some(100),
            base_url: Some("https://example.invalid/".to_string()),
            last_ok_at: Some(1),
            last_failure: None,
            tier: UpdateServiceTier::Srv,
            insecure: false,
        };

        let jittered = UpdateServiceStatus {
            rtt_ms: Some(101),
            last_ok_at: Some(2),
            ..previous.clone()
        };
        assert!(!status_is_publishable(&previous, &jittered));

        let visible = UpdateServiceStatus {
            rtt_ms: Some(120),
            ..previous.clone()
        };
        assert!(status_is_publishable(&previous, &visible));

        let offline = UpdateServiceStatus {
            state: UpdateServiceState::Offline,
            ..previous.clone()
        };
        assert!(status_is_publishable(&previous, &offline));

        let downgraded = UpdateServiceStatus {
            insecure: true,
            ..previous.clone()
        };
        assert!(status_is_publishable(&previous, &downgraded));

        let switched = UpdateServiceStatus {
            tier: UpdateServiceTier::Cloudflare,
            ..previous.clone()
        };
        assert!(status_is_publishable(&previous, &switched));
    }

    #[test]
    fn jittered_delays_stay_close_to_the_base_delay() {
        for base in PROBE_RETRY_DELAYS {
            let jittered = jitter(base);
            let expected = base.as_millis() as u64;
            let deviation = jittered.as_millis().abs_diff(expected as u128) as u64;

            assert!(
                deviation <= expected / 5 + 1,
                "jitter moved {base:?} to {jittered:?}"
            );
        }
    }

    #[test]
    fn failure_categories_are_kept_for_the_indicator_and_cleared_by_success() {
        let mut tracker = HealthTracker::new();

        tracker.record_failure_with(ProbeFailure::Certificate);
        assert_eq!(tracker.last_failure(), Some(ProbeFailure::Certificate));
        assert_eq!(tracker.state(), UpdateServiceState::Unknown);

        tracker.record_failure_with(ProbeFailure::Timeout);
        assert_eq!(tracker.last_failure(), Some(ProbeFailure::Timeout));
        assert_eq!(tracker.state(), UpdateServiceState::Offline);

        tracker.record_success(90);
        assert_eq!(tracker.last_failure(), None);
        assert_eq!(tracker.state(), UpdateServiceState::Online);
    }

    /// End-to-end probe against `scripts/dev/update-service-stub.mjs`.
    ///
    /// Ignored by default because it needs the stub listening, and reads the
    /// override from the environment so the port can be chosen per run:
    ///
    /// ```text
    /// node scripts/dev/update-service-stub.mjs --port 8787 &
    /// HAYENAI_UPDATE_BASE=http://127.0.0.1:8787/ \
    ///   cargo test -p theseus --release update_service -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "requires scripts/dev/update-service-stub.mjs"]
    async fn reachable_stub_converges_online_with_a_latency() {
        let service = UpdateService::new();

        let status = service.probe_once().await;

        assert_eq!(status.state, UpdateServiceState::Online, "{status:?}");
        assert!(status.rtt_ms.is_some(), "{status:?}");
        assert_eq!(status.last_failure, None, "{status:?}");
        assert!(
            status
                .base_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("http://127.0.0.1:"),
            "{status:?}"
        );
    }

    /// Same harness, pointed at a port nothing is listening on.
    ///
    /// Proves the two-failures rule end to end: the round exhausts its retries
    /// and the status settles on offline instead of hanging or panicking.
    #[tokio::test]
    #[ignore = "requires an unreachable HAYENAI_UPDATE_BASE port"]
    async fn unreachable_base_converges_offline_without_samples() {
        let service = UpdateService::new();

        let status = service.probe_once().await;

        assert_eq!(status.state, UpdateServiceState::Offline, "{status:?}");
        assert_eq!(status.rtt_ms, None, "{status:?}");
        assert!(status.last_failure.is_some(), "{status:?}");
        assert_eq!(status.last_ok_at, None, "{status:?}");
    }

    /// A self-signed certificate must be refused and classified as such.
    ///
    /// Run against `scripts/dev/update-service-stub.mjs --tls-selfsigned` with
    /// `HAYENAI_UPDATE_BASE=https://127.0.0.1:<port>/`. This is the case the
    /// tooltip's "certificate not trusted" line exists for: validation stays
    /// strict, so the service goes offline instead of connecting anyway.
    #[tokio::test]
    #[ignore = "requires the stub in --tls-selfsigned mode"]
    async fn untrusted_certificate_is_classified_as_a_certificate_failure() {
        let service = UpdateService::new();

        let status = service.probe_once().await;

        assert_eq!(status.state, UpdateServiceState::Offline, "{status:?}");
        assert_eq!(
            status.last_failure,
            Some(ProbeFailure::Certificate),
            "{status:?}"
        );
    }

    /// Exercises the business-endpoint proxy against the stub.
    ///
    /// This is the path the announcement feed and the "latest version" hint use;
    /// it must go through the same resolved base as the health probe.
    #[tokio::test]
    #[ignore = "requires scripts/dev/update-service-stub.mjs"]
    async fn stub_serves_announcements_and_manifest_through_the_resolved_base() {
        let service = UpdateService::new();

        let announcements = service
            .get_json("api/announcements")
            .await
            .expect("the stub serves announcements");
        assert_eq!(announcements["ok"], true, "{announcements}");
        assert!(
            announcements["data"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "{announcements}"
        );

        let manifest = service
            .get_json("latest?channel=release")
            .await
            .expect("the stub serves a manifest");
        assert_eq!(manifest["version"], "9.9.9-stub", "{manifest}");
    }

    /// Prints the base the SRV lookup produces, for manual inspection.
    ///
    /// Ignored because it performs a real DNS query: the resolver on some
    /// networks times out for this domain, which is a property of the network
    /// rather than of this code.
    #[tokio::test]
    #[ignore = "performs a real DNS SRV lookup"]
    async fn resolved_base_without_override_comes_from_the_srv_record() {
        let service = UpdateService::new();

        let base = service.resolve_base_url().await.expect("a base URL");

        println!("resolved base = {base}");
        assert!(base.starts_with("https://"), "{base}");
        assert!(base.ends_with('/'), "{base}");
    }

    #[test]
    fn certificate_failures_are_recognised_by_their_message() {
        // Shapes rustls and webpki produce for an untrusted or wrong chain.
        for message in [
            "invalid peer certificate: UnknownIssuer",
            "error sending request: certificate has expired",
            "InvalidCertificate(NotValidForHostname)",
            "self-signed certificate in certificate chain",
        ] {
            assert!(
                looks_like_certificate_failure(message),
                "{message} should be treated as a certificate failure"
            );
        }

        // Ordinary reachability trouble must stay a plain (debug-level) failure,
        // otherwise every offline network would warn once per interval.
        for message in [
            "error sending request: connection refused (os error 10061)",
            "dns error: failed to lookup address information",
            "operation timed out",
            "connection reset by peer",
        ] {
            assert!(
                !looks_like_certificate_failure(message),
                "{message} must not be reported as a certificate failure"
            );
        }
    }

    // ── Tier selection and failover ────────────────────────────

    #[test]
    fn srv_record_wins_and_keeps_its_dynamic_port() {
        // The tunnel port is whatever STUN handed out; 7928 is a real observed
        // value, and pinning it in the assertion is the point: nothing may
        // rewrite it to 5443 or to the HTTPS default.
        let endpoint = ServiceEndpoint {
            target: "update.example.com".to_string(),
            port: 7928,
        };

        let candidates = build_candidates(None, Some(&endpoint), None);

        assert_eq!(candidates[0].tier, UpdateServiceTier::Srv);
        assert_eq!(
            candidates[0].base,
            "https://update.example.com:7928/"
        );
        assert!(!candidates[0].insecure);
        assert_eq!(
            candidates.last().map(|candidate| candidate.tier),
            Some(UpdateServiceTier::Cloudflare),
            "the tunnel must always be the last resort"
        );
    }

    #[test]
    fn missing_srv_falls_through_to_ipv6() {
        let address: std::net::Ipv6Addr = "2001:db8::1".parse().unwrap();

        let candidates = build_candidates(None, None, Some(address));

        assert_eq!(candidates[0].tier, UpdateServiceTier::Ipv6);
        assert_eq!(
            candidates[0].base,
            format!("https://[2001:db8::1]:{UPDATE_SERVICE_IPV6_PORT}/")
        );
    }

    #[test]
    fn no_ipv6_falls_through_to_cloudflare() {
        // Today's reality: the domain publishes no AAAA, so the chain is SRV →
        // Cloudflare, and the last resort must still be present.
        let candidates = build_candidates(None, None, None);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].tier, UpdateServiceTier::Cloudflare);
        assert_eq!(candidates[0].base, UPDATE_SERVICE_CLOUDFLARE_BASE);
    }

    #[test]
    fn an_override_locks_the_candidate_list() {
        let endpoint = ServiceEndpoint {
            target: "update.example.com".to_string(),
            port: 7928,
        };

        let candidates =
            build_candidates(Some("http://127.0.0.1:8787/"), Some(&endpoint), None);

        assert_eq!(candidates.len(), 1, "an override must not fail over");
        assert_eq!(candidates[0].tier, UpdateServiceTier::Override);
        assert_eq!(candidates[0].base, "http://127.0.0.1:8787/");
    }

    #[test]
    fn tier_advances_only_after_two_failures() {
        // One dropped packet must not move the launcher onto a slower path.
        assert_eq!(advance_tier(0, 0, 3), 0);
        assert_eq!(advance_tier(0, 1, 3), 0);
        assert_eq!(advance_tier(0, 2, 3), 1);
        assert_eq!(advance_tier(1, 1, 3), 1);
        assert_eq!(advance_tier(1, 2, 3), 2);
        // Nothing below the last tier, so repeated failures stay put.
        assert_eq!(advance_tier(2, 9, 3), 2);
        assert_eq!(advance_tier(0, 2, 1), 0);
    }

    #[test]
    fn an_access_login_page_is_not_a_healthy_tier() {
        assert!(is_access_gate(
            Some("https://example.com/login"),
            ""
        ));
        assert!(
            is_access_gate(None, "<title>Cloudflare Access</title>"),
            "a 200 that renders the login page is still a gate"
        );
        assert!(!is_access_gate(
            Some("https://update.example.com:7928/api/health"),
            ""
        ));
        assert!(!is_access_gate(None, r#"{"ok":true,"ts":1750000000}"#));
    }

    #[test]
    fn only_protocol_failures_downgrade_to_plaintext() {
        // Protocol mismatch: retried over http on the same tier.
        assert!(is_tls_protocol_failure(
            "error sending request: TLS alert received: protocol version"
        ));
        assert!(is_tls_protocol_failure("handshake failure"));

        // Certificate trouble must never be retried in the clear.
        assert!(!is_tls_protocol_failure(
            "invalid peer certificate: UnknownIssuer"
        ));
        assert!(!is_tls_protocol_failure(
            "self-signed certificate in certificate chain"
        ));
        assert!(!is_tls_protocol_failure("connection refused"));

        assert_eq!(
            insecure_variant("https://update.example.com:7928/").as_deref(),
            Some("http://update.example.com:7928/")
        );
        assert_eq!(insecure_variant("http://127.0.0.1:8787/"), None);
    }

    #[test]
    fn a_redirect_moves_the_base_to_the_new_host() {
        let base = "https://update.example.com:7928/";

        // 307 to another host, pointing straight at the health path.
        assert_eq!(
            redirect_base(base, "https://github.com/bananaikun/HaYenai/releases/api/health")
                .as_deref(),
            Some("https://github.com/bananaikun/HaYenai/releases/")
        );
        // Relative redirect stays on the same authority and port.
        assert_eq!(
            redirect_base(base, "/api/health").as_deref(),
            Some("https://update.example.com:7928/")
        );
        // A redirect into a sub-path keeps that directory as the new base.
        assert_eq!(
            redirect_base(base, "https://other.example:8443/gw/api/health")
                .as_deref(),
            Some("https://other.example:8443/gw/")
        );
        // Garbage never becomes a base.
        assert_eq!(redirect_base(base, "javascript:alert(1)"), None);
    }

}
