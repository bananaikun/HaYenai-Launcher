pub const PRODUCT_NAME: &str = "HaYenai";
pub const SHORT_PRODUCT_NAME: &str = "HaYenai";
pub const WEBSITE: &str = "https://github.com/Bananaikun/HaYenai";
pub const BUNDLE_IDENTIFIER: &str = "com.hayenai.launcher";
pub const DEEP_LINK_SCHEME: &str = "hayenai";

/// SRV record that carries the update service address.
///
/// The service lives on dynamic DNS (`IPv6` first, `CGNAT` behind it) and is
/// fronted by a reverse proxy on a non-standard port, so neither the host nor
/// the port may be compiled in. Resolve this record at runtime instead.
pub const UPDATE_SERVICE_SRV: &str = "";  // 开源版不用 SRV 发现，更新走 GitHub Releases

/// Host the update service is published under.
///
/// Used for the tier-2 `AAAA` lookup: when no SRV record answers, the service is
/// still reachable over IPv6 on a fixed port, because that path does not go
/// through NAT traversal.
pub const UPDATE_SERVICE_HOST: &str = "github.com";

/// Port the IPv6 endpoint listens on.
///
/// Fixed on purpose: only the IPv4/STUN path gets an ephemeral port from the
/// SRV record, the IPv6 listener is bound once.
pub const UPDATE_SERVICE_IPV6_PORT: u16 = 23443;

/// Last-resort base URL behind a Cloudflare tunnel.
///
/// Slower than the direct paths and dependent on the tunnel's ingress rules, so
/// it is only used when both direct tiers fail.
pub const UPDATE_SERVICE_CLOUDFLARE_BASE: &str = "https://placeholder.invalid/";

/// Base URL used when the SRV lookup yields no usable record.
///
/// Plain HTTPS on the default port: the fallback exists so that a broken or
/// missing SRV record degrades to "maybe reachable" instead of "no address".
pub const UPDATE_SERVICE_FALLBACK_BASE: &str = "https://github.com/bananaikun/HaYenai/releases/";

/// Health endpoint path, relative to the resolved base URL.
pub const UPDATE_SERVICE_HEALTH_PATH: &str = "api/health";

/// Environment variable that overrides the resolved base URL.
///
/// Debug and CI only: it is read from the process environment, never from
/// settings, so a shipped build cannot be pointed at another host by accident.
pub const UPDATE_SERVICE_BASE_ENV: &str = "HAYENAI_UPDATE_BASE";

/// Longest accepted data directory suffix; enough to name a branch or a build.
const MAX_DATA_DIR_SUFFIX_LEN: usize = 32;

pub fn user_agent(version: &str, os: &str) -> String {
    format!("hayenai/{version} ({os})")
}

/// Name of the directory the launcher keeps its settings, databases and logs in.
///
/// A build can opt into its own directory by setting `AXOLOTL_DATA_DIR_SUFFIX`
/// at build time (see `build.rs`). Local and test builds use this to stay away
/// from the database of an installed launcher: the two otherwise share one
/// directory, and a database touched by a newer build can no longer be opened
/// by an older one. Releases leave the variable unset, so they resolve to the
/// plain identifier.
pub fn app_data_dir_identifier(app_identifier: &str) -> String {
    data_dir_identifier(app_identifier, option_env!("AXOLOTL_DATA_DIR_SUFFIX"))
}

/// Appends a sanitized suffix to the identifier.
///
/// The result becomes a directory name, so anything that could escape the data
/// directory (path separators, drive letters, traversal) is dropped rather than
/// escaped. A suffix that leaves nothing usable behind is ignored entirely -
/// `build.rs` refuses such a value before it can reach a build, so this fallback
/// is only reachable from a direct call.
fn data_dir_identifier(base: &str, suffix: Option<&str>) -> String {
    let Some(suffix) = suffix else {
        return base.to_string();
    };

    let filtered: String = suffix
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.')
        })
        .collect();

    // Trim on both sides of the length cap. A suffix of dots alone would
    // otherwise spend the whole cap and silently drop what follows it.
    let trimmed = filtered.trim_matches(['.', '-']);
    let capped = &trimmed[..trimmed.len().min(MAX_DATA_DIR_SUFFIX_LEN)];
    let sanitized = capped.trim_matches(['.', '-']);

    if sanitized.is_empty() {
        base.to_string()
    } else {
        format!("{base}-{sanitized}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_is_unique_and_contains_no_contact_information() {
        let user_agent = user_agent("1.2.3", "windows");

        assert_eq!(user_agent, "hayenai/1.2.3 (windows)");
        assert!(!user_agent.contains("ghs.red"));
        assert!(!user_agent.contains("http"));
        assert!(!user_agent.contains('@'));
    }

    #[test]
    fn data_dir_identifier_without_suffix_keeps_the_identifier() {
        assert_eq!(
            data_dir_identifier("com.hayenai.launcher", None),
            "com.hayenai.launcher"
        );
    }

    #[test]
    fn data_dir_identifier_appends_a_readable_suffix() {
        for (suffix, expected) in [
            ("pr538", "com.hayenai.launcher-pr538"),
            ("fix_issue_538", "com.hayenai.launcher-fix_issue_538"),
            ("v1.2.3", "com.hayenai.launcher-v1.2.3"),
        ] {
            assert_eq!(
                data_dir_identifier("com.hayenai.launcher", Some(suffix)),
                expected,
                "unexpected identifier for suffix {suffix:?}"
            );
        }
    }

    #[test]
    fn data_dir_identifier_never_escapes_the_data_directory() {
        for suffix in [
            "../evil",
            "..\\evil",
            "a/b",
            "a\\b",
            "..",
            "/",
            "\\",
            "C:\\Users",
            "\\\\server\\share",
            ".....",
            "..-.-",
        ] {
            let identifier =
                data_dir_identifier("com.hayenai.launcher", Some(suffix));

            assert!(
                !identifier.contains(['/', '\\', ':']),
                "suffix {suffix:?} produced a path separator or drive letter in {identifier:?}"
            );
            assert!(
                !identifier.ends_with(['.', '-']),
                "suffix {suffix:?} left a trailing dot or dash in {identifier:?}, which Windows would strip"
            );

            let remainder = identifier
                .strip_prefix("com.hayenai.launcher-")
                .map_or(identifier.as_str(), |remainder| remainder);
            assert!(
                !remainder.is_empty() && remainder != "." && remainder != "..",
                "suffix {suffix:?} produced the bare component {remainder:?}"
            );
        }
    }

    #[test]
    fn data_dir_identifier_keeps_the_prefix_so_device_names_cannot_win() {
        // Windows treats a path component as a device when the whole name is
        // one. The identifier always carries the prefix, so it never is.
        for suffix in ["CON", "NUL", "PRN", "AUX", "COM1", "LPT1"] {
            assert_eq!(
                data_dir_identifier("com.hayenai.launcher", Some(suffix)),
                format!("com.hayenai.launcher-{suffix}")
            );
        }
    }

    #[test]
    fn data_dir_identifier_drops_characters_that_are_unsafe_in_a_path() {
        for (suffix, expected) in [
            ("a b", "com.hayenai.launcher-ab"),
            ("a:b", "com.hayenai.launcher-ab"),
            // Fullwidth solidus: not a separator the filesystem honours, but no
            // more welcome in a directory name than any other non-ASCII.
            ("a\u{ff0f}b", "com.hayenai.launcher-ab"),
            // Every non-ASCII character goes, so a suffix typed in another
            // script keeps only its ASCII part - which may be nothing at all.
            ("版本v2", "com.hayenai.launcher-v2"),
            ("版本", "com.hayenai.launcher"),
        ] {
            assert_eq!(
                data_dir_identifier("com.hayenai.launcher", Some(suffix)),
                expected,
                "unexpected identifier for suffix {suffix:?}"
            );
        }
    }

    #[test]
    fn data_dir_identifier_trims_before_capping_the_length() {
        // Capping first would let a run of dots spend the whole allowance and
        // drop the characters after it, so the suffix would vanish silently.
        let suffix = format!("{}abc", ".".repeat(40));

        assert_eq!(
            data_dir_identifier("com.hayenai.launcher", Some(&suffix)),
            "com.hayenai.launcher-abc"
        );
    }

    #[test]
    fn data_dir_identifier_ignores_unusable_suffixes() {
        for suffix in ["", "   ", "..", "--", "///", "..-.-"] {
            assert_eq!(
                data_dir_identifier("com.hayenai.launcher", Some(suffix)),
                "com.hayenai.launcher",
                "suffix {suffix:?} should have been ignored"
            );
        }
    }

    #[test]
    fn data_dir_identifier_bounds_the_suffix_length() {
        let identifier =
            data_dir_identifier("com.hayenai.launcher", Some(&"a".repeat(200)));

        assert_eq!(
            identifier,
            format!("com.hayenai.launcher-{}", "a".repeat(MAX_DATA_DIR_SUFFIX_LEN))
        );
    }
}
