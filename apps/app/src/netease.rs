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

/* ---------------- 官网登录（拿 Cookie） ----------------
   为什么改成「拉官网的登录页」而不是自己调扫码接口：
   网易对二维码登录接口（/api/login/qrcode/…）风控很紧 —— 我们用 reqwest 虽然能拿到 Set-Cookie 了，
   但手机确认后仍可能被 8821 拒。官网登录页是**官方自己在维护的正常入口**，
   扫的就是它自己的码，走的是它自己的会话，不存在「模拟客户端」这一层。
   我们只做两件事：开一个子窗口让用户登录，再把该站点的 cookie 读出来（MUSIC_U 是 HttpOnly，
   JS 读不到，必须走 WebView 的 cookie 存储）。 */

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const LOGIN_LABEL: &str = "netease-login";
/// 官网登录页（带 targetUrl 会直接停在扫码登录那一屏，少点一次）
const LOGIN_URL: &str = "https://music.163.com/login?targetUrl=/discover";
/// 读 cookie 的站点根：MUSIC_U 的作用域是 music.163.com
const COOKIE_SITE: &str = "https://music.163.com/";

/// 打开（或聚焦）官网登录窗口。
#[tauri::command]
pub async fn netease_open_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(LOGIN_LABEL) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    let url = LOGIN_URL
        .parse()
        .map_err(|error| format!("登录页地址无效：{error}"))?;

    WebviewWindowBuilder::new(&app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("网易云登录")
        .inner_size(1000.0, 720.0)
        .min_inner_size(720.0, 560.0)
        .center()
        .build()
        .map_err(|error| format!("打开登录窗口失败：{error}"))?;

    Ok(())
}

/// 读 `music.163.com` 的 cookie（`k=v; k=v`）。没登录过就返回 `None`。
///
/// ⚠ 必须回到**主线程**执行：Windows 下 `cookies_for_url` 走 WebView2 的 COM 接口，
/// 在命令自己的线程里直接调会**死锁**（实测：invoke 永不返回，窗口却还活着）。
/// 所以这里 post 到主线程 + 通道回收，并加超时兜底，避免再出现"点了没反应"。
#[tauri::command]
pub async fn netease_read_cookie(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Option<String>, String>>();
    let handle = app.clone();

    app.run_on_main_thread(move || {
        let answer = (|| -> Result<Option<String>, String> {
            let url = COOKIE_SITE
                .parse()
                .map_err(|error| format!("站点地址无效：{error}"))?;

            let webview = handle
                .get_webview_window(LOGIN_LABEL)
                .or_else(|| handle.get_webview_window("main"))
                .ok_or_else(|| "找不到可用窗口".to_string())?;

            let cookies = webview
                .cookies_for_url(url)
                .map_err(|error| format!("读取 cookie 失败：{error}"))?;

            let joined = cookies
                .iter()
                .filter(|cookie| !cookie.name().is_empty())
                .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
                .collect::<Vec<_>>()
                .join("; ");

            Ok(if joined.is_empty() { None } else { Some(joined) })
        })();
        let _ = tx.send(answer);
    })
    .map_err(|error| format!("派发到主线程失败：{error}"))?;

    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(answer) => answer,
        Err(_) => Err("读取 cookie 超时（5s）".to_string()),
    }
}

/// 登录成功 / 用户取消后关掉登录窗口。
#[tauri::command]
pub fn netease_close_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LOGIN_LABEL) {
        window
            .close()
            .map_err(|error| format!("关闭登录窗口失败：{error}"))?;
    }
    Ok(())
}
