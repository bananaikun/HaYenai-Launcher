//! 房间系统：聊天 + 分享 + 成员管理。
//!
//! ## 为什么服务跑在房主本机
//!
//! 房间数据属于房主，不该经过任何第三方。房主在 EasyTier 的虚拟 IP 上开一个 HTTP 服务，
//! 成员按虚拟 IP 访问 —— 在他们眼里这就是一台局域网服务器，跟自建 MC 服务器一个体验。
//! 代价是房主关掉启动器房间就结束，这符合「房主本地」的预期。
//!
//! ## 为什么手写 HTTP 而不上框架
//!
//! 需要的只是「GET + query 参数 → JSON」，没有任何 HTTP 语义要用到。用 tokio 直接读写
//! 一个有 Content-Length 的响应，比为了 `http_body_util` 这类辅助库去动整个依赖树划算。
//! 协议上只支持 `Connection: close`，不处理 chunked —— 客户端是我们自己的，不做分块上传。

use crate::api::{Result, TheseusSerializableError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 单条消息上限。房间是聊天室不是文件柜，分享走单独字段。
const MAX_TEXT: usize = 4000;
/// 邀请码长度。32 位取自 64 字符集 ≈ 192 bit，暴力猜是不可能的。
const CODE_LEN: usize = 32;
/// 邀请码字符集。只用 URL 安全的符号，省掉一层转义 —— 它同时要进 URL 和 EasyTier 的密钥。
const CODE_CHARSET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

#[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
pub struct RoomConfig {
    pub id: String,
    pub name: String,
    /// 邀请码。同时是 EasyTier 的 network-secret。
    pub code: String,
    pub host_name: String,
    pub created_at: u64,
    #[serde(default)]
    pub settings: RoomSettings,
    /// 该房间最后一条消息的编号。消息本体在 `messages.jsonl` 里，
    /// 这个计数随 rooms.json 一起落盘，发消息时不用去扫全文算 id。
    #[serde(default)]
    pub last_message_id: u64,
}

/// 房间设置。全部随房间一起持久化，重启不丢。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
pub struct RoomSettings {
    /// 内置模块开关。键是模块 id，值是开/关 ——
    /// 模块是启动器自带的（自动欢迎、词云统计），不是第三方代码，
    /// 所以这里只存布尔值，没有加载任何外部脚本的入口。
    #[serde(default)]
    pub plugins: std::collections::HashMap<String, bool>,
    #[serde(default)]
    pub bot: Option<RoomBot>,
    #[serde(default)]
    pub qq: Option<RoomQq>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
pub struct RoomBot {
    /// 机器人在房间里显示的名字。
    pub name: String,
    /// 用哪个 AI 方案（persona + rules + 模型）。空表示还没选。
    #[serde(default)]
    pub preset_id: String,
    /// 只有消息以 `@名字` 开头才回 —— 不这么做的话，屋里聊天它会一直插嘴。
    #[serde(default = "default_true")]
    pub mention_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
pub struct RoomQq {
    /// 这版只存配置不实接，`enabled` 打开时 UI 会明说「尚未生效」。
    pub enabled: bool,
    pub group_id: String,
}

fn default_true() -> bool {
    true
}

/// 房间虚拟网段。房主固定 `.1`，成员从 `.2` 开始按加入顺序发。
const VIRTUAL_PREFIX: &str = "10.126.126.";
const HOST_VIRTUAL_IP: &str = "10.126.126.1";

/// 在已有成员里找一个没被占用的尾号。
fn next_member_octet(used: &std::collections::HashSet<u32>) -> u32 {
    for octet in 2..=254u32 {
        if !used.contains(&octet) {
            return octet;
        }
    }
    // 255 个位置都满了 —— 这种规模的「房间」早就不是聊天室了，给最后一个当兜底。
    254
}

/// 内置模块清单。前端据此渲染开关，后端据此决定行为 —— 两边读同一份定义。
pub const PLUGINS: &[(&str, &str)] = &[
    ("welcome", "自动欢迎"),
    ("wordcloud", "词云统计"),
];

/// 房间文件存放目录。分享的配置和小截图落在这里。
fn files_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>, room_id: &str) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| input_error(format!("拿不到数据目录：{error}")))?
        .join("rooms")
        .join(room_id)
        .join("files");
    std::fs::create_dir_all(&dir)
        .map_err(|error| input_error(format!("建房间目录失败：{error}")))?;
    Ok(dir)
}

/// 单文件上限。「小文件直传」只承诺配置文件那一档，不是传整合包的通道。
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
pub struct RoomMessage {
    pub id: u64,
    /// 归属房间。房间之间必须隔离 —— 否则 A 房的聊天会出现在 B 房里。
    #[serde(default)]
    pub room_id: String,
    pub from: String,
    pub kind: String,
    pub text: String,
    /// 分享类消息的附加信息（整合包名、截图路径等）。聊天消息为空。
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
pub struct RoomMember {
    #[serde(default)]
    pub room_id: String,
    pub name: String,
    pub account_kind: String,
    pub note: String,
    pub joined_at: u64,
    /// 房间内的固定虚拟地址（房主占 `.1`，成员按加入顺序 `.2` 起）。
    ///
    /// 固定下来的意义是「日志和成员列表里对得上人」，重连也不会换地址。
    /// 老数据没有这个字段，`members()` 读的时候会补一次。
    #[serde(default)]
    pub virtual_ip: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct RoomStore {
    #[serde(default)]
    rooms: Vec<RoomConfig>,
    #[serde(default)]
    messages: Vec<RoomMessage>,
    #[serde(default)]
    members: Vec<RoomMember>,
}

#[derive(Debug, Default)]
struct HostRuntime {
    /// 正在跑服务的是哪个房间。
    room_id: Option<String>,
    addr: Option<String>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

static HOST: Mutex<HostRuntime> = Mutex::new(HostRuntime {
    room_id: None,
    addr: None,
    shutdown: None,
});

/* ------------------------------------------------------------------ *
 * 存储
 * ------------------------------------------------------------------ */

fn input_error(message: impl Into<String>) -> TheseusSerializableError {
    theseus::Error::from(theseus::ErrorKind::InputError(message.into())).into()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn store_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| input_error(format!("拿不到数据目录：{error}")))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| input_error(format!("建数据目录失败：{error}")))?;
    Ok(dir.join("rooms.json"))
}

fn load<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<RoomStore> {
    let path = store_path(app)?;
    if !path.is_file() {
        return Ok(RoomStore::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| input_error(format!("读房间数据失败：{error}")))?;
    let mut store: RoomStore =
        serde_json::from_str(&text).map_err(|error| input_error(format!("房间数据损坏：{error}")))?;
    // 老数据自带 messages：读的时候顺手搬到 jsonl，之后这份 JSON 里就没有消息了。
    migrate_messages(app, &mut store)?;
    Ok(store)
}

/// 先写临时文件再改名 —— 中途断电不会留下半个 JSON，下次启动还能读。
fn save<R: tauri::Runtime>(app: &tauri::AppHandle<R>, store: &RoomStore) -> Result<()> {
    let path = store_path(app)?;
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(store)
        .map_err(|error| input_error(format!("序列化房间数据失败：{error}")))?;
    std::fs::write(&tmp, text)
        .map_err(|error| input_error(format!("写房间数据失败：{error}")))?;
    std::fs::rename(&tmp, &path)
        .map_err(|error| input_error(format!("替换房间数据失败：{error}")))?;
    Ok(())
}

/* ------------------------------------------------------------------ *
 * 邀请码
 * ------------------------------------------------------------------ */

/// 生成 32 位邀请码。
///
/// 不用 `rand`：uuid v4 已经给了 122 bit 的高质量随机，取两段字节做模运算足够，
/// 为一个邀请码再引一个随机数库不划算。
pub fn generate_code() -> String {
    let mut bytes = Vec::with_capacity(CODE_LEN);
    while bytes.len() < CODE_LEN {
        let uuid = uuid::Uuid::new_v4();
        bytes.extend_from_slice(uuid.as_bytes());
    }
    bytes
        .into_iter()
        .take(CODE_LEN)
        .map(|byte| CODE_CHARSET[(byte as usize) % CODE_CHARSET.len()] as char)
        .collect()
}

/* ------------------------------------------------------------------ *
 * 房间操作
 * ------------------------------------------------------------------ */

pub fn list<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<Vec<RoomConfig>> {
    let mut store = load(&app)?;
    store.rooms.sort_by_key(|room| std::cmp::Reverse(room.created_at));
    Ok(store.rooms)
}

pub fn create<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    name: String,
    host_name: String,
) -> Result<RoomConfig> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(input_error("房间名不能为空"));
    }
    let mut store = load(&app)?;
    let room = RoomConfig {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        code: generate_code(),
        host_name: if host_name.trim().is_empty() {
            "房主".to_string()
        } else {
            host_name.trim().to_string()
        },
        created_at: now_ms(),
        settings: RoomSettings::default(),
        last_message_id: 0,
    };
    store.rooms.push(room.clone());
    save(&app, &store)?;
    Ok(room)
}

pub fn remove<R: tauri::Runtime>(app: tauri::AppHandle<R>, room_id: &str) -> Result<()> {
    {
        let mut host = HOST.lock().map_err(|_| input_error("房间状态被占用"))?;
        if host.room_id.as_deref() == Some(room_id) {
            if let Some(shutdown) = host.shutdown.take() {
                let _ = shutdown.send(());
            }
            host.room_id = None;
            host.addr = None;
        }
    }
    let mut store = load(&app)?;
    store.rooms.retain(|room| room.id != room_id);
    // 聊天记录和成员跟着房间一起消失 —— 留着只会在下次同 id 时变成幽灵数据。
    // 消息本体在 rooms/<id>/ 下，整个目录删掉（只删这一个房间的目录）。
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_dir_all(dir.join("rooms").join(room_id));
    }
    store.messages.retain(|message| message.room_id != room_id);
    store.members.retain(|member| member.room_id != room_id);
    save(&app, &store)
}

/* ------------------------------------------------------------------ *
 * 消息存储：每房间一个 messages.jsonl，只追加
 *
 * 为什么不再塞进 rooms.json：那份文件一改就要整份重写，2000 条消息时
 * 每发一句都在重写 400 KB；而且一条消息写坏会把整个房间库连带弄坏。
 * 追加写只影响新加的那一行，坏行也能在读取时跳过。
 * ------------------------------------------------------------------ */

fn messages_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    room_id: &str,
) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| input_error(format!("拿不到数据目录：{error}")))?
        .join("rooms")
        .join(room_id);
    std::fs::create_dir_all(&dir)
        .map_err(|error| input_error(format!("建房间目录失败：{error}")))?;
    Ok(dir.join("messages.jsonl"))
}

fn read_messages<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    room_id: &str,
) -> Result<Vec<RoomMessage>> {
    let path = messages_path(app, room_id)?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| input_error(format!("读聊天记录失败：{error}")))?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        // 单行坏了只丢那一条，不让整个房间读不出来。
        .filter_map(|line| serde_json::from_str::<RoomMessage>(line).ok())
        .filter(|message| message.room_id == room_id)
        .collect())
}

fn append_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    message: &RoomMessage,
) -> Result<()> {
    use std::io::Write;
    let path = messages_path(app, &message.room_id)?;
    let line = serde_json::to_string(message)
        .map_err(|error| input_error(format!("序列化消息失败：{error}")))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| input_error(format!("打开聊天记录失败：{error}")))?;
    writeln!(file, "{line}").map_err(|error| input_error(format!("写聊天记录失败：{error}")))?;
    Ok(())
}

/// 老版本把所有消息存在 rooms.json 里。这里一次性搬到 jsonl，
/// 搬之前先留一份 `rooms.json.bak` —— 万一中途断电还能人工救回来。
fn migrate_messages<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    store: &mut RoomStore,
) -> Result<()> {
    if store.messages.is_empty() {
        return Ok(());
    }
    let path = store_path(app)?;
    let _ = std::fs::copy(&path, path.with_extension("json.bak"));

    let mut by_room: std::collections::HashMap<String, Vec<RoomMessage>> =
        std::collections::HashMap::new();
    for message in store.messages.drain(..) {
        by_room
            .entry(message.room_id.clone())
            .or_default()
            .push(message);
    }

    for (room_id, rows) in by_room {
        // 早期消息可能没有 roomId（字段是后加的）。空 id 写下去会落到
        // `rooms/messages.jsonl` 这个根目录垃圾文件里 —— 直接丢掉。
        if room_id.trim().is_empty() {
            continue;
        }
        // 已经有 jsonl 的房间不再重复写，否则每读一次就多一份历史。
        if !read_messages(app, &room_id)?.is_empty() {
            continue;
        }
        for row in &rows {
            append_message(app, row)?;
        }
        if let Some(room) = store.rooms.iter_mut().find(|room| room.id == room_id) {
            let max_id = rows.iter().map(|row| row.id).max().unwrap_or(0);
            room.last_message_id = room.last_message_id.max(max_id);
        }
    }

    save(app, store)
}

/* ------------------------------------------------------------------ *
 * 消息
 * ------------------------------------------------------------------ */

pub fn push_message<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
    from: &str,
    kind: &str,
    text: &str,
    payload: Option<serde_json::Value>,
) -> Result<RoomMessage> {
    let trimmed = text.trim();
    if trimmed.chars().count() > MAX_TEXT {
        return Err(input_error(format!("单条消息最多 {MAX_TEXT} 个字符")));
    }
    let mut store = load(&app)?;

    // 有本地房间记录就用它的计数器；成员视角（room_id 是 member-xxx）没有记录，
    // 退回去读一遍 jsonl 的尾巴 —— 那种情况消息量不会大。
    let id = match store.rooms.iter_mut().find(|room| room.id == room_id) {
        Some(room) => {
            room.last_message_id += 1;
            room.last_message_id
        }
        None => read_messages(&app, room_id)?
            .last()
            .map(|message| message.id + 1)
            .unwrap_or(1),
    };

    let message = RoomMessage {
        id,
        room_id: room_id.to_string(),
        from: from.trim().to_string(),
        kind: kind.to_string(),
        text: trimmed.to_string(),
        payload,
        ts: now_ms(),
    };

    // 先追加消息再更新元数据：中途断电最多让计数落后，下次按 jsonl 尾巴还能纠正。
    append_message(&app, &message)?;
    save(&app, &store)?;
    Ok(message)
}

pub fn messages_since<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
    since: u64,
    limit: usize,
) -> Result<Vec<RoomMessage>> {
    let limit = limit.clamp(1, 500);
    let mut rows = read_messages(&app, room_id)?
        .into_iter()
        .filter(|message| message.id > since)
        .collect::<Vec<_>>();
    if rows.len() > limit {
        rows = rows.split_off(rows.len() - limit);
    }
    Ok(rows)
}

/// 已经分配出去的尾号。
fn occupied_octets(members: &[RoomMember], room_id: &str) -> std::collections::HashSet<u32> {
    members
        .iter()
        .filter(|member| member.room_id == room_id)
        .filter_map(|member| member.virtual_ip.as_deref())
        .filter_map(|ip| ip.rsplit('.').next())
        .filter_map(|octet| octet.parse::<u32>().ok())
        .collect()
}

pub fn members<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
) -> Result<Vec<RoomMember>> {
    let mut store = load(&app)?;

    // 老房间的成员没有固定地址：这里按加入顺序补一次并落盘，
    // 否则「固定 IP」对升级前建的房间就永远不生效。
    let mut used = occupied_octets(&store.members, room_id);
    let mut patched = false;
    for member in store
        .members
        .iter_mut()
        .filter(|member| member.room_id == room_id && member.virtual_ip.is_none())
    {
        let octet = if member.account_kind == "host" {
            1
        } else {
            next_member_octet(&used)
        };
        member.virtual_ip = Some(format!("{VIRTUAL_PREFIX}{octet}"));
        used.insert(octet);
        patched = true;
    }
    if patched {
        save(&app, &store)?;
    }

    Ok(store
        .members
        .into_iter()
        .filter(|member| member.room_id == room_id)
        .collect())
}

/// 加入登记。同一账号重复加入只更新备注，不产生第二个成员 ——
/// 房间内账号唯一是主人明确要求的。
pub fn upsert_member<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
    name: String,
    account_kind: String,
    note: String,
) -> Result<Vec<RoomMember>> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(input_error("账号名不能为空"));
    }
    let mut store = load(&app)?;
    match store
        .members
        .iter_mut()
        .find(|member| member.room_id == room_id && member.name == name)
    {
        Some(existing) => {
            existing.account_kind = account_kind;
            if !note.trim().is_empty() {
                existing.note = note.trim().to_string();
            }
        }
        None => {
            // 房主拿 .1，成员按已有占用情况往后排。
            let virtual_ip = if account_kind == "host" {
                HOST_VIRTUAL_IP.to_string()
            } else {
                let used = occupied_octets(&store.members, room_id);
                format!("{VIRTUAL_PREFIX}{}", next_member_octet(&used))
            };
            store.members.push(RoomMember {
                room_id: room_id.to_string(),
                name,
                account_kind,
                note: note.trim().to_string(),
                joined_at: now_ms(),
                virtual_ip: Some(virtual_ip),
            });
        }
    }
    save(&app, &store)?;
    Ok(store.members)
}

/* ------------------------------------------------------------------ *
 * 房主服务
 * ------------------------------------------------------------------ */

/// 启动房主的房间服务。`bind` 用虚拟网卡地址时，成员才能通过 EasyTier 访问到。
pub async fn host_start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: String,
    bind: String,
    port: u16,
) -> Result<String> {
    {
        let host = HOST.lock().map_err(|_| input_error("房间状态被占用"))?;
        if host.room_id.is_some() {
            return Err(input_error("已经有房间在跑了，先关掉再开"));
        }
    }

    let store = load(&app)?;
    let room = store
        .rooms
        .iter()
        .find(|room| room.id == room_id)
        .cloned()
        .ok_or_else(|| input_error("房间不存在"))?;

    let listener = TcpListener::bind((bind.as_str(), port))
        .await
        .map_err(|error| input_error(format!("监听 {bind}:{port} 失败：{error}")))?;
    let addr = listener
        .local_addr()
        .map(|value| value.to_string())
        .unwrap_or_else(|_| format!("{bind}:{port}"));

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let expected_code = room.code.clone();
    let room_for_task = room.id.clone();
    let app_for_task = app.clone();

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, _peer)) = accepted else { continue };
                    let code = expected_code.clone();
                    let room = room_for_task.clone();
                    let app = app_for_task.clone();
                    // 每个连接一个 task：一个慢客户端不该把整个房间堵住。
                    tauri::async_runtime::spawn(async move {
                        let _ = serve_connection(stream, &code, &room, &app).await;
                    });
                }
            }
        }
    });

    let mut host = HOST.lock().map_err(|_| input_error("房间状态被占用"))?;
    host.room_id = Some(room_id);
    host.addr = Some(addr.clone());
    host.shutdown = Some(shutdown_tx);
    Ok(addr)
}

pub fn host_stop() -> Result<()> {
    let mut host = HOST.lock().map_err(|_| input_error("房间状态被占用"))?;
    if let Some(shutdown) = host.shutdown.take() {
        let _ = shutdown.send(());
    }
    host.room_id = None;
    host.addr = None;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub running: bool,
    pub room_id: Option<String>,
    pub addr: Option<String>,
}

pub fn host_info() -> HostInfo {
    match HOST.lock() {
        Ok(host) => HostInfo {
            running: host.room_id.is_some(),
            room_id: host.room_id.clone(),
            addr: host.addr.clone(),
        },
        Err(_) => HostInfo {
            running: false,
            room_id: None,
            addr: None,
        },
    }
}

struct ParsedRequest {
    method: String,
    path: String,
    query: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}

/// 读一个 HTTP 请求。
///
/// 只做我们自己的客户端会用到的那部分：`Content-Length` + 完整的 body 字节。
/// 不处理 chunked（客户端是启动器自己，不做分块上传），也不处理 keep-alive
/// （一次一连接，响应完就关）。这两个省略让代码短了一大截，且没有可用性损失。
async fn read_request(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<Option<ParsedRequest>> {
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = vec![0u8; 8192];
    let deadline = std::time::Duration::from_secs(15);

    let header_end = loop {
        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
        if buffer.len() > 64 * 1024 {
            return Ok(None);
        }
        let read = tokio::time::timeout(deadline, stream.read(&mut chunk))
            .await
            .unwrap_or(Ok(0))
            .unwrap_or(0);
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let first_line = lines.next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();

    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_FILE_BYTES {
        return Ok(None);
    }

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = tokio::time::timeout(deadline, stream.read(&mut chunk))
            .await
            .unwrap_or(Ok(0))
            .unwrap_or(0);
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
        if body.len() > MAX_FILE_BYTES {
            return Ok(None);
        }
    }
    body.truncate(content_length);

    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target.as_str(), ""),
    };

    Ok(Some(ParsedRequest {
        method,
        path: path.to_string(),
        query: parse_query(query),
        body,
    }))
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// 处理一个连接。
async fn serve_connection<R: tauri::Runtime>(
    mut stream: tokio::net::TcpStream,
    expected_code: &str,
    room_id: &str,
    app: &tauri::AppHandle<R>,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream).await? else {
        return Ok(());
    };

    // 跨源预检：前端在 tauri.localhost，服务在虚拟 IP 上，正经是两个源。
    if request.method == "OPTIONS" {
        return write_empty(&mut stream, 204).await;
    }

    let provided = request.query.get("k").map(String::as_str).unwrap_or_default();
    if provided != expected_code {
        // 虚拟网段本身已经隔离了外人，密钥是第二道门。
        return write_json(&mut stream, 403, &serde_json::json!({ "error": "bad key" })).await;
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/post") => {
            let from = request.query.get("from").cloned().unwrap_or_default();
            let text = request.query.get("text").cloned().unwrap_or_default();
            let kind = request
                .query
                .get("kind")
                .cloned()
                .unwrap_or_else(|| "text".to_string());
            let payload = request
                .query
                .get("payload")
                .and_then(|raw| serde_json::from_str(raw).ok());
            respond(
                &mut stream,
                push_message(app.clone(), room_id, &from, &kind, &text, payload),
            )
            .await
        }
        ("GET", "/pull") => {
            let since = request
                .query
                .get("since")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            respond(
                &mut stream,
                messages_since(app.clone(), room_id, since, 200),
            )
            .await
        }
        ("GET", "/members") => respond(&mut stream, members(app.clone(), room_id)).await,
        ("GET", "/settings") => {
            respond(&mut stream, room_settings(app.clone(), room_id)).await
        }
        ("POST", "/settings") => {
            let parsed: std::result::Result<RoomSettings, _> =
                serde_json::from_slice(&request.body);
            match parsed {
                Ok(settings) => {
                    respond(&mut stream, save_settings(app.clone(), room_id, settings)).await
                }
                Err(error) => {
                    write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": format!("设置格式不对：{error}") }),
                    )
                    .await
                }
            }
        }
        ("POST", "/file") => {
            let name = request.query.get("name").cloned().unwrap_or_default();
            respond(
                &mut stream,
                write_shared_file(app.clone(), room_id, &name, &request.body),
            )
            .await
        }
        ("GET", "/file") => {
            let name = request.query.get("name").cloned().unwrap_or_default();
            match read_shared_file(app, room_id, &name) {
                Ok(bytes) => write_bytes(&mut stream, 200, &bytes).await,
                Err(_) => {
                    write_json(&mut stream, 404, &serde_json::json!({ "error": "no file" })).await
                }
            }
        }
        _ => write_json(&mut stream, 404, &serde_json::json!({ "error": "no route" })).await,
    }
}

async fn respond<T: Serialize>(
    stream: &mut tokio::net::TcpStream,
    result: Result<T>,
) -> std::io::Result<()> {
    match result {
        Ok(value) => write_json(stream, 200, &value).await,
        Err(error) => {
            write_json(stream, 500, &serde_json::json!({ "error": format!("{error:?}") })).await
        }
    }
}

async fn write_empty(stream: &mut tokio::net::TcpStream, status: u16) -> std::io::Result<()> {
    let reason = if status == 204 { "No Content" } else { "OK" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn write_bytes(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn write_json(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &impl Serialize,
) -> std::io::Result<()> {
    let payload = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    // CORS 放开到 `*`：客户端是 webview 里的前端，`tauri.localhost` 与服务端口不同源。
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

/// 只解析 `a=b&c=d`，值做百分号解码。没有数组、没有重复键语义 ——
/// 比起引一个 query 解析库，这 20 行更好读，出问题也容易查。
fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => (pair, ""),
        };
        out.insert(decode_component(key), decode_component(value));
    }
    out
}

fn decode_component(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &raw[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/* ------------------------------------------------------------------ *
 * 房间设置与共享文件
 * ------------------------------------------------------------------ */

pub fn room_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
) -> Result<RoomSettings> {
    let store = load(&app)?;
    Ok(store
        .rooms
        .iter()
        .find(|room| room.id == room_id)
        .map(|room| room.settings.clone())
        .unwrap_or_default())
}

pub fn save_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
    settings: RoomSettings,
) -> Result<RoomSettings> {
    let mut store = load(&app)?;
    let room = store
        .rooms
        .iter_mut()
        .find(|room| room.id == room_id)
        .ok_or_else(|| input_error("房间不存在"))?;
    room.settings = settings;
    let saved = room.settings.clone();
    save(&app, &store)?;
    Ok(saved)
}

/// 文件名白名单。
///
/// 只放行字母数字和 `._-`，并且拒绝以 `.` 开头 —— 挡掉 `../`、绝对路径、
/// 以及 Windows 上 `CON`/`NUL` 这类保留名会造成的诡异行为。文件名来自网络，不可信。
fn sanitize_file_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 120 || trimmed.starts_with('.') {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return None;
    }
    // 去掉纯点段（`a..b` 不含 `..` 路径语义，但保险起来一起挡）。
    if trimmed.contains("..") {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn write_shared_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    room_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<serde_json::Value> {
    let Some(name) = sanitize_file_name(name) else {
        return Err(input_error("文件名不合法（只允许字母数字和 . _ -）"));
    };
    if bytes.len() > MAX_FILE_BYTES {
        return Err(input_error(format!(
            "文件超过 {} MB —— 大文件请让成员自己下载",
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    let dir = files_dir(&app, room_id)?;
    let path = dir.join(&name);
    std::fs::write(&path, bytes)
        .map_err(|error| input_error(format!("写文件失败：{error}")))?;
    Ok(serde_json::json!({ "ok": true, "name": name, "size": bytes.len() }))
}

pub fn read_shared_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    room_id: &str,
    name: &str,
) -> Result<Vec<u8>> {
    let Some(name) = sanitize_file_name(name) else {
        return Err(input_error("文件名不合法"));
    };
    let path = files_dir(app, room_id)?.join(name);
    std::fs::read(&path).map_err(|error| input_error(format!("读文件失败：{error}")))
}

/// 把本机的一个文件上传到房主的房间服务。
///
/// 走的是同一套 HTTP：先读本地文件，再 POST 到 `/file`。
/// 之所以不在 Rust 里直接调 `std::fs` 写房主目录 —— 那只有在房主本机才成立，
/// 成员必须走网络。
pub async fn upload_file_to_host(
    base: &str,
    code: &str,
    name: &str,
    bytes: Vec<u8>,
) -> Result<serde_json::Value> {
    let url = format!(
        "{base}/file?k={}&name={}",
        urlencoding::encode(code),
        urlencoding::encode(name)
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| input_error(format!("HTTP 客户端创建失败：{error}")))?;
    let response = client
        .post(&url)
        .header("content-type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .map_err(|error| input_error(format!("上传失败：{error}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(input_error(format!("房主返回 {status}：{text}")));
    }
    serde_json::from_str(&text).map_err(|error| input_error(format!("响应不是 JSON：{error}")))
}

/// 成员通过虚拟 IP 访问房主服务。
pub async fn client_post(
    base: &str,
    code: &str,
    from: &str,
    text: &str,
    kind: &str,
    payload: Option<&serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut url = format!(
        "{base}/post?k={}&from={}&text={}&kind={}",
        urlencoding::encode(code),
        urlencoding::encode(from),
        urlencoding::encode(text),
        urlencoding::encode(kind)
    );
    if let Some(value) = payload {
        url.push_str("&payload=");
        url.push_str(&urlencoding::encode(&value.to_string()));
    }
    get_json(&url).await
}

pub async fn client_pull(base: &str, code: &str, since: u64) -> Result<Vec<RoomMessage>> {
    let url = format!(
        "{base}/pull?k={}&since={since}",
        urlencoding::encode(code)
    );
    let value = get_json(&url).await?;
    serde_json::from_value(value).map_err(|error| input_error(format!("消息格式不对：{error}")))
}

/// 成员拉房主服务上的成员列表。房主的 /members 是唯一的真相来源，
/// 成员本地再存一份只会两边不一致。
pub async fn client_members(base: &str, code: &str) -> Result<Vec<RoomMember>> {
    let url = format!("{base}/members?k={}", urlencoding::encode(code));
    let value = get_json(&url).await?;
    serde_json::from_value(value)
        .map_err(|error| input_error(format!("成员格式不对：{error}")))
}

async fn get_json(url: &str) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|error| input_error(format!("HTTP 客户端创建失败：{error}")))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| input_error(format!("连不上房主：{error}")))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| input_error(format!("读取响应失败：{error}")))?;
    if !status.is_success() {
        return Err(input_error(format!("房主返回 {status}：{text}")));
    }
    serde_json::from_str(&text).map_err(|error| input_error(format!("响应不是 JSON：{error}")))
}

/* ------------------------------------------------------------------ *
 * Tauri 命令
 * ------------------------------------------------------------------ */

#[tauri::command]
pub async fn room_list(app: tauri::AppHandle) -> Result<Vec<RoomConfig>> {
    list(app)
}

#[tauri::command]
pub async fn room_create(
    app: tauri::AppHandle,
    name: String,
    host_name: String,
) -> Result<RoomConfig> {
    create(app, name, host_name)
}

#[tauri::command]
pub async fn room_remove(app: tauri::AppHandle, room_id: String) -> Result<()> {
    remove(app, &room_id)
}

#[tauri::command]
pub async fn room_messages(
    app: tauri::AppHandle,
    room_id: String,
    since: u64,
) -> Result<Vec<RoomMessage>> {
    messages_since(app, &room_id, since, 500)
}

#[tauri::command]
pub async fn room_members(
    app: tauri::AppHandle,
    room_id: String,
) -> Result<Vec<RoomMember>> {
    members(app, &room_id)
}

#[tauri::command]
pub async fn room_join(
    app: tauri::AppHandle,
    room_id: String,
    name: String,
    account_kind: String,
    note: String,
) -> Result<Vec<RoomMember>> {
    upsert_member(app, &room_id, name, account_kind, note)
}

#[tauri::command]
pub async fn room_host_start(
    app: tauri::AppHandle,
    room_id: String,
    bind: String,
    port: u16,
) -> Result<String> {
    host_start(app, room_id, bind, port).await
}

#[tauri::command]
pub async fn room_host_stop() -> Result<()> {
    host_stop()
}

#[tauri::command]
pub async fn room_host_info() -> Result<HostInfo> {
    Ok(host_info())
}

#[tauri::command]
pub async fn room_send(
    app: tauri::AppHandle,
    room_id: String,
    from: String,
    text: String,
    kind: Option<String>,
    payload: Option<serde_json::Value>,
    base: Option<String>,
    code: Option<String>,
) -> Result<()> {
    let kind = kind.unwrap_or_else(|| "text".to_string());
    match (base.filter(|value| !value.trim().is_empty()), code) {
        // 成员：走房主的服务。自己直接写本地库只会在各自机器上分裂出两份聊天记录。
        (Some(base), Some(code)) => {
            client_post(&base, &code, &from, &text, &kind, payload.as_ref()).await?;
            Ok(())
        }
        _ => {
            push_message(app, &room_id, &from, &kind, &text, payload)?;
            Ok(())
        }
    }
}

#[tauri::command]
pub async fn room_pull(
    app: tauri::AppHandle,
    room_id: String,
    since: u64,
    base: Option<String>,
    code: Option<String>,
) -> Result<Vec<RoomMessage>> {
    match (base.filter(|value| !value.trim().is_empty()), code) {
        (Some(base), Some(code)) => client_pull(&base, &code, since).await,
        _ => messages_since(app, &room_id, since, 500),
    }
}

#[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub label: String,
}

/// 内置模块清单。前端拿它渲染开关，不硬编码在模板里 ——
/// 以后加模块只改 `PLUGINS` 一处。
#[tauri::command]
pub async fn room_plugins() -> Result<Vec<PluginInfo>> {
    Ok(PLUGINS
        .iter()
        .map(|(id, label)| PluginInfo {
            id: id.to_string(),
            label: label.to_string(),
        })
        .collect())
}

#[tauri::command]
pub async fn room_get_settings(
    app: tauri::AppHandle,
    room_id: String,
) -> Result<RoomSettings> {
    room_settings(app, &room_id)
}

#[tauri::command]
pub async fn room_save_settings(
    app: tauri::AppHandle,
    room_id: String,
    settings: RoomSettings,
) -> Result<RoomSettings> {
    save_settings(app, &room_id, settings)
}

/// 扫一个实例的 mods 目录，生成可分享的清单。
///
/// 分享只发清单不发 jar —— 几百 MB 的模组走房主上行会把宽带拉满，
/// 而成员本来就有自己的下载器，按清单下还能各自校验。
#[tauri::command]
pub async fn room_instance_manifest(
    instance_id: String,
) -> Result<serde_json::Value> {
    let meta = theseus::instance::get(&instance_id)
        .await?
        .ok_or_else(|| input_error("实例不存在"))?;
    let game_version = meta
        .loader_components
        .iter()
        .find(|component| {
            matches!(
                component.kind,
                theseus::state::LoaderComponentKind::Vanilla
            )
        })
        .and_then(|component| component.version.clone())
        .unwrap_or_default();
    let loaders = meta
        .loader_components
        .iter()
        .filter(|component| {
            !matches!(
                component.kind,
                theseus::state::LoaderComponentKind::Vanilla
            )
        })
        .map(|component| {
            format!(
                "{:?} {}",
                component.kind,
                component.version.clone().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();

    let mods_dir = std::path::Path::new(&meta.instance.path).join("mods");
    let mut mods = Vec::new();
    if let Ok(mut reader) = tokio::fs::read_dir(&mods_dir).await {
        while let Ok(Some(entry)) = reader.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jar") {
                continue;
            }
            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            mods.push(serde_json::json!({
                "file": name,
                // 去掉 `.jar` —— 装好之后 mods 目录里就是这么个文件名，
                // 成员比对时不用再管扩展名。
                "name": name.trim_end_matches(".jar"),
                "size": size,
            }));
        }
    }
    mods.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });

    Ok(serde_json::json!({
        "instance": meta.instance.name,
        "game_version": game_version,
        "loaders": loaders,
        "mods": mods,
        "mod_count": mods.len(),
    }))
}

#[tauri::command]
pub async fn room_remote_members(
    base: String,
    code: String,
) -> Result<Vec<RoomMember>> {
    client_members(&base, &code).await
}

/// 成员把本地文件推到房主的房间服务。房主自己分享时前端会直接走 `room_share_file`。
#[tauri::command]
pub async fn room_upload_file(
    base: String,
    code: String,
    name: String,
    bytes: Vec<u8>,
) -> Result<serde_json::Value> {
    upload_file_to_host(&base, &code, &name, bytes).await
}

/// 房主分享本机的一个文件：复制进房间目录，然后发一条 `file` 消息。
///
/// 复制而不是记原路径 —— 分享出去的东西一旦被人挪了位置，成员那边就 404，
/// 而且原文件可能在别的盘上、以后还会被删。
#[tauri::command]
pub async fn room_share_file(
    app: tauri::AppHandle,
    room_id: String,
    path: String,
    from: String,
    base: Option<String>,
    code: Option<String>,
) -> Result<RoomMessage> {
    let source = std::path::Path::new(&path);
    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| input_error("拿不到文件名"))?;
    let bytes = std::fs::read(source)
        .map_err(|error| input_error(format!("读文件失败：{error}")))?;
    let size = bytes.len();
    // 成员：推给房主，由房主落盘并发消息 —— 自己写本地副本等于只有自己看得到。
    if let (Some(base), Some(code)) = (
        base.filter(|value| !value.trim().is_empty()),
        code.filter(|value| !value.trim().is_empty()),
    ) {
        upload_file_to_host(&base, &code, name, bytes.clone()).await?;
        client_post(
            &base,
            &code,
            &from,
            "file",
            name,
            Some(&serde_json::json!({ "name": name, "size": size })),
        )
        .await?;
        return Ok(RoomMessage {
            id: 0,
            room_id: room_id.clone(),
            from,
            kind: "file".to_string(),
            text: name.to_string(),
            payload: Some(serde_json::json!({ "name": name, "size": size })),
            ts: now_ms(),
        });
    }
    if size > MAX_FILE_BYTES {
        return Err(input_error(format!(
            "文件超过 {} MB —— 大文件请让成员自己下载",
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    write_shared_file(app.clone(), &room_id, name, &bytes)?;
    push_message(
        app,
        &room_id,
        &from,
        "file",
        name,
        Some(serde_json::json!({ "name": name, "size": size })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn code_is_url_safe_and_long_enough() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code
            .chars()
            .all(|c| CODE_CHARSET.contains(&(c as u8))));
        // URL 里不能出现需要转义的字符 —— 它同时要进 EasyTier 的密钥和 query。
        assert!(!code.contains('&') && !code.contains('=') && !code.contains('?'));
    }

    #[test]
    fn codes_do_not_repeat() {
        let mut seen = HashSet::new();
        for _ in 0..64 {
            assert!(seen.insert(generate_code()), "邀请码不该重复");
        }
    }

    #[test]
    fn query_parses_and_decodes() {
        let params = parse_query("k=abc&from=%E9%98%BF%E7%BD%97%E5%A8%9C&text=hi+there");
        assert_eq!(params.get("k").unwrap(), "abc");
        assert_eq!(params.get("from").unwrap(), "阿罗娜");
        assert_eq!(params.get("text").unwrap(), "hi there");
    }

    #[test]
    fn query_without_value_is_empty_string() {
        let params = parse_query("flag");
        assert_eq!(params.get("flag").unwrap(), "");
    }

    #[test]
    fn malformed_percent_does_not_panic() {
        // 半个 `%` 不能让服务崩 —— 它来自网络，内容不可信。
        let params = parse_query("text=100%");
        assert_eq!(params.get("text").unwrap(), "100%");
    }
}
