//! `hayenai-we` 取流协议的纯逻辑：路径映射、MIME、缓存头与 Range。
//!
//! 渲染器是「按名逐个取文件」的：`webwallgl` 拿 `httpSource(baseUrl)` 之后会依次
//! 请求 `project.json` / `scene.json` / `materials/*.json` / `*.tex` / `*.mp4`。
//! 所以这一层的形状必须和后台的 `GET /api/we/pkg/<id>/<rel>` 1:1 —— 本模块就是
//! 那份行为的本地实现，只是把 HTTP 换成 Tauri 的自定义协议。
//!
//! 与 Tauri 解耦的部分全在这里（返回普通的 `status + headers + body`），
//! `apps/app/src/we_protocol.rs` 只负责把请求拆出来、把结果包成 `Response`。
//! 这样这段逻辑在 `theseus` 里就能跑单测。
//!
//! 三条硬规则：
//!
//! 1. **路径穿越挡在读取之前**：URL 解码之后再过一遍 [`safe_rel`]，
//!    `%2e%2e%2f` 与 `..` 一样被拒；打开文件后再用 `canonicalize` 复核一次，
//!    防包内符号链接指向外面。
//! 2. **永不整文件读**：Range 请求 `seek` 到起点后只读这一段。
//! 3. **内容寻址 ⇒ 可以 immutable**：id 里带内容哈希，同一个 URL 的字节不会变。
//!
//! 除 `/<壁纸 id>/<rel>`（已导入的包）之外，还有一条**只读**的旁路：
//! `/steam/<工坊 id>/<rel>` 直接映射到 Steam 库里的工坊目录
//! （见 [`super::steam`]）。这样前端可以把 `preview.gif` 当缩略图用，
//! 不必为了「扫一眼」把 1.8 GB 复制一遍。工坊目录会随订阅更新而变化，
//! 所以那条旁路用短缓存（[`STEAM_CACHE_CONTROL`]）而不是 `immutable`。

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::pkg::safe_rel;
use super::steam::is_valid_workshop_id;
use super::store::{PACKAGES_DIR_NAME, is_valid_id};

/// 自定义协议名。
pub const SCHEME: &str = "hayenai-we";
/// Windows 上自定义协议映射到的域名（`http://hayenai-we.localhost/<id>/<rel>`）。
pub const HOST: &str = "hayenai-we.localhost";
/// 缓存策略：id 带内容哈希，同一个 URL 的字节永不改变。
pub const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
/// Steam 工坊目录的缓存策略。
///
/// 那边的文件属于 Steam（订阅更新会原地改内容），URL 里也没有内容哈希，
/// 所以**不能**标 `immutable`；5 分钟足够挡住「一屏 20 张缩略图」的重复请求。
pub const STEAM_CACHE_CONTROL: &str = "public, max-age=300";
/// 工坊取流路径的前缀（`/steam/<id>/<rel>`）。
pub const STEAM_PREFIX: &str = "steam/";
/// 一次响应最多吐多少字节。
///
/// Tauri 的同步协议 handler 只能返回 `Vec<u8>`，没有流式回退；
/// 超过这个量级的**整文件**请求直接拒绝，免得一张 1 GB 的视频把内存打满。
/// 视频元素与 `webwallgl` 都发 Range 请求，正常路径不会撞到这条线。
pub const MAX_BODY_BYTES: u64 = 512 * 1024 * 1024;

/// 一次取流请求里我们关心的东西。
#[derive(Debug, Clone, Copy)]
pub struct ProtocolRequest<'a> {
    pub method: &'a str,
    /// 原始（未解码）的 URL 路径。
    pub path: &'a str,
    /// `Range` 头原值。
    pub range: Option<&'a str>,
    /// `If-None-Match` 头原值。
    pub if_none_match: Option<&'a str>,
}

/// 解析结果：状态码 + 头 + 体。头名一律小写，值是最终字符串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl ProtocolResponse {
    /// 空体响应（错误与 304 用）。
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// 按名字找响应头，单测用。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// 处理一次取流请求。
///
/// `root` 是 `wallpapers/` 根目录（`packages/` 在它下面）。
///
/// 这个入口**不认** `/steam/…` 旁路（没有 Steam 目录表可查，一律 404）；
/// Tauri 适配层走 [`resolve_with_steam`]。
pub fn resolve(root: &Path, request: &ProtocolRequest<'_>) -> ProtocolResponse {
    resolve_with_steam(root, request, &|_| None)
}

/// 同 [`resolve`]，另外接上 `/steam/<工坊 id>/<rel>` 这条只读旁路。
///
/// `steam_lookup` 负责把工坊 id 翻成本机上那个目录；返回 `None` 就是 404。
/// 传进来的实现应该是 [`super::steam::remembered_dir`] —— 它只认「本次扫描
/// 真正看到过的目录」，前端给不出任意路径。
pub fn resolve_with_steam(
    root: &Path,
    request: &ProtocolRequest<'_>,
    steam_lookup: &dyn Fn(&str) -> Option<PathBuf>,
) -> ProtocolResponse {
    /* CORS 预检。
       `<video crossorigin="anonymous">` 是为了让画进 canvas 的视频帧不被污染
       （见 `VideoWallpaper.vue`）—— 而一旦走 CORS 模式，浏览器的 `Range` 请求
       就不再是"简单请求"，会**先发 OPTIONS 预检**。
       预检被 405 掉，视频就直接加载失败、壁纸退回海报 —— 实测正是这个症状
       （「囍」的 99MB mp4 报 video-error）。所以这里必须放行 OPTIONS。
       真正的 CORS 响应头由 `apps/app/src/we_protocol.rs` 统一加。 */
    if request.method == "OPTIONS" {
        return ProtocolResponse {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        };
    }

    if !matches!(request.method, "GET" | "HEAD") {
        return ProtocolResponse::empty(405);
    }

    // 1. 先解码再校验：`%2e%2e%2f` 解码后就是 `../`，必须走同一套检查。
    let Some(decoded) = percent_decode(request.path) else {
        return ProtocolResponse::empty(400);
    };

    // 2. `/steam/<id>/<rel>`：映射到 Steam 工坊目录（只读）。
    //    **在**解码之后判前缀，`/%73team/…` 也躲不过去。
    if let Some(rest) = strip_steam_prefix(&decoded) {
        return resolve_steam(rest, request, steam_lookup);
    }

    // 3. 否则就是已导入的包：`/<id>/<rel>`。
    let Some((id, rel_raw)) = split_id_and_rel(&decoded) else {
        return ProtocolResponse::empty(400);
    };
    if !is_valid_id(id) {
        return ProtocolResponse::empty(403);
    }
    let Some(rel) = safe_rel(rel_raw) else {
        return ProtocolResponse::empty(403);
    };

    let packages = root.join(PACKAGES_DIR_NAME);
    serve_file(
        &packages.join(id).join(&rel),
        &packages,
        &rel,
        request,
        CACHE_CONTROL,
    )
}

/// 去掉开头的 `/` 与前缀 `steam/`；不是工坊路径就 `None`。
fn strip_steam_prefix(path: &str) -> Option<&str> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    trimmed.strip_prefix(STEAM_PREFIX)
}

/// `/steam/<工坊 id>/<rel>` 的落点：先查目录表，再当普通文件伺服。
fn resolve_steam(
    rest: &str,
    request: &ProtocolRequest<'_>,
    steam_lookup: &dyn Fn(&str) -> Option<PathBuf>,
) -> ProtocolResponse {
    let Some((id, rel_raw)) = rest.split_once('/') else {
        return ProtocolResponse::empty(400);
    };
    if id.is_empty() || rel_raw.is_empty() {
        return ProtocolResponse::empty(400);
    }
    // 工坊 id 只接受纯数字：它既拼路径，也出现在 URL 里。
    if !is_valid_workshop_id(id) {
        return ProtocolResponse::empty(403);
    }
    let Some(rel) = safe_rel(rel_raw) else {
        return ProtocolResponse::empty(403);
    };
    let Some(dir) = steam_lookup(id) else {
        // 没扫到 / 结果过期 / 已经取消订阅，都是「现在没有」。
        return ProtocolResponse::empty(404);
    };
    serve_file(
        &dir.join(&rel),
        &dir,
        &rel,
        request,
        STEAM_CACHE_CONTROL,
    )
}

/// 真正读盘那一段：定位 → 复核边界 → ETag → Range → 只读这一段。
///
/// `candidate` 是最终文件，`boundary` 是它必须待着的目录（`packages/`
/// 或某个工坊条目目录）。`cache_control` 由调用方决定要不要 `immutable`。
fn serve_file(
    candidate: &Path,
    boundary: &Path,
    rel: &str,
    request: &ProtocolRequest<'_>,
    cache_control: &'static str,
) -> ProtocolResponse {
    let Ok(metadata) = fs::metadata(candidate) else {
        return ProtocolResponse::empty(404);
    };
    if !metadata.is_file() {
        return ProtocolResponse::empty(404);
    }

    // 复核一次：canonicalize 之后必须仍在边界目录下。
    // 词法校验挡不住「包目录里被人塞了一个指向外面的符号链接」。
    let (Ok(canonical_boundary), Ok(canonical_file)) = (
        fs::canonicalize(boundary),
        fs::canonicalize(candidate),
    ) else {
        return ProtocolResponse::empty(404);
    };
    if !canonical_file.starts_with(&canonical_boundary) {
        return ProtocolResponse::empty(403);
    }

    let size = metadata.len();
    let etag = make_etag(size, modified_millis(&metadata), rel);
    let content_type = mime_for(rel);

    // 命中 ETag 就走 304：渲染器重复取同一份贴图时能省掉一次传输。
    if let Some(header) = request.if_none_match
        && etag_matches(header, &etag)
    {
        return ProtocolResponse {
            status: 304,
            headers: vec![
                ("etag", etag),
                ("cache-control", cache_control.to_string()),
                ("accept-ranges", "bytes".to_string()),
            ],
            body: Vec::new(),
        };
    }

    // Range。
    let (status, start, length, extra) =
        match request.range.and_then(|value| parse_range(value, size)) {
            Some(RangeDecision::Unsatisfiable) => {
                return ProtocolResponse {
                    status: 416,
                    headers: vec![
                        ("content-range", format!("bytes */{size}")),
                        ("accept-ranges", "bytes".to_string()),
                        ("cache-control", cache_control.to_string()),
                        ("etag", etag),
                    ],
                    body: Vec::new(),
                };
            }
            Some(RangeDecision::Satisfiable { start, end }) => (
                206,
                start,
                end - start + 1,
                vec![("content-range", format!("bytes {start}-{end}/{size}"))],
            ),
            // 没有 Range，或者 Range 写法不认识（多段等）：按 RFC 忽略，回全量。
            None => (200, 0, size, Vec::new()),
        };

    if length > MAX_BODY_BYTES {
        tracing::warn!(
            size,
            length,
            "wallpaper 取流请求超过单次响应上限，已拒绝"
        );
        return ProtocolResponse::empty(500);
    }

    let Some(body) = read_slice(&canonical_file, start, length) else {
        return ProtocolResponse::empty(500);
    };

    let mut headers: Vec<(&'static str, String)> = vec![
        ("content-type", content_type.to_string()),
        ("content-length", length.to_string()),
        ("accept-ranges", "bytes".to_string()),
        ("cache-control", cache_control.to_string()),
        ("etag", etag),
        ("x-content-type-options", "nosniff".to_string()),
    ];
    headers.extend(extra);

    ProtocolResponse {
        status,
        headers,
        body: if request.method == "HEAD" { Vec::new() } else { body },
    }
}

/// 百分比解码。**只**认 `%XX`，`+` 原样保留（那是表单编码，不是路径编码）。
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 把 `/<id>/<rel>` 拆开。缺 id 或缺 rel 都算 400。
fn split_id_and_rel(path: &str) -> Option<(&str, &str)> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let (id, rel) = trimmed.split_once('/')?;
    if id.is_empty() || rel.is_empty() {
        return None;
    }
    Some((id, rel))
}

/// ETag：大小 + 修改时间 + 路径散列。三者一起变才换 ETag。
fn make_etag(size: u64, modified_millis: u64, rel: &str) -> String {
    format!(
        "\"{size:x}-{modified_millis:x}-{:x}\"",
        fnv1a(rel.as_bytes())
    )
}

/// `If-None-Match` 比对：支持逗号分隔的多值与 `W/` 弱校验前缀。
fn etag_matches(candidate: &str, etag: &str) -> bool {
    candidate.split(',').map(str::trim).any(|value| {
        value == "*" || value.trim_start_matches("W/") == etag
    })
}

fn modified_millis(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// FNV-1a 64：给 ETag 里的路径用，不需要密码学强度。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Range 头的解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeDecision {
    /// 可以满足：闭区间 `start..=end`。
    Satisfiable { start: u64, end: u64 },
    /// 起点超过文件长度（`bytes=<size>-` 这一类）。
    Unsatisfiable,
}

/// 解析 `Range`。返回 `None` 表示「这个头我不认，按 200 处理」（RFC 7233 允许忽略）。
fn parse_range(header: &str, size: u64) -> Option<RangeDecision> {
    let spec = header.trim();
    let list = spec.strip_prefix("bytes=")?;
    let mut ranges = list.split(',');
    let first = ranges.next()?.trim();
    // 多段 Range 一律忽略：我们没必要拼 multipart。
    if ranges.next().is_some() {
        return None;
    }

    let (start_raw, end_raw) = first.split_once('-')?;
    let start_raw = start_raw.trim();
    let end_raw = end_raw.trim();

    if start_raw.is_empty() {
        // 后缀形式 `bytes=-N`：最后 N 字节。
        let suffix = end_raw.parse::<u64>().ok()?;
        if suffix == 0 || size == 0 {
            return Some(RangeDecision::Unsatisfiable);
        }
        return Some(RangeDecision::Satisfiable {
            start: size.saturating_sub(suffix),
            end: size - 1,
        });
    }

    let start = start_raw.parse::<u64>().ok()?;
    if start >= size {
        return Some(RangeDecision::Unsatisfiable);
    }
    let end = if end_raw.is_empty() {
        size - 1
    } else {
        end_raw.parse::<u64>().ok()?.min(size - 1)
    };
    if end < start {
        return Some(RangeDecision::Unsatisfiable);
    }
    Some(RangeDecision::Satisfiable { start, end })
}

/// 只读 `[start, start + length)` 这一段，绝不整文件读进内存。
fn read_slice(path: &Path, start: u64, length: u64) -> Option<Vec<u8>> {
    if length == 0 {
        return Some(Vec::new());
    }
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut body = vec![0_u8; length as usize];
    file.read_exact(&mut body).ok()?;
    Some(body)
}

/// 扩展名 → MIME。 `.tex` 与 `.pkg` 一律 `application/octet-stream`：
/// 它们是渲染器按字节读的私有格式，不该被当成可渲染对象。
pub fn mime_for(rel: &str) -> &'static str {
    let extension = Path::new(rel)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => "application/json; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" | "md" | "vert" | "frag" | "glsl" => {
            "text/plain; charset=utf-8"
        }
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "tex" | "pkg" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// 进程内存读数，只给「大文件不整读」这条验收用。
#[cfg(all(test, windows))]
mod memory {
    use std::ffi::c_void;
    use std::mem::size_of;

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCountersEx {
        base: ProcessMemoryCounters,
        private_usage: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCountersEx2 {
        base: ProcessMemoryCountersEx,
        private_working_set_size: usize,
        shared_commit_usage: u64,
    }

    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut c_void,
            cb: u32,
        ) -> i32;
    }

    /// `PROCESS_MEMORY_COUNTERS_EX2.PrivateWorkingSetSize` —— 就是「私有工作集」。
    pub fn private_working_set() -> Option<usize> {
        let mut counters = ProcessMemoryCountersEx2 {
            base: ProcessMemoryCountersEx {
                base: ProcessMemoryCounters {
                    cb: size_of::<ProcessMemoryCountersEx2>() as u32,
                    ..ProcessMemoryCounters::default()
                },
                private_usage: 0,
            },
            private_working_set_size: 0,
            shared_commit_usage: 0,
        };
        // SAFETY: 缓冲区按 Windows 的结构体布局手写，大小写进 cb；
        // 调用只写这块我们自己拥有的内存。
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                GetCurrentProcess(),
                std::ptr::addr_of_mut!(counters).cast(),
                size_of::<ProcessMemoryCountersEx2>() as u32,
            )
        };
        (ok != 0).then_some(counters.private_working_set_size)
    }

    /// 退路：`WorkingSetSize`（含共享页），老系统上 EX2 可能不被支持。
    pub fn working_set() -> Option<usize> {
        let mut counters = ProcessMemoryCountersEx {
            base: ProcessMemoryCounters {
                cb: size_of::<ProcessMemoryCountersEx>() as u32,
                ..ProcessMemoryCounters::default()
            },
            private_usage: 0,
        };
        // SAFETY: 同上，`cb` 说明了缓冲区布局。
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                GetCurrentProcess(),
                std::ptr::addr_of_mut!(counters).cast(),
                size_of::<ProcessMemoryCountersEx>() as u32,
            )
        };
        (ok != 0).then_some(counters.base.working_set_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ID: &str = "local-river-0123456789ab";

    /// 造一个 `wallpapers/packages/<id>/...` 的最小布局。
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("wallpapers");
        let package = root.join(PACKAGES_DIR_NAME).join(ID);
        fs::create_dir_all(&package).expect("mkdir");
        fs::write(package.join("project.json"), b"{\"type\":\"scene\"}")
            .expect("write project.json");
        fs::write(package.join("materials.json"), vec![b'x'; 4096])
            .expect("write materials.json");
        (temp, root)
    }

    fn get<'a>(path: &'a str) -> ProtocolRequest<'a> {
        ProtocolRequest {
            method: "GET",
            path,
            range: None,
            if_none_match: None,
        }
    }

    #[test]
    fn serves_a_file_with_cache_headers() {
        let (_temp, root) = fixture();

        let response = resolve(
            &root,
            &get(&format!("/{ID}/project.json")),
        );

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"type\":\"scene\"}");
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(response.header("cache-control"), Some(CACHE_CONTROL));
        let expected_length = response.body.len().to_string();
        assert_eq!(
            response.header("content-length"),
            Some(expected_length.as_str())
        );
        assert_eq!(response.header("accept-ranges"), Some("bytes"));
        assert!(response.header("etag").is_some_and(|value| {
            value.starts_with('"') && value.ends_with('"')
        }));
    }

    #[test]
    fn rejects_traversal_in_every_shape() {
        let (_temp, root) = fixture();
        let outside = root.join("secret.txt");
        fs::write(&outside, b"secret").expect("write secret");

        for path in [
            format!("/{ID}/../../../secret.txt"),
            format!("/{ID}/..%2f..%2f..%2fsecret.txt"),
            format!("/{ID}/%2e%2e%2f%2e%2e%2fsecret.txt"),
            format!("/{ID}/%2E%2E%2Fsecret.txt"),
            format!("/{ID}/..\\..\\secret.txt"),
            format!("/{ID}/C:/Windows/win.ini"),
            format!("/{ID}/%2fetc%2fpasswd"),
            format!("/{ID}/project.json%00.txt"),
            format!("/../{ID}/project.json"),
        ] {
            let response = resolve(&root, &get(&path));
            assert!(
                matches!(response.status, 400 | 403 | 404),
                "{path} should not be served, got {}",
                response.status
            );
            assert!(
                !response.body.starts_with(b"secret"),
                "{path} leaked the file outside wallpapers"
            );
        }
    }

    #[test]
    fn rejects_invalid_ids_and_malformed_paths() {
        let (_temp, root) = fixture();

        assert_eq!(resolve(&root, &get("/")).status, 400);
        assert_eq!(resolve(&root, &get(&format!("/{ID}"))).status, 400);
        assert_eq!(resolve(&root, &get(&format!("/{ID}/"))).status, 400);
        // id 不合法 ⇒ 403，不是 404：路径形状本身就不被接受。
        assert_eq!(
            resolve(&root, &get("/..%2fwallpapers/project.json")).status,
            403
        );
        assert_eq!(
            resolve(&root, &get("/a b/project.json")).status,
            403
        );
        // 合法 id 但包不存在 ⇒ 404。
        assert_eq!(
            resolve(&root, &get("/local-ghost-000000000000/x.json")).status,
            404
        );
    }

    #[test]
    fn returns_304_when_the_etag_matches() {
        let (_temp, root) = fixture();
        let path = format!("/{ID}/project.json");
        let first = resolve(&root, &get(&path));
        let etag = first.header("etag").expect("etag").to_string();

        for candidate in [etag.clone(), format!("W/{etag}"), format!("{etag}, \"other\"")] {
            let response = resolve(
                &root,
                &ProtocolRequest {
                    if_none_match: Some(&candidate),
                    ..get(&path)
                },
            );
            assert_eq!(response.status, 304, "{candidate} should hit");
            assert!(response.body.is_empty());
            assert_eq!(response.header("etag"), Some(etag.as_str()));
        }

        let response = resolve(
            &root,
            &ProtocolRequest {
                if_none_match: Some("\"different\""),
                ..get(&path)
            },
        );
        assert_eq!(response.status, 200);
    }

    #[test]
    fn serves_a_byte_range() {
        let (_temp, root) = fixture();
        let path = format!("/{ID}/materials.json");

        let response = resolve(
            &root,
            &ProtocolRequest {
                range: Some("bytes=0-99"),
                ..get(&path)
            },
        );

        assert_eq!(response.status, 206);
        assert_eq!(response.body.len(), 100);
        assert_eq!(
            response.header("content-range"),
            Some("bytes 0-99/4096")
        );
        assert_eq!(response.header("content-length"), Some("100"));
        assert_eq!(response.header("cache-control"), Some(CACHE_CONTROL));
    }

    #[test]
    fn clamps_an_end_past_the_file_size() {
        let (_temp, root) = fixture();
        let path = format!("/{ID}/materials.json");

        let response = resolve(
            &root,
            &ProtocolRequest {
                range: Some("bytes=4090-9999"),
                ..get(&path)
            },
        );

        assert_eq!(response.status, 206);
        assert_eq!(response.header("content-range"), Some("bytes 4090-4095/4096"));
        assert_eq!(response.body.len(), 6);
    }

    #[test]
    fn returns_416_for_an_unsatisfiable_range() {
        let (_temp, root) = fixture();
        let path = format!("/{ID}/materials.json");
        let size = 4096_u64;
        let expected = format!("bytes */{size}");

        for header in [format!("bytes={size}-"), format!("bytes={}-", size + 1)] {
            let response = resolve(
                &root,
                &ProtocolRequest {
                    range: Some(&header),
                    ..get(&path)
                },
            );
            assert_eq!(response.status, 416, "{header} should be 416");
            assert_eq!(
                response.header("content-range"),
                Some(expected.as_str())
            );
            assert!(response.body.is_empty());
        }

        // `bytes=0-` 其实是合法的「从 0 到结尾」。
        let response = resolve(
            &root,
            &ProtocolRequest {
                range: Some("bytes=0-"),
                ..get(&path)
            },
        );
        assert_eq!(response.status, 206);
        assert_eq!(
            response.header("content-range"),
            Some("bytes 0-4095/4096")
        );
    }

    #[test]
    fn serves_the_whole_file_without_a_range() {
        let (_temp, root) = fixture();

        let response =
            resolve(&root, &get(&format!("/{ID}/materials.json")));

        assert_eq!(response.status, 200);
        assert_eq!(response.body.len(), 4096);
        assert_eq!(response.header("content-length"), Some("4096"));
        assert!(response.header("content-range").is_none());
    }

    #[test]
    fn supports_suffix_ranges() {
        let (_temp, root) = fixture();
        let path = format!("/{ID}/materials.json");

        let response = resolve(
            &root,
            &ProtocolRequest {
                range: Some("bytes=-16"),
                ..get(&path)
            },
        );

        assert_eq!(response.status, 206);
        assert_eq!(
            response.header("content-range"),
            Some("bytes 4080-4095/4096")
        );
        assert_eq!(response.body.len(), 16);
    }

    #[test]
    fn ignores_ranges_it_does_not_understand() {
        let (_temp, root) = fixture();
        let path = format!("/{ID}/materials.json");

        for header in [
            "bytes=0-1,5-6",
            "items=0-10",
            "bytes=abc-def",
            "bytes=-",
        ] {
            let response = resolve(
                &root,
                &ProtocolRequest {
                    range: Some(header),
                    ..get(&path)
                },
            );
            assert_eq!(response.status, 200, "{header} should fall back to 200");
            assert_eq!(response.body.len(), 4096);
        }
    }

    #[test]
    fn head_requests_get_headers_without_a_body() {
        let (_temp, root) = fixture();

        let response = resolve(
            &root,
            &ProtocolRequest {
                method: "HEAD",
                ..get(&format!("/{ID}/materials.json"))
            },
        );

        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-length"), Some("4096"));
        assert!(response.body.is_empty());
    }

    #[test]
    fn other_methods_are_rejected() {
        let (_temp, root) = fixture();

        let response = resolve(
            &root,
            &ProtocolRequest {
                method: "POST",
                ..get(&format!("/{ID}/project.json"))
            },
        );

        assert_eq!(response.status, 405);
    }

    #[test]
    fn cors_preflight_is_answered_so_video_can_use_crossorigin() {
        // `<video crossorigin="anonymous">` 的 Range 请求会先发 OPTIONS 预检；
        // 这里回 405 的话视频直接加载失败、壁纸退回海报（实测「囍」就是这样）。
        let (_temp, root) = fixture();

        let response = resolve(
            &root,
            &ProtocolRequest {
                method: "OPTIONS",
                ..get(&format!("/{ID}/big.mp4"))
            },
        );

        assert_eq!(response.status, 204);
        assert!(response.body.is_empty());
    }

    #[test]
    fn maps_mime_types_by_extension() {
        assert_eq!(mime_for("project.json"), "application/json; charset=utf-8");
        assert_eq!(mime_for("assets/PREVIEW.JPG"), "image/jpeg");
        assert_eq!(mime_for("wallpaper.mp4"), "video/mp4");
        assert_eq!(mime_for("a/b/c.png"), "image/png");
        assert_eq!(mime_for("materials/x.tex"), "application/octet-stream");
        assert_eq!(mime_for("scene.pkg"), "application/octet-stream");
        assert_eq!(mime_for("no-extension"), "application/octet-stream");
        assert_eq!(mime_for("shader.vert"), "text/plain; charset=utf-8");
    }

    #[test]
    fn percent_decode_keeps_plus_and_rejects_bad_escapes() {
        assert_eq!(percent_decode("/a%20b").as_deref(), Some("/a b"));
        assert_eq!(percent_decode("/a+b").as_deref(), Some("/a+b"));
        assert_eq!(percent_decode("%2e%2e%2f").as_deref(), Some("../"));
        assert!(percent_decode("%zz").is_none());
        assert!(percent_decode("%2").is_none());
        assert!(percent_decode("%ff").is_none(), "非法 UTF-8 直接拒");
    }

    #[test]
    fn a_hundred_megabyte_file_serves_a_kilobyte_without_growing_memory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("wallpapers");
        let package = root.join(PACKAGES_DIR_NAME).join(ID);
        fs::create_dir_all(&package).expect("mkdir");
        let video = package.join("big.mp4");
        {
            let file = File::create(&video).expect("create");
            // 稀疏文件：占满 100 MB 的地址空间，不真的写盘。
            file.set_len(100 * 1024 * 1024).expect("set_len");
        }
        let path = format!("/{ID}/big.mp4");

        #[cfg(windows)]
        let before = memory::private_working_set()
            .or_else(memory::working_set)
            .expect("read process memory");

        let response = resolve(
            &root,
            &ProtocolRequest {
                range: Some("bytes=0-1023"),
                ..get(&path)
            },
        );

        #[cfg(windows)]
        {
            let after = memory::private_working_set()
                .or_else(memory::working_set)
                .expect("read process memory");
            let delta = after.saturating_sub(before);
            println!(
                "私有工作集：{} KB → {} KB，增量 {} KB",
                before / 1024,
                after / 1024,
                delta / 1024
            );
            assert!(
                delta < 8 * 1024 * 1024,
                "增量 {} 字节，超过 8 MB：说明整文件被读进了内存",
                delta
            );
        }

        assert_eq!(response.status, 206);
        assert_eq!(response.body.len(), 1024);
        assert_eq!(
            response.header("content-range"),
            Some("bytes 0-1023/104857600")
        );
    }

    #[test]
    fn read_slice_never_reads_past_the_end() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("small.bin");
        fs::write(&path, b"0123456789").expect("write");

        assert_eq!(read_slice(&path, 0, 10), Some(b"0123456789".to_vec()));
        assert_eq!(read_slice(&path, 5, 5), Some(b"56789".to_vec()));
        assert_eq!(read_slice(&path, 0, 0), Some(Vec::new()));
        // 越过文件末尾必须失败，而不是返回一段短的。
        assert_eq!(read_slice(&path, 5, 10), None);
        assert_eq!(read_slice(&path, 20, 1), None);
        assert_eq!(read_slice(&temp.path().join("missing"), 0, 1), None);
    }

    #[test]
    fn the_single_response_cap_is_half_a_gigabyte() {
        assert_eq!(MAX_BODY_BYTES, 512 * 1024 * 1024);
    }

    // -- `/steam/<id>/<rel>` 旁路 -----------------------------------------

    /// 造一个「Steam 工坊目录」并返回一个只认 `1001` 的目录表。
    fn steam_fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("temp dir");
        let entry = temp.path().join("431960").join("1001");
        fs::create_dir_all(&entry).expect("mkdir");
        fs::write(entry.join("project.json"), b"{\"title\":\"river\"}")
            .expect("write");
        fs::write(entry.join("preview.gif"), b"GIF89a-preview")
            .expect("write");
        // 工坊目录的上一层放一个「不该被读到」的文件。
        fs::write(temp.path().join("431960").join("secret.txt"), b"secret")
            .expect("write");
        (temp, entry)
    }

    fn with_steam(
        root: &Path,
        path: &str,
        entry: &Path,
    ) -> ProtocolResponse {
        let entry = entry.to_path_buf();
        let lookup = move |id: &str| {
            (id == "1001").then(|| entry.clone())
        };
        resolve_with_steam(root, &get(path), &lookup)
    }

    #[test]
    fn steam_route_serves_a_preview_with_a_short_cache() {
        let (temp, entry) = steam_fixture();
        let root = temp.path().join("wallpapers");

        let response = with_steam(&root, "/steam/1001/preview.gif", &entry);

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"GIF89a-preview");
        assert_eq!(response.header("content-type"), Some("image/gif"));
        // 工坊内容会被 Steam 原地更新 ⇒ 不能标 immutable。
        assert_eq!(response.header("cache-control"), Some(STEAM_CACHE_CONTROL));
        assert!(response.header("etag").is_some());
        assert_eq!(response.header("accept-ranges"), Some("bytes"));
    }

    #[test]
    fn steam_route_supports_range_and_etag() {
        let (temp, entry) = steam_fixture();
        let root = temp.path().join("wallpapers");
        let path = "/steam/1001/preview.gif";

        let range = resolve_with_steam(
            &root,
            &ProtocolRequest {
                range: Some("bytes=0-3"),
                ..get(path)
            },
            &|_| Some(entry.clone()),
        );
        assert_eq!(range.status, 206);
        assert_eq!(range.body, b"GIF8");
        assert_eq!(range.header("content-range"), Some("bytes 0-3/14"));

        let first = with_steam(&root, path, &entry);
        let etag = first.header("etag").expect("etag").to_string();
        let second = resolve_with_steam(
            &root,
            &ProtocolRequest {
                if_none_match: Some(&etag),
                ..get(path)
            },
            &|_| Some(entry.clone()),
        );
        assert_eq!(second.status, 304);
        assert_eq!(second.header("cache-control"), Some(STEAM_CACHE_CONTROL));
    }

    #[test]
    fn steam_route_rejects_traversal_foreign_ids_and_unknown_entries() {
        let (temp, entry) = steam_fixture();
        let root = temp.path().join("wallpapers");

        for path in [
            "/steam/1001/../secret.txt",
            "/steam/1001/..%2fsecret.txt",
            "/steam/1001/%2e%2e%2fsecret.txt",
            "/steam/1001/..\\secret.txt",
            "/steam/1001/C:/Windows/win.ini",
        ] {
            let response = with_steam(&root, path, &entry);
            assert!(
                matches!(response.status, 400 | 403 | 404),
                "{path} 不该被伺服，得到 {}",
                response.status
            );
            assert!(!response.body.starts_with(b"secret"), "{path} 泄漏了文件");
        }

        // id 形状不对 ⇒ 403（不是 404：路径形状本身不被接受）。
        for path in ["/steam/../x", "/steam/abc/preview.gif", "/steam/١٠/preview.gif"]
        {
            assert_eq!(with_steam(&root, path, &entry).status, 403, "{path}");
        }

        // 形状对但没扫到 ⇒ 404。
        assert_eq!(
            with_steam(&root, "/steam/9999/preview.gif", &entry).status,
            404
        );

        // 缺 id / 缺 rel ⇒ 400。
        assert_eq!(with_steam(&root, "/steam/1001", &entry).status, 400);
        assert_eq!(with_steam(&root, "/steam/1001/", &entry).status, 400);
    }

    #[test]
    fn steam_route_is_decided_after_percent_decoding() {
        let (temp, entry) = steam_fixture();
        let root = temp.path().join("wallpapers");

        // `%73` = `s`：解码后才是 `/steam/…`，也必须走旁路而不是当成包 id。
        let response = with_steam(&root, "/%73team/1001/preview.gif", &entry);
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"GIF89a-preview");
    }

    #[test]
    fn the_plain_entry_point_never_serves_steam_paths() {
        let (temp, _entry) = steam_fixture();
        let root = temp.path().join("wallpapers");

        // `resolve` 不带目录表 ⇒ 旁路一律 404，不会被误当成包 id。
        assert_eq!(resolve(&root, &get("/steam/1001/preview.gif")).status, 404);
    }

    #[test]
    fn steam_route_refuses_methods_other_than_get_and_head() {
        let (temp, entry) = steam_fixture();
        let root = temp.path().join("wallpapers");

        let response = resolve_with_steam(
            &root,
            &ProtocolRequest {
                method: "POST",
                ..get("/steam/1001/preview.gif")
            },
            &|_| Some(entry.clone()),
        );
        assert_eq!(response.status, 405);
    }
}
