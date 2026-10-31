//! 网易云接口转发（只为「扫码登录」这一条路存在）
//!
//! 为什么要在 Rust 侧发请求，而不是用 `@tauri-apps/plugin-http`：
//! 网易云的扫码协议依赖**会话 cookie** —— 签发 unikey 那次请求的 `Set-Cookie`
//! 必须原样带进轮询，否则手机扫码确认后服务端会以 `8821` 拒掉整个会话
//! （实测：用 plugin-http 时响应头里根本看不到 `set-cookie`，Tauri 不把它交给 JS）。
//! `reqwest` 在 Rust 侧能读到 `SET_COOKIE`，于是这一条路才成立。
//!
//! 安全边界：只允许 `https://music.163.com/api/...`，且只发 GET ——
//! 这个命令不做成通用代理，避免变成一个能被页面脚本拿去的任意 HTTP 出口。

use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;

const BASE: &str = "https://music.163.com";

#[derive(Serialize)]
pub struct NeteaseResponse {
    status: u16,
    body: String,
    /// 原始 `Set-Cookie` 行（多条分开给，前端自己拼；不同属性的合并规则很脏）
    set_cookie: Vec<String>,
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            // 网易云的接口普遍 302 到登录页再跳回，保留有限跳转
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default()
    })
}

/// `headers` 用 `(name, value)` 数组而不是 HashMap：同名头（比如多个 Cookie）要能都传。
#[tauri::command]
pub async fn netease_request(
    path: String,
    headers: Vec<(String, String)>,
) -> Result<NeteaseResponse, String> {
    if !path.starts_with("/api/") {
        return Err("netease_request 只允许 /api/ 开头的路径".to_string());
    }

    let url = format!("{BASE}{path}");
    let mut request = client().get(&url);
    for (name, value) in headers {
        request = request.header(name, value);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("请求网易云失败：{error}"))?;

    let status = response.status().as_u16();
    let set_cookie = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取网易云响应失败：{error}"))?;

    Ok(NeteaseResponse {
        status,
        body,
        set_cookie,
    })
}
