//! Minecraft 服务器状态查询（Server List Ping / SLP，协议 1.7+）。
//!
//! 为什么要自己写：WebView 里发不出裸 TCP，`fetch` 只能打 HTTP。要拿「延迟 / 在线人数 /
//! MOTD / 版本」就只能走 Java 版的 SLP 握手，而这套东西必须在 Rust 侧做。
//!
//! 协议（最小集，够用且不会因为版本变化而坏）：
//! ```text
//! C→S  Handshake  0x00: varint proto(-1 表示"只问状态"), string host, u16 port, varint 1
//! C→S  Request    0x00: 空
//! S→C  Response   varint 长度 + varint 0x00 + string(JSON)
//! ```
//! 只读一次响应就够 —— favicon 之类的大字段在 JSON 里，解析时只取需要的几个键。
//!
//! ⚠ 这个文件刻意**不引第三方 JSON 库**：只取了几个固定键，用最小扫描即可，
//! 少一个依赖就少一次编译时间（这台机编 Rust 是以分钟计的）。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::Serialize;

/// 单次查询的总超时。局域网的服务器 200ms 就有响应，外网 3s 也够了；
/// 再长会让 UI 一直转圈。
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub online: bool,
    /// 往返时延（毫秒）。连不上时为 `None`。
    pub latency_ms: Option<u64>,
    pub version: Option<String>,
    pub protocol: Option<i32>,
    pub players_online: Option<i32>,
    pub players_max: Option<i32>,
    /// MOTD 的纯文本形式（§ 颜色码已剥掉，换行用 `\n`）。
    pub motd: Option<String>,
    /// 服务器图标：SLP 的 `favicon` 字段，本身就是 `data:image/png;base64,…`，前端直接塞 `<img src>`。
    pub favicon: Option<String>,
    /// `true` 表示服务器在，但状态查询被挡（部分服关了 query / 有防火墙），
    /// 此时只有 `latency_ms` 是真的。
    pub status_blocked: bool,
    pub error: Option<String>,
}

impl ServerStatus {
    fn offline(error: impl Into<String>) -> Self {
        Self {
            online: false,
            latency_ms: None,
            version: None,
            protocol: None,
            players_online: None,
            players_max: None,
            motd: None,
            favicon: None,
            status_blocked: false,
            error: Some(error.into()),
        }
    }
}

/* ---------------- varint ---------------- */

fn write_varint(out: &mut Vec<u8>, mut value: i32) {
    let mut buf = [0u8; 5];
    let mut len = 0;
    loop {
        let byte = (value & 0x7f) as u8;
        value = ((value as u32) >> 7) as i32;
        if value == 0 {
            buf[len] = byte;
            len += 1;
            break;
        }
        buf[len] = byte | 0x80;
        len += 1;
    }
    out.extend_from_slice(&buf[..len]);
}

fn write_string(out: &mut Vec<u8>, text: &str) {
    write_varint(out, text.len() as i32);
    out.extend_from_slice(text.as_bytes());
}

fn read_varint(stream: &mut TcpStream) -> std::io::Result<i32> {
    let mut result = 0i32;
    for shift in 0..5 {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        result |= ((byte[0] & 0x7f) as i32) << (shift * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "varint 过长",
    ))
}

/* ---------------- 解析（只取需要的键） ---------------- */

/// 从 JSON 文本里取 `"key"` 后面的标量值。够用就行：SLP 的响应结构是固定的几层，
/// 这里不做通用解析，只做「找键 → 取值」。
fn pick_str(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = json.find(&needle)?;
    let rest = &json[at + needle.len()..];
    let colon = rest.find(':')?;
    let tail = rest[colon + 1..].trim_start();
    if !tail.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut chars = tail[1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            },
            '"' => break,
            other => out.push(other),
        }
    }
    Some(out)
}

fn pick_i32(json: &str, key: &str) -> Option<i32> {
    let needle = format!("\"{key}\"");
    let at = json.find(&needle)?;
    let rest = &json[at + needle.len()..];
    let colon = rest.find(':')?;
    let tail = rest[colon + 1..].trim_start();
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

/// 剥掉 § 颜色码（`§a` `§l` …），并压掉多余空白。
fn strip_format_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
            continue;
        }
        // MOTD 常用「非断行空格」当对齐，换成普通空格
        out.push(if c == '\u{00a0}' { ' ' } else { c });
    }
    out.trim().to_string()
}

/// MOTD 可能是字符串，也可能是 `{text, extra[]}` 复合结构。这里把能看到的文本全拼起来。
fn extract_motd(json: &str) -> Option<String> {
    // 顶层 "description" 之后的那段才是 MOTD，先截出来避免误取到别的 "text"
    let at = json.find("\"description\"")?;
    let slice = &json[at..];
    let end = slice.find("\"players\"").or_else(|| slice.find("\"version\"")).unwrap_or(slice.len());
    let section = &slice[..end];

    if let Some(text) = pick_str(section, "text") {
        return Some(strip_format_codes(&text));
    }
    if let Some(text) = pick_str(section, "description") {
        return Some(strip_format_codes(&text));
    }
    None
}

/* ---------------- 主流程 ---------------- */

fn resolve(host: &str, port: u16) -> Result<std::net::SocketAddr, String> {
    let target = format!("{host}:{port}");
    target
        .to_socket_addrs()
        .map_err(|error| format!("解析失败：{error}"))?
        .next()
        .ok_or_else(|| "解析不到地址".to_string())
}

/// 查询一台服务器的状态。
///
/// `host` 可以是域名或 IP，`port` 默认 25565。SRV 记录不在这里处理 ——
/// 由调用方先解析好真实地址再传进来（Rust 侧做 SRV 要再引 DNS 库，不值得）。
pub fn query(host: &str, port: u16) -> ServerStatus {
    let addr = match resolve(host, port) {
        Ok(addr) => addr,
        Err(error) => return ServerStatus::offline(error),
    };

    let started = Instant::now();
    let mut stream = match TcpStream::connect_timeout(&addr, TIMEOUT) {
        Ok(stream) => stream,
        Err(error) => return ServerStatus::offline(format!("连不上：{error}")),
    };
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));

    // Handshake（protocol = -1：只问状态，不参与版本协商）
    let mut packet = Vec::new();
    write_varint(&mut packet, 0x00);
    write_varint(&mut packet, -1);
    write_string(&mut packet, host);
    packet.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut packet, 1);

    let mut framed = Vec::new();
    write_varint(&mut framed, packet.len() as i32);
    framed.extend_from_slice(&packet);
    // Status Request
    write_varint(&mut framed, 1);
    write_varint(&mut framed, 0x00);

    if let Err(error) = stream.write_all(&framed) {
        return ServerStatus::offline(format!("发送失败：{error}"));
    }
    let _ = stream.flush();

    // 长度 + packetId + JSON
    let length = match read_varint(&mut stream) {
        Ok(value) if value > 0 && value < 8 * 1024 * 1024 => value,
        Ok(_) => {
            return ServerStatus {
                status_blocked: true,
                online: true,
                latency_ms: Some(started.elapsed().as_millis() as u64),
                ..ServerStatus::offline("状态响应长度异常")
            }
        }
        Err(error) => {
            // 端口开着但不说协议话 —— 多半是 query 被关或有防火墙
            return ServerStatus {
                status_blocked: true,
                online: true,
                latency_ms: Some(started.elapsed().as_millis() as u64),
                ..ServerStatus::offline(format!("无状态响应：{error}"))
            };
        }
    };

    let mut body = vec![0u8; length as usize];
    if let Err(error) = stream.read_exact(&mut body) {
        return ServerStatus::offline(format!("读取失败：{error}"));
    }
    let latency = started.elapsed().as_millis() as u64;

    // body[0] 是 packetId varint（=0x00，单字节），其后是 string 长度 + JSON
    let mut cursor = 1usize;
    let mut json_len = 0usize;
    let mut shift = 0;
    while cursor < body.len() && shift < 5 {
        let byte = body[cursor];
        cursor += 1;
        json_len |= ((byte & 0x7f) as usize) << (shift * 7);
        shift += 1;
        if byte & 0x80 == 0 {
            break;
        }
    }
    if cursor + json_len > body.len() {
        return ServerStatus::offline("响应被截断");
    }
    let json = String::from_utf8_lossy(&body[cursor..cursor + json_len]).into_owned();

    ServerStatus {
        online: true,
        latency_ms: Some(latency),
        version: pick_str(&json, "name").map(|v| v.to_string()),
        protocol: pick_i32(&json, "protocol"),
        players_online: pick_i32(&json, "online"),
        players_max: pick_i32(&json, "max"),
        motd: extract_motd(&json),
        favicon: pick_str(&json, "favicon").filter(|value| value.starts_with("data:image/")),
        status_blocked: false,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_shapes() {
        let mut out = Vec::new();
        write_varint(&mut out, 25565);
        assert_eq!(out, vec![0xdd, 0xc7, 0x01]);
        let mut out2 = Vec::new();
        write_varint(&mut out2, 0);
        assert_eq!(out2, vec![0x00]);
    }

    #[test]
    fn strips_format_codes_and_nbsp() {
        assert_eq!(strip_format_codes("§aHello §lWorld\u{00a0}!"), "Hello World !");
    }

    #[test]
    fn picks_scalars_without_a_json_lib() {
        let json = r#"{"version":{"name":"1.21.1","protocol":767},"players":{"max":20,"online":3}}"#;
        assert_eq!(pick_str(json, "name").as_deref(), Some("1.21.1"));
        assert_eq!(pick_i32(json, "protocol"), Some(767));
        assert_eq!(pick_i32(json, "online"), Some(3));
        assert_eq!(pick_i32(json, "max"), Some(20));
    }

    #[test]
    fn extracts_plain_and_composite_motd() {
        let plain = r#"{"description":"A §aMinecraft §rServer","players":{"max":1}}"#;
        assert_eq!(extract_motd(plain).as_deref(), Some("A Minecraft Server"));
        let composite =
            r#"{"description":{"text":"Line1","extra":[{"text":" Line2"}]},"players":{"max":1}}"#;
        assert_eq!(extract_motd(composite).as_deref(), Some("Line1"));
    }
}

/// Tauri 命令：`invoke('server_status', { host, port })`。
///
/// 阻塞的 TCP 往返必须扔进 `spawn_blocking` —— 直接在 async 里跑会占住运行时线程，
/// 首页那种「一次刷 5 台服务器」的场景会互相拖死。
#[tauri::command]
pub async fn server_status(host: String, port: Option<u16>) -> Result<ServerStatus, String> {
    let port = port.unwrap_or(25565);
    tauri::async_runtime::spawn_blocking(move || query(&host, port))
        .await
        .map_err(|error| error.to_string())
}
