use crate::api::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::http::HeaderValue;
use tauri::http::header::ACCEPT;
use tauri::{Manager, ResourceId, Runtime, Webview};
use tauri_plugin_http::reqwest;
use tauri_plugin_http::reqwest::ClientBuilder;
use tauri_plugin_updater::{Error, Update, UpdaterExt};
use theseus::update_service::{UpdateService, UPDATE_SERVICE_FALLBACK_BASE};
use theseus::{
    LoadingBarType, emit_loading, init_loading, launcher_user_agent,
};
use tokio::time::Instant;
use url::Url;

/// Path of the release catalog, relative to the resolved base URL.
///
/// 只有 apt 托管的 Linux 更新还在用 HaYenai 更新服务器的 `/api/versions`
/// （.deb 没有 minisign 签名，得靠目录里的 sha256/size 校验）。Windows/macOS/
/// AppImage 的检查与下载都走 GitHub Releases。
const UPDATE_CATALOG_PATH: &str = "api/versions";

// The updater plugin builds `Update` with no request timeout, so a stalled
// connection would hang the download forever. Bound the whole download.
const UPDATE_DOWNLOAD_TIMEOUT: Duration =
    Duration::from_secs(15 * 60);

// ── Shared types ─────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    rid: ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    force_update: bool,
    raw_json: serde_json::Value,
}

#[derive(Default)]
pub struct PendingUpdateData(pub Mutex<Option<(Arc<Update>, Vec<u8>)>>);

// ── Update Server API types ─────────────────────────────────────

#[derive(Deserialize)]
struct VersionsResponse {
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize)]
struct VersionEntry {
    version: String,
    artifacts: Vec<ArtifactEntry>,
}

#[derive(Deserialize)]
struct ArtifactEntry {
    kind: String,
    #[serde(default)]
    variant: Option<String>,
    platform: String,
    architecture: String,
    relative_path: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: u64,
}

/// The .deb asset for an apt-managed Linux update, from the Update Server
/// catalog (`/api/versions`). The deb has no minisign signature, so its
/// integrity is verified with the catalog's sha256 and size instead.
struct AptDebAsset {
    url: Url,
    sha256: String,
    size: u64,
}

fn apt_deb_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        arch => Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            format!("Unsupported architecture for apt updates: {arch}"),
        ))
        .into()),
    }
}

/// Joins a path onto the resolved update-service base.
///
/// The base always ends with `/`, so a plain concatenation is enough; the
/// fallback constant stands in for the (unlikely) case where resolution has not
/// produced anything yet.
async fn update_server_url(path: &str) -> String {
    let base = UpdateService::shared()
        .resolve_base_url()
        .await
        .unwrap_or_else(|| UPDATE_SERVICE_FALLBACK_BASE.to_string());
    format!("{base}{path}")
}

async fn fetch_apt_deb_asset(version: &str) -> Result<AptDebAsset> {
    let base = update_server_url("").await;
    let catalog_url = update_server_url(UPDATE_CATALOG_PATH).await;
    let response = ClientBuilder::new()
        .user_agent(launcher_user_agent())
        .timeout(UPDATE_DOWNLOAD_TIMEOUT)
        .build()?
        .get(catalog_url)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Failed to fetch update catalog: {}",
            response.status()
        ))
        .into());
    }

    let catalog: VersionsResponse = response.json().await?;
    let release = catalog
        .versions
        .iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                "Update catalog has no entry for version {version}"
            )))
        })?;

    let arch = std::env::consts::ARCH;
    let artifact = release
        .artifacts
        .iter()
        .find(|entry| {
            entry.kind == "installer"
                && entry.variant.as_deref() == Some("deb")
                && entry.platform == "linux"
                && entry.architecture == arch
        })
        .ok_or_else(|| {
            theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                "Update catalog has no deb artifact for {version} on {arch}"
            )))
        })?;

    let sha256 = artifact.sha256.clone().ok_or_else(|| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Update catalog has no sha256 for the deb artifact of {version}"
        )))
    })?;

    let url =
        Url::parse(&format!("{base}{}", artifact.relative_path)).map_err(
            |error| {
                theseus::Error::from(theseus::ErrorKind::OtherError(
                    error.to_string(),
                ))
            },
        )?;

    Ok(AptDebAsset {
        url,
        sha256,
        size: artifact.size,
    })
}

// ── Updater plugin helpers ───────────────────────────────────────

fn update_channel(channel: &str) -> Result<&str> {
    match channel {
        "release" | "beta" => Ok(channel),
        _ => Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            format!("Unknown update channel: {channel}"),
        ))
        .into()),
    }
}

fn update_platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x86_64"),
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("linux", "aarch64") => Ok("linux-aarch64"),
        ("macos", "x86_64") => Ok("darwin-x86_64"),
        ("macos", "aarch64") => Ok("darwin-aarch64"),
        (os, arch) => {
            Err(theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                "Unsupported updater platform: {os}-{arch}"
            )))
            .into())
        }
    }
}

/// GitHub Releases 上那份签名过的 `latest.json`。
///
/// 之前这里是「DNS SRV 解析出的 HaYenai 更新服务器 + `/latest`」；换源到 GitHub 后
/// 地址是固定的，不需要任何解析，也不再需要先探测服务是否在线。
/// 两个通道（release / beta）暂时共用同一份清单：每次发版都会覆盖它。
const GITHUB_UPDATE_ENDPOINT: &str =
    "https://github.com/bananaikun/HaYenai-Launcher/releases/latest/download/latest.json";

/* ---------------- GitHub 加速镜像 ----------------
   国内直连 release 资产实测只有 40 KB/s 左右（65 MB 的安装包要半小时）。
   这些是「前缀式」代理：把整条 GitHub URL 拼在后面即可，
   `https://gh-proxy.com/https://github.com/...`。

   两个用途：
   1. 清单（latest.json）也走镜像，check 才快；
   2. 真正的下载地址在 check 之后**现测现选**最够快的那个（见
      `fastest_download_url`），全挂了就退回 GitHub 原始地址。
   ⚠ 签名与 sha256 都是对文件内容做的，镜像只是转发字节，校验不受影响。 */
const GITHUB_MIRRORS: &[&str] = &["https://gh-proxy.com/", "https://ghfast.top/"];

/// 探测镜像时最多取这么多字节（够算出速度，又不至于白下太多）。
const MIRROR_PROBE_BYTES: u64 = 1_048_576;
/// 单个候选的探测预算。
const MIRROR_PROBE_BUDGET: Duration = Duration::from_secs(6);
/// 低于这个速度就认为「跟直连差不多」，继续找下一个。
const MIRROR_MIN_BYTES_PER_SEC: f64 = 250_000.0;

/// 把 GitHub 地址套上镜像前缀。只处理 `https://github.com/...`。
fn mirrored(url: &Url, mirror: &str) -> Option<Url> {
    if url.scheme() != "https" || url.host_str()? != "github.com" {
        return None;
    }
    Url::parse(&format!("{mirror}{url}")).ok()
}

/// 清单端点：镜像优先，最后才是 GitHub 直连（插件按顺序尝试，失败才降级）。
async fn manifest_endpoints() -> Result<Vec<Url>> {
    let base = update_endpoint().await?;
    let mut list: Vec<Url> = GITHUB_MIRRORS
        .iter()
        .filter_map(|mirror| mirrored(&base, mirror))
        .collect();
    list.push(base);
    Ok(list)
}

/// The updater manifest URL.
async fn update_endpoint() -> Result<Url> {
    Url::parse(GITHUB_UPDATE_ENDPOINT).map_err(|error| {
        theseus::Error::from(theseus::ErrorKind::OtherError(error.to_string()))
            .into()
    })
}

/// 取一小段数据，算这个候选的真实速度（字节/秒）。失败返回 `None`。
async fn probe_speed(client: &reqwest::Client, url: &Url) -> Option<f64> {
    let started = Instant::now();
    let response = client
        .get(url.clone())
        .header(
            reqwest::header::RANGE,
            format!("bytes=0-{}", MIRROR_PROBE_BYTES - 1),
        )
        .timeout(MIRROR_PROBE_BUDGET)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let mut received = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        received += chunk.len() as u64;
        if received >= MIRROR_PROBE_BYTES || started.elapsed() > MIRROR_PROBE_BUDGET {
            break;
        }
    }
    if received == 0 {
        return None;
    }
    let seconds = started.elapsed().as_secs_f64().max(0.001);
    Some(received as f64 / seconds)
}

/// 给下载地址挑一个够快的镜像：逐个探测，达到阈值就立刻用；
/// 都不行就退回原始 GitHub 地址（保持「至少能下」）。
async fn fastest_download_url(original: &Url) -> Url {
    let candidates: Vec<Url> = GITHUB_MIRRORS
        .iter()
        .filter_map(|mirror| mirrored(original, mirror))
        .collect();
    if candidates.is_empty() {
        return original.clone();
    }

    let client = match ClientBuilder::new()
        .user_agent(launcher_user_agent())
        .build()
    {
        Ok(client) => client,
        Err(_) => return original.clone(),
    };

    let mut best: Option<(f64, Url)> = None;
    for candidate in candidates {
        match probe_speed(&client, &candidate).await {
            Some(speed) if speed >= MIRROR_MIN_BYTES_PER_SEC => {
                tracing::info!(
                    mirror = %candidate, speed_kib = (speed / 1024.0) as u64,
                    "update download mirror selected"
                );
                return candidate;
            }
            Some(speed) => {
                tracing::debug!(mirror = %candidate, speed_kib = (speed / 1024.0) as u64, "mirror too slow");
                if best.as_ref().map(|(s, _)| speed > *s).unwrap_or(true) {
                    best = Some((speed, candidate));
                }
            }
            None => {
                tracing::debug!(mirror = %candidate, "mirror probe failed");
            }
        }
    }

    match best {
        Some((_, url)) => url,
        None => original.clone(),
    }
}

/// Build the platform-updater with the given endpoints and run a check.
async fn check_with_endpoints<R: Runtime>(
    webview: &Webview<R>,
    channel: &str,
) -> Result<Option<Update>> {
    let channel = update_channel(channel)?;
    let platform = update_platform()?;
    let current_version =
        webview.app_handle().package_info().version.to_string();

    let mut updater = webview
        .updater_builder()
        .endpoints(manifest_endpoints().await?)?
        .header("Accept", "application/json")?
        .header("X-HaYenai-Channel", channel)?
        .header("X-HaYenai-Platform", platform)?
        .header("X-HaYenai-Version", current_version)?;

    #[cfg(target_os = "windows")]
    {
        let install_dir = std::env::current_exe()
            .map_err(|error| {
                theseus::Error::from(theseus::ErrorKind::OtherError(format!(
                    "Failed to resolve current executable: {error}"
                )))
            })?
            .parent()
            .ok_or_else(|| {
                theseus::Error::from(theseus::ErrorKind::OtherError(
                    "Current executable has no parent directory".to_string(),
                ))
            })?
            .to_path_buf();

        tracing::debug!(
            install_dir = %install_dir.display(),
            "Using current executable directory for Windows app updates"
        );
        updater = updater.installer_arg(format!(
            "/INSTALL_DIR=\"{}\"",
            install_dir.display()
        ));
    }

    let updater = updater.build()?;
    updater.check().await.map_err(Into::into)
}

/// Check the updater manifest through the configured Update Server endpoint.
async fn check_with_updater<R: Runtime>(
    webview: &Webview<R>,
    channel: &str,
) -> Result<Option<UpdateMetadata>> {
    let Some(mut update) = check_with_endpoints(webview, channel).await? else {
        return Ok(None);
    };
    update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);

    // On Debian and derivatives the plugin's minisign signature check cannot
    // validate the unsigned .deb, so point the download at the deb from the
    // Update Server catalog instead of the AppImage artifact. Its integrity
    // is verified with the catalog's sha256/size during the download.
    if is_apt_linux() {
        update.download_url = fetch_apt_deb_asset(&update.version).await?.url;
    } else {
        // 直连 GitHub 只有几十 KB/s；换成一个实测够快的镜像（都不行就保持原样）。
        update.download_url = fastest_download_url(&update.download_url).await;
    }

    let published_at = update
        .raw_json
        .get("published_at")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let force_update = update
        .raw_json
        .get("force_update")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let metadata = UpdateMetadata {
        rid: webview.resources_table().add(update.clone()),
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date: None,
        body: update.body.clone(),
        published_at,
        force_update,
        raw_json: update.raw_json,
    };

    Ok(Some(metadata))
}

// ── Tauri commands ───────────────────────────────────────────────

#[tauri::command]
pub async fn check_app_update<R: Runtime>(
    webview: Webview<R>,
    channel: String,
) -> Result<Option<UpdateMetadata>> {
    check_with_updater(&webview, &channel).await
}

// Reimplementation of Update::download mostly, minus the actual download part
#[tauri::command]
pub async fn get_update_size<R: Runtime>(
    webview: Webview<R>,
    rid: ResourceId,
) -> Result<Option<u64>> {
    let update = webview.resources_table().get::<Update>(rid)?;

    let mut headers = update.headers.clone();
    if !headers.contains_key(ACCEPT) {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/octet-stream"),
        );
    }

    let mut request = ClientBuilder::new().user_agent(launcher_user_agent());
    if let Some(timeout) = update.timeout {
        request = request.timeout(timeout);
    }
    if let Some(ref proxy) = update.proxy {
        let proxy = reqwest::Proxy::all(proxy.as_str())?;
        request = request.proxy(proxy);
    }
    let response = request
        .build()?
        .head(update.download_url.clone())
        .headers(headers)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "Download request failed with status: {}",
            response.status()
        ))
        .into());
    }

    let content_length = response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());

    Ok(content_length)
}

#[tauri::command]
pub async fn enqueue_update_for_installation<R: Runtime>(
    webview: Webview<R>,
    rid: ResourceId,
) -> Result<()> {
    let pending_data = webview.state::<PendingUpdateData>().inner();

    let update = webview.resources_table().get::<Update>(rid)?;

    let progress = init_loading(
        LoadingBarType::LauncherUpdate {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
        },
        1.0,
        "Downloading update...",
    )
    .await?;

    let download_start = Instant::now();
    let update_data = if is_apt_linux() {
        // The .deb carries no minisign signature, so the plugin's signed
        // download cannot be used. Fetch the catalog entry and verify the
        // downloaded bytes against its sha256 and size instead.
        let asset = fetch_apt_deb_asset(&update.version).await?;

        let mut headers = update.headers.clone();
        if !headers.contains_key(ACCEPT) {
            headers.insert(
                ACCEPT,
                HeaderValue::from_static("application/octet-stream"),
            );
        }

        let mut request =
            ClientBuilder::new().user_agent(launcher_user_agent());
        if let Some(timeout) = update.timeout {
            request = request.timeout(timeout);
        }
        if let Some(ref proxy) = update.proxy {
            let proxy = reqwest::Proxy::all(proxy.as_str())?;
            request = request.proxy(proxy);
        }
        let response = request
            .build()?
            .get(update.download_url.clone())
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::Network(format!(
                "Download request failed with status: {}",
                response.status()
            ))
            .into());
        }

        let total_size = response.content_length().unwrap_or(asset.size);
        let mut buffer = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.extend_from_slice(&chunk);
            if total_size > 0
                && let Err(e) = emit_loading(
                    &progress,
                    buffer.len() as f64 / total_size as f64,
                    None,
                )
            {
                tracing::error!("Failed to update download progress bar: {e}");
            }
        }

        if buffer.len() as u64 != asset.size {
            return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
                format!(
                    "Downloaded deb size mismatch: expected {}, got {}",
                    asset.size,
                    buffer.len()
                ),
            ))
            .into());
        }

        let digest = Sha256::digest(&buffer);
        let digest_hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if digest_hex != asset.sha256 {
            return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
                "Downloaded deb sha256 mismatch".to_string(),
            ))
            .into());
        }

        buffer
    } else {
        update
            .download(
                |chunk_size, total_size| {
                    let Some(total_size) = total_size else {
                        return;
                    };
                    if let Err(e) = emit_loading(
                        &progress,
                        chunk_size as f64 / total_size as f64,
                        None,
                    ) {
                        tracing::error!(
                            "Failed to update download progress bar: {e}"
                        );
                    }
                },
                || {},
            )
            .await?
    };
    let download_duration = download_start.elapsed();
    tracing::info!("Downloaded update in {download_duration:?}");

    pending_data
        .0
        .lock()
        .unwrap()
        .replace((update, update_data));

    Ok(())
}

#[tauri::command]
pub fn remove_enqueued_update<R: Runtime>(webview: Webview<R>) {
    let pending_data = webview.state::<PendingUpdateData>().inner();
    pending_data.0.lock().unwrap().take();
}

// ── Debian / derivatives apt update ─────────────────────────────

/// Whether this Linux system updates HaYenai through apt (Debian and its
/// derivatives) and has `pkexec` available for a single privileged prompt.
#[tauri::command]
pub fn is_apt_linux() -> bool {
    #[cfg(target_os = "linux")]
    {
        let debian_like = std::path::Path::new("/etc/debian_version").exists()
            || std::path::Path::new("/etc/apt").is_dir()
            || std::path::Path::new("/usr/bin/apt-get").exists();
        let has_pkexec = ["/usr/bin/pkexec", "/bin/pkexec"]
            .iter()
            .any(|path| std::path::Path::new(path).exists());
        debian_like && has_pkexec
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Install the downloaded .deb on Debian and its derivatives, prompting for
/// root once via `pkexec`. The package is installed from the absolute path
/// of a temporary file, which is removed afterwards.
pub async fn install_apt_package(version: &str, data: &[u8]) -> Result<()> {
    if !is_apt_linux() {
        return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            "apt updates are only supported on Debian-based Linux systems with pkexec"
                .to_string(),
        ))
        .into());
    }

    let arch = apt_deb_arch()?;
    let deb_path = std::env::temp_dir()
        .join(format!("HaYenai.Launcher_{version}_{arch}.deb"));
    std::fs::write(&deb_path, data).map_err(|io| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Failed to write the downloaded deb: {io}"
        )))
    })?;
    let _deb_cleanup = TempDebFile(deb_path.clone());

    let install_path = deb_path.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("pkexec")
            .arg("apt")
            .arg("install")
            .arg("-y")
            .arg(&install_path)
            .output()
    })
    .await
    .map_err(|join| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Failed to run the apt updater: {join}"
        )))
    })?
    .map_err(|io| {
        theseus::Error::from(theseus::ErrorKind::OtherError(format!(
            "Failed to start pkexec: {io}"
        )))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(theseus::Error::from(theseus::ErrorKind::OtherError(
            format!("apt install failed: {}", stderr.trim()),
        ))
        .into());
    }

    Ok(())
}

/// Removes a temporary Debian package when installation finishes or fails.
///
/// The installer awaits a blocking task and has several fallible operations
/// after creating the file. Keeping cleanup in `Drop` makes every return path
/// (including task and process-launch errors) remove the package.
struct TempDebFile(std::path::PathBuf);

impl Drop for TempDebFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.0.display(),
                error = %error,
                "Failed to remove temporary deb file"
            );
        }
    }
}
