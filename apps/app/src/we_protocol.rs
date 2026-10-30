//! `hayenai-we` 取流协议的 Tauri 适配层。
//!
//! 逻辑全在 [`theseus::wallpaper::protocol`] 里（可在 `theseus` 里跑单测）；
//! 这里只做三件事：认域名、拆请求头、把结果包成 `http::Response<Vec<u8>>`。
//!
//! Windows 上自定义协议映射成 `http://hayenai-we.localhost/<id>/<rel>`，
//! macOS / Linux 上是 `hayenai-we://localhost/<id>/<rel>`，两种形态都收。
//!
//! 两条路由共用这个协议：
//!
//! * `/<壁纸 id>/<rel>` —— 已导入的包（`wallpapers/packages/<id>/`）；
//! * `/steam/<工坊 id>/<rel>` —— **只读**映射到 Steam 库里的工坊目录，
//!   查表用 [`theseus::wallpaper::steam::remembered_dir`]（只认本次扫描结果）。
//!   前端因此可以拿 `preview.gif` 直接当缩略图，不必先复制 1.8 GB。

use std::path::{Path, PathBuf};

use tauri::http::{Request, Response, StatusCode};
use tauri::{Manager, Runtime};

use theseus::wallpaper::protocol::{
    self, ProtocolRequest, ProtocolResponse,
};

/// 协议名，注册时用。
pub const SCHEME: &str = protocol::SCHEME;
/// 域名（Windows 形态）。
pub const HOST: &str = protocol::HOST;
/// macOS / Linux 形态下 WebView 给出的域名。
const LOCALHOST: &str = "localhost";

/// 解析 `wallpapers/` 根目录。
///
/// 走 `DirectoryInfo::initial_settings_dir_path`，这样 `THESEUS_CONFIG_DIR`
/// （便携模式）与 `AXOLOTL_DATA_DIR_SUFFIX`（本地 / CI 构建）都自动生效，
/// 不会和设置系统各算一套。
pub fn root_for<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    let identifier = app.config().identifier.clone();
    theseus::DirectoryInfo::initial_settings_dir_path(&identifier)
        .map(|dir| theseus::wallpaper::wallpapers_root(&dir))
}

/// 请求落在一个本协议认识的域名上吗。
fn host_is_ours(host: Option<&str>) -> bool {
    matches!(host, Some(HOST) | Some(LOCALHOST))
}

/// 处理一次请求。
///
/// 路由判断在 [`protocol::resolve_with_steam`] 里（在百分号解码之后做，
/// `/%73team/…` 也走旁路），这里只把 Steam 目录表接进去。
pub fn handle(request: &Request<Vec<u8>>, root: &Path) -> Response<Vec<u8>> {
    if !host_is_ours(request.uri().host()) {
        return empty(StatusCode::FORBIDDEN);
    }

    let method = request.method().as_str();

    // CORS 预检。上面那套头只加在真实响应上，预检这一步必须先放行，
    // 否则带 Range 的 fetch 连请求都发不出来。
    if method == "OPTIONS" {
        return into_response(ProtocolResponse {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        });
    }

    let range = header_value(request, "range");
    let if_none_match = header_value(request, "if-none-match");

    let outcome = protocol::resolve_with_steam(
        root,
        &ProtocolRequest {
            method,
            path: request.uri().path(),
            range: range.as_deref(),
            if_none_match: if_none_match.as_deref(),
        },
        // 只认「本进程最近一次扫描真正看到过的目录」：前端给不出任意路径。
        &theseus::wallpaper::steam::remembered_dir,
    );

    into_response(outcome)
}

/// 根目录还没解析出来时的兜底响应（理论上只在初始化失败时出现）。
pub fn unavailable() -> Response<Vec<u8>> {
    empty(StatusCode::SERVICE_UNAVAILABLE)
}

fn header_value(
    request: &Request<Vec<u8>>,
    name: &'static str,
) -> Option<String> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// 跨源头：这套头是壁纸能不能渲染的**生死线**。
///
/// webwallgl 的 `httpSource` 用 `fetch()` 取 `project.json` / `scene.json` / 贴图，
/// 而浏览器对自定义协议同样走 CORS：响应里没有 `Access-Control-Allow-Origin`，
/// fetch 就一律抛 `Failed to fetch`（跟路径存不存在无关）。
/// 于是引擎一个字节都拿不到 → mount 失败 → 界面回退 `preview.jpg`，
/// 看起来就是「壁纸一直只显示缩略图」。（`<img>` 能显示只是因为图片请求不带 CORS。）
const CORS_ALLOW_ORIGIN: &str = "access-control-allow-origin";
const CORS_ALLOW_HEADERS: &str = "access-control-allow-headers";
const CORS_ALLOW_METHODS: &str = "access-control-allow-methods";
const CORS_EXPOSE_HEADERS: &str = "access-control-expose-headers";

fn into_response(outcome: ProtocolResponse) -> Response<Vec<u8>> {
    let status = StatusCode::from_u16(outcome.status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    for (name, value) in outcome.headers {
        builder = builder.header(name, value);
    }
    // Range 是引擎断点取 pkg 用的；Content-Length/Content-Range 要暴露出去，
    // 否则前端读不到长度，进度与切片都会失效。
    builder = builder
        .header(CORS_ALLOW_ORIGIN, "*")
        .header(CORS_ALLOW_HEADERS, "range, if-none-match, content-type")
        .header(CORS_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(CORS_EXPOSE_HEADERS, "content-length, content-range, accept-ranges, etag");
    builder.body(outcome.body).unwrap_or_else(|error| {
        tracing::warn!(%error, "wallpaper 取流响应构造失败");
        empty(StatusCode::INTERNAL_SERVER_ERROR)
    })
}

fn empty(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .body(Vec::new())
        .expect("failed to build wallpaper protocol response")
}
