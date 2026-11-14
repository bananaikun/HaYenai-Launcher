//! P2P 组网：用 EasyTier 把房间成员放进同一个虚拟局域网。
//!
//! ## 分工
//!
//! - **MC 游戏联机**由 Terracotta 负责（见 `terracotta.rs`），它只隧道一个 TCP 端口，
//!   并且能让 MC 客户端刷到「局域网世界」。
//! - **房间服务**（聊天、分享、语音模组的 UDP 端口）需要一条**通用**的网络通路。
//!   Terracotta 给不了，所以这里用 EasyTier 建虚拟网卡：房主在虚拟 IP 上跑 HTTP/WS
//!   服务和语音端口，其他成员按 IP 直连，跟同一个局域网里没区别。
//!
//! ## 为什么是子进程而不是嵌 crate
//!
//! EasyTier 是 LGPL-3.0。静态链接进 `HaYenai.exe` 会触发「必须允许用户替换该库」的义务，
//! 对一个不分发目标文件的安装包来说很难满足。跑独立进程不构成衍生作品，顺带还免疫版本冲突。
//!
//! ## 提权只在必要的一侧发生
//!
//! 建 TUN 网卡在 Windows 上必须管理员权限。房主必须建（房间服务要绑在虚拟 IP 上），
//! 成员也必须建（要拿到同网段地址才能访问房主的服务）—— 两侧都要，这是这个方案的代价。

use crate::api::{Result, TheseusSerializableError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{Manager, Runtime};

/// RPC 端口。避开 EasyTier 默认的 15888，免得和用户自己装的 EasyTier 抢。
const RPC_PORT: u16 = 15888;

#[derive(Debug)]
struct Running {
    child: Option<Child>,
    /// 提权启动的进程拿不到句柄，只能记着「有这么个东西在跑」，停止时再提权去 kill。
    elevated: bool,
    /// 记下这次用的配置：`start` 要拿它做幂等判断，守卫重启也要靠它。
    config: LanConfig,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// 断线自动重启最多几次。超过就停手 —— 一直被防火墙拦的情况下
/// 不停重拉只会刷失败日志，不如把错误摆到界面上让人处理。
const MAX_RESTARTS: u32 = 3;

#[derive(Clone, Debug, Default)]
struct GuardState {
    restarts: u32,
    /// 已放弃自动重连，等用户手动点。
    gave_up: bool,
}

static GUARD_STATE: Mutex<GuardState> = Mutex::new(GuardState {
    restarts: 0,
    gave_up: false,
});

/// 守卫线程只允许存在一份，免得每次 start 都叠一个监控循环。
static GUARD_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
pub struct LanConfig {
    /// 网络名。房间 ID 直接拿来用。
    pub network_name: String,
    /// 网络密钥 = 邀请码。相同 name + secret 才是同一个网络。
    pub network_secret: String,
    /// 中继 / 对等节点地址，如 `tcp://[240e:...]:11010`。
    #[serde(default)]
    pub peers: Vec<String>,
    /// 房主要导出的本机网段（让成员能访问房主 LAN 上的服务）；留空则自动取 /24。
    /// 只有 `role = "host"` 时生效。
    #[serde(default)]
    pub proxy_cidr: Option<String>,
    /// 指定虚拟 IP；留空交给 DHCP。
    #[serde(default)]
    pub virtual_ip: Option<String>,
    /// `host` 房主（建 TUN + 导出本机网段） / `member` 成员（建 TUN 拿地址）
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "member".to_string()
}

#[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
pub struct LanEnv {
    pub local_ips: Vec<String>,
    pub suggested_cidr: Option<String>,
    pub is_admin: bool,
    pub missing_files: Vec<String>,
    pub core_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
pub struct LanStatus {
    pub running: bool,
    pub elevated: bool,
    /// 是否处于「常驻托管」——退房间不停，只有主动 stop 或退出应用才停。
    pub resident: bool,
    /// 自动重连已用次数。
    pub restarts: u32,
    /// 自动重连已放弃（到上限或提权失败）。
    pub gave_up: bool,
    pub virtual_ip: Option<String>,
    pub hostname: Option<String>,
    pub public_ip: Option<String>,
    pub nat_type: Option<String>,
    pub peers: Vec<PeerRow>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
pub struct PeerRow {
    pub ipv4: String,
    pub hostname: String,
    pub cost: String,
    pub latency: String,
    pub loss: String,
    pub nat_type: String,
    pub tunnel: String,
}

/// 统一把「人话」包成命令层能序列化的错误。
fn input_error(message: impl Into<String>) -> TheseusSerializableError {
    theseus::Error::from(theseus::ErrorKind::InputError(message.into())).into()
}

fn lock_error() -> TheseusSerializableError {
    input_error("联机状态被占用")
}

/* ------------------------------------------------------------------ *
 * 环境
 * ------------------------------------------------------------------ */

fn binaries_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    let dir = app.path().resource_dir().ok()?.join("resources/easytier");
    dir.is_dir().then_some(dir)
}

/// Windows 上判断是否管理员。`net session` 是最省事的一种：非管理员必失败。
fn is_admin() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new("net")
            .args(["session"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        // 本项目只发布 Windows 包；非 Windows 上按"没有特权"处理最安全。
        false
    }
}

fn local_ipv4_addresses() -> Vec<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } | Select-Object -ExpandProperty IPAddress",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// 把 `192.168.10.118` 推成 `192.168.10.0/24`。
fn cidr_of(ip: &str) -> Option<String> {
    let mut parts = ip.split('.');
    let a = parts.next()?;
    let b = parts.next()?;
    let c = parts.next()?;
    parts.next()?;
    Some(format!("{a}.{b}.{c}.0/24"))
}

pub fn env<R: Runtime>(app: tauri::AppHandle<R>) -> LanEnv {
    let dir = binaries_dir(&app);
    let mut missing = Vec::new();
    for name in ["easytier-core.exe", "easytier-cli.exe", "wintun.dll"] {
        let present = dir
            .as_ref()
            .map(|d| d.join(name).is_file())
            .unwrap_or(false);
        if !present {
            missing.push(name.to_string());
        }
    }
    let local_ips = local_ipv4_addresses();
    LanEnv {
        suggested_cidr: local_ips
            .iter()
            .find(|ip| ip.starts_with("192.168.") || ip.starts_with("10."))
            .and_then(|ip| cidr_of(ip))
            .or_else(|| local_ips.first().and_then(|ip| cidr_of(ip))),
        local_ips,
        is_admin: is_admin(),
        missing_files: missing,
        core_path: dir.map(|d| d.join("easytier-core.exe").to_string_lossy().to_string()),
    }
}

/* ------------------------------------------------------------------ *
 * 启动 / 停止
 * ------------------------------------------------------------------ */

/// 组命令行。抽出来是为了能单独测 —— 参数拼错在真机上只表现为「连不上」，
/// 很难从日志反推是哪一个开关写错了。
pub fn build_args(config: &LanConfig) -> Vec<String> {
    let mut args = vec![
        "--network-name".to_string(),
        config.network_name.clone(),
        "--network-secret".to_string(),
        config.network_secret.clone(),
        "-r".to_string(),
        format!("127.0.0.1:{RPC_PORT}"),
        // 只开 tcp/udp，不开 ws/wss —— 少占端口，也少被安全软件盯。
        "-l".to_string(),
        "tcp:11010".to_string(),
        "-l".to_string(),
        "udp:11010".to_string(),
    ];

    for peer in &config.peers {
        let trimmed = peer.trim();
        if !trimmed.is_empty() {
            args.push("-p".to_string());
            args.push(trimmed.to_string());
        }
    }

    // 两侧都要 TUN：房主绑服务要地址，成员访问房主也要地址。
    match config
        .virtual_ip
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(ip) => {
            args.push("-i".to_string());
            args.push(ip.to_string());
        }
        None => args.push("-d".to_string()),
    }

    if config.role == "host" {
        if let Some(cidr) = config
            .proxy_cidr
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            args.push("-n".to_string());
            args.push(cidr.to_string());
        }
    }

    args
}

/// 真正把进程拉起来。`start` 与守卫重启都走这里，避免两套拉起逻辑漂移。
fn spawn_process<R: Runtime>(app: &tauri::AppHandle<R>, config: &LanConfig) -> Result<Running> {
    let dir = binaries_dir(app)
        .ok_or_else(|| input_error("缺少内置的 EasyTier 运行时，安装包可能不完整"))?;
    let core = dir.join("easytier-core.exe");
    if !core.is_file() {
        return Err(input_error("找不到 easytier-core.exe"));
    }

    let args = build_args(config);
    let elevated = !is_admin();
    let log_path = std::env::temp_dir().join("hayenai-easytier.log");

    let child = if elevated {
        // TUN 必须要管理员。用 ShellExecute 的 runas 拉起来，拿不到句柄 ——
        // 后面靠 RPC 端口响应判断「到底起没起来」，比盲信 spawn 成功更实在。
        let arg_line = args
            .iter()
            .map(|value| {
                if value.contains(' ') {
                    format!("\"{value}\"")
                } else {
                    value.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs -WindowStyle Hidden",
            core.to_string_lossy().replace('\'', "''"),
            arg_line.replace('\'', "''")
        );
        match Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
        {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                let message = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(input_error(if message.is_empty() {
                    "提权被拒绝。联机需要管理员权限来建虚拟网卡。".to_string()
                } else {
                    format!("提权失败：{message}")
                }));
            }
            Err(error) => return Err(input_error(format!("没法调起提权：{error}"))),
        }
        None
    } else {
        let log = std::fs::File::create(&log_path).ok();
        let mut command = Command::new(&core);
        command
            .args(&args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(
                log.as_ref()
                    .and_then(|file| file.try_clone().ok())
                    .map(Stdio::from)
                    .unwrap_or_else(Stdio::null),
            )
            .stderr(log.map(Stdio::from).unwrap_or_else(Stdio::null));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let child = command
            .spawn()
            .map_err(|error| input_error(format!("启动 EasyTier 失败：{error}")))?;
        Some(child)
    };

    Ok(Running {
        child,
        elevated,
        config: config.clone(),
    })
}

/// 同一个网络 = 网络名与密钥都相同。只有这俩变了才需要重启进程。
fn same_network(a: &LanConfig, b: &LanConfig) -> bool {
    a.network_name == b.network_name && a.network_secret == b.network_secret
}

pub fn start<R: Runtime>(app: tauri::AppHandle<R>, config: LanConfig) -> Result<()> {
    if config.network_name.trim().is_empty() || config.network_secret.trim().is_empty() {
        return Err(input_error("网络名和密钥不能为空 —— 它们是房间的身份"));
    }

    {
        let guard = RUNNING.lock().map_err(|_| lock_error())?;
        if let Some(existing) = guard.as_ref() {
            // 幂等：同一个网络已经在跑就直接复用。来回切同一个房间不会反复提权 ——
            // 「一次授权后静默」的全部秘密就是进程不退出，UAC 自然不会再来。
            if same_network(&existing.config, &config) {
                return Ok(());
            }
            return Err(input_error("已经在跑另一个房间的网络，先停再开"));
        }
    }

    let running = spawn_process(&app, &config)?;
    *RUNNING.lock().map_err(|_| lock_error())? = Some(running);
    spawn_guard(app);
    Ok(())
}

/// 断线守卫：RPC 连续 3 次不通就重拉进程，最多 `MAX_RESTARTS` 次。
fn spawn_guard<R: Runtime + 'static>(app: tauri::AppHandle<R>) {
    use std::sync::atomic::Ordering;
    if GUARD_ON.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || {
        let mut misses = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(8));

            let config = match RUNNING.lock() {
                Ok(guard) => match guard.as_ref() {
                    Some(running) => running.config.clone(),
                    // 用户主动停了 —— 守卫跟着退出。
                    None => break,
                },
                Err(_) => break,
            };

            if run_cli(&app, &["node"]).is_some() {
                misses = 0;
                continue;
            }
            misses += 1;
            if misses < 3 {
                continue;
            }
            misses = 0;

            let (gave_up, restarts) = GUARD_STATE
                .lock()
                .map(|state| (state.gave_up, state.restarts))
                .unwrap_or((true, MAX_RESTARTS));
            if gave_up || restarts >= MAX_RESTARTS {
                if let Ok(mut state) = GUARD_STATE.lock() {
                    state.gave_up = true;
                }
                break;
            }

            // 先清托管记录再杀：否则旧句柄会挂在一个已经死掉的进程上。
            if let Ok(mut guard) = RUNNING.lock() {
                if let Some(mut running) = guard.take() {
                    if let Some(mut child) = running.child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            }
            kill_by_name(true);
            std::thread::sleep(std::time::Duration::from_millis(700));

            match spawn_process(&app, &config) {
                Ok(running) => {
                    if let Ok(mut guard) = RUNNING.lock() {
                        *guard = Some(running);
                    }
                    if let Ok(mut state) = GUARD_STATE.lock() {
                        state.restarts += 1;
                    }
                }
                Err(_) => {
                    if let Ok(mut state) = GUARD_STATE.lock() {
                        state.gave_up = true;
                    }
                    break;
                }
            }
        }
        GUARD_ON.store(false, Ordering::SeqCst);
    });
}

pub fn stop<R: Runtime>(app: tauri::AppHandle<R>) -> Result<()> {
    // 用户主动停 = 重置自动重连的计数，下次开房重新享有 3 次机会。
    if let Ok(mut state) = GUARD_STATE.lock() {
        *state = GuardState::default();
    }
    let mut guard = RUNNING.lock().map_err(|_| lock_error())?;
    let Some(mut running) = guard.take() else {
        // 没托管进程：可能是上次启动器被强杀留下的，顺手清一下。
        kill_by_name(false);
        return Ok(());
    };
    if let Some(mut child) = running.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if running.elevated {
        kill_by_name(true);
    }
    let _ = app;
    Ok(())
}

fn kill_by_name(elevate: bool) {
    let script = if elevate {
        "Start-Process -FilePath taskkill -ArgumentList '/F','/IM','easytier-core.exe' -Verb RunAs -WindowStyle Hidden -Wait"
    } else {
        "& taskkill /F /IM easytier-core.exe 2>$null"
    };
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if !elevate {
            command.creation_flags(CREATE_NO_WINDOW);
        }
    }
    let _ = command.status();
}

/* ------------------------------------------------------------------ *
 * 状态
 * ------------------------------------------------------------------ */

fn run_cli<R: Runtime>(app: &tauri::AppHandle<R>, args: &[&str]) -> Option<Value> {
    let dir = binaries_dir(app)?;
    let cli = dir.join("easytier-cli.exe");
    if !cli.is_file() {
        return None;
    }
    let mut command = Command::new(cli);
    command
        .args(["-p", &format!("127.0.0.1:{RPC_PORT}"), "-o", "json"])
        .args(args)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

pub fn status<R: Runtime>(app: tauri::AppHandle<R>) -> LanStatus {
    let managed = RUNNING.lock().map(|guard| guard.is_some()).unwrap_or(false);
    let elevated = RUNNING
        .lock()
        .map(|guard| guard.as_ref().map(|r| r.elevated).unwrap_or(false))
        .unwrap_or(false);

    let (restarts, gave_up) = GUARD_STATE
        .lock()
        .map(|state| (state.restarts, state.gave_up))
        .unwrap_or((0, false));

    // 以 RPC 是否响应为准，而不是「我们以为它在跑」—— 提权拉起的进程根本没句柄。
    let Some(node) = run_cli(&app, &["node"]) else {
        return LanStatus {
            running: false,
            elevated,
            resident: managed,
            restarts,
            gave_up,
            virtual_ip: None,
            hostname: None,
            public_ip: None,
            nat_type: None,
            peers: Vec::new(),
            error: managed.then(|| "EasyTier 已启动但还没响应，稍等几秒再看".to_string()),
        };
    };

    let peers = run_cli(&app, &["peer"])
        .map(|value| parse_peers(&value))
        .unwrap_or_default();

    let hostname = pick(&node, &["Hostname", "hostname"]);
    // ★ 新版 `easytier-cli node` 的输出里没有虚拟地址字段（实测只有 hostname /
    //   public_ip / nat_type），但自己的地址会出现在 `peer` 列表里，hostname 相同。
    //   只认 node 的话 virtualIp 永远是 null，前端等 30 秒后一直显示「连接中」。
    let virtual_ip = pick(&node, &["Virtual IP", "virtual_ip", "ipv4"]).or_else(|| {
        let me = hostname.as_deref()?;
        peers
            .iter()
            .find(|peer| !peer.ipv4.is_empty() && peer.hostname == me)
            .map(|peer| peer.ipv4.clone())
    });

    LanStatus {
        running: true,
        elevated,
        resident: managed,
        restarts,
        gave_up,
        virtual_ip,
        hostname,
        public_ip: pick(&node, &["Public IPv4", "public_ip"]),
        nat_type: pick(&node, &["UDP Stun Type", "udp_nat_type", "nat_type"]),
        peers,
        error: None,
    }
}

/// `easytier-cli` 的 JSON 键名随版本变过。与其死记某个键，不如按候选列表挨个试 ——
/// 拿不到只是显示成空，不会崩。
fn pick(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value.get(*key) {
            if let Some(text) = found.as_str() {
                if !text.trim().is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn parse_peers(value: &Value) -> Vec<PeerRow> {
    let rows = value
        .as_array()
        .cloned()
        .or_else(|| value.get("peers").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    rows.into_iter()
        .map(|row| PeerRow {
            ipv4: pick(&row, &["ipv4", "Virtual IP"]).unwrap_or_default(),
            hostname: pick(&row, &["hostname", "Hostname"]).unwrap_or_default(),
            cost: pick(&row, &["cost", "Cost"]).unwrap_or_default(),
            latency: pick(&row, &["lat_ms", "latency"]).unwrap_or_default(),
            loss: pick(&row, &["loss_rate", "loss"]).unwrap_or_default(),
            nat_type: pick(&row, &["nat_type"]).unwrap_or_default(),
            tunnel: pick(&row, &["tunnel_proto", "tunnel"]).unwrap_or_default(),
        })
        .collect()
}

/* ------------------------------------------------------------------ *
 * Tauri 命令
 *
 * 全部走 blocking 池：开进程、调 powershell、跑 easytier-cli 都是几百毫秒级的阻塞操作，
 * 压在 async 线程上会让状态轮询（每秒一次）互相排队。
 * ------------------------------------------------------------------ */

async fn blocking<T, F>(job: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|error| input_error(format!("任务中断：{error}")))
}

#[tauri::command]
pub async fn lan_env(app: tauri::AppHandle) -> Result<LanEnv> {
    blocking(move || env(app)).await
}

#[tauri::command]
pub async fn lan_start(app: tauri::AppHandle, config: LanConfig) -> Result<()> {
    blocking(move || start(app, config)).await?
}

/// 幂等启动：已经跑同一个网络就什么都不做。
///
/// 前端切房间统一走这条，而不是 `lan_start` —— 后者在已启动时会报错，
/// 让「进房间」这种重复动作变得不可重入。
#[tauri::command]
pub async fn lan_ensure(app: tauri::AppHandle, config: LanConfig) -> Result<()> {
    blocking(move || start(app, config)).await?
}

#[tauri::command]
pub async fn lan_stop(app: tauri::AppHandle) -> Result<()> {
    blocking(move || stop(app)).await?
}

#[tauri::command]
pub async fn lan_status(app: tauri::AppHandle) -> Result<LanStatus> {
    blocking(move || status(app)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(role: &str) -> LanConfig {
        LanConfig {
            network_name: "room-123".to_string(),
            network_secret: "abcdefgh".to_string(),
            peers: vec!["tcp://[240e::1]:11010".to_string()],
            proxy_cidr: None,
            virtual_ip: None,
            role: role.to_string(),
        }
    }

    #[test]
    fn both_roles_create_a_tun() {
        // 房主要绑服务、成员要访问房主，两侧都得有地址。
        for role in ["host", "member"] {
            let args = build_args(&config(role));
            assert!(args.contains(&"-d".to_string()), "{role} 应该走 DHCP");
            assert!(!args.contains(&"--no-tun".to_string()));
        }
    }

    #[test]
    fn only_host_exports_its_subnet() {
        let mut host = config("host");
        host.proxy_cidr = Some("192.168.10.0/24".to_string());
        assert!(build_args(&host)
            .windows(2)
            .any(|w| w[0] == "-n" && w[1] == "192.168.10.0/24"));

        let mut member = config("member");
        member.proxy_cidr = Some("192.168.10.0/24".to_string());
        assert!(!build_args(&member).contains(&"-n".to_string()));
    }

    #[test]
    fn explicit_ip_replaces_dhcp() {
        let mut cfg = config("member");
        cfg.virtual_ip = Some("10.126.126.5".to_string());
        let args = build_args(&cfg);
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-i" && w[1] == "10.126.126.5"));
        assert!(!args.contains(&"-d".to_string()));
    }

    #[test]
    fn every_peer_becomes_a_flag() {
        let args = build_args(&config("member"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-p" && w[1] == "tcp://[240e::1]:11010"));
    }

    #[test]
    fn empty_peers_are_skipped() {
        let mut cfg = config("member");
        cfg.peers = vec!["".to_string(), "  ".to_string(), "tcp://a:1".to_string()];
        let count = build_args(&cfg).iter().filter(|a| *a == "-p").count();
        assert_eq!(count, 1, "空的 peer 不该生成 -p");
    }

    #[test]
    fn cidr_derives_from_lan_ip() {
        assert_eq!(cidr_of("192.168.10.118").as_deref(), Some("192.168.10.0/24"));
        assert_eq!(cidr_of("10.1.2.3").as_deref(), Some("10.1.2.0/24"));
        assert_eq!(cidr_of("not-an-ip"), None);
    }
}
