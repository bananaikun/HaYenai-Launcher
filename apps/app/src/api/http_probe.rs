//! 通用 HTTP 探活：只回答「这个地址回了什么状态码」。
//!
//! 为什么不在前端用 `fetch` / `@tauri-apps/plugin-http`：
//! - 浏览器的 `fetch(mode:'no-cors')` 拿到的是 opaque response，**404/500 也算成功**
//!   —— 拿它判在线会把挂掉的服务显示成绿的；
//! - `plugin-http` 需要在 capabilities 里逐条放行 URL，而这里的目标地址是用户自己填的，
//!   不可能预先列全；给 `http://*` 开全量又等于把整个网段暴露给 WebView。
//!
//! 所以由 Rust 发一次 GET，前端只看 `ok`（2xx）和 `status`。

use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpProbe {
    /// HTTP 状态码。连不上时不会有这个字段（直接返回 Err）。
    pub status: u16,
    /// 2xx 才算通。
    pub ok: bool,
}

#[tauri::command]
pub async fn http_probe(url: String, timeout_ms: Option<u64>) -> Result<HttpProbe, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(4000).clamp(500, 15_000));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        // 只关心状态码：不跟随重定向会得到一个 3xx，对我们没用
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();

    Ok(HttpProbe {
        status: status.as_u16(),
        ok: status.is_success(),
    })
}
