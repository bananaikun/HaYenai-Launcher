//! 磁悬条「系统资源」档的数据源：本机 CPU / 内存 / 网络上下行。
//!
//! 为什么放在这里而不是前端算：
//! - Web 端拿不到进程级的 CPU 占用与网卡计数器，`navigator.connection` 只有链路类型；
//! - `sysinfo` 已经在依赖树里（app-lib 用它读磁盘与内存），加一个 command 的成本极低。
//!
//! 采样策略：
//! - `System` / `Networks` 都跨调用复用（放进 `OnceLock<Mutex<..>>`）。sysinfo 的占用率是
//!   **两次刷新之间的差值**，每次新建实例只会永远返回 0；
//! - 网络速度同理，用「累计字节 + 上次采样时刻」算每秒速率；
//! - 首次调用没有基准：CPU 返回 0、网速返回 0，下一拍就正常。宁可给一个 0，
//!   也不要为了基准值在命令里 `sleep` —— 那会把 IPC 线程占住。
//!
//! 前端只在磁悬条切到这一档时轮询（约 1.5s 一次），不切就不调用。

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use sysinfo::{Networks, System};

#[derive(Serialize)]
pub struct SystemStats {
    /// CPU 总占用百分比（0-100）
    pub cpu: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    /// 每秒接收/发送字节
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
    /// 每秒磁盘读取/写入字节（所有物理盘求和）
    pub disk_read_bytes_per_sec: u64,
    pub disk_write_bytes_per_sec: u64,
    /// GPU 占用百分比。取不到就是 `None`（非 NVIDIA 机器、或没装驱动工具）——
    /// 界面上显示 `—`，不要用 0 冒充"空闲"。
    pub gpu: Option<f32>,
    /// GPU 名字，取不到为 `None`
    pub gpu_name: Option<String>,
}

struct NetSample {
    at: Instant,
    rx: u64,
    tx: u64,
}

fn system() -> &'static Mutex<System> {
    static SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();
    SYSTEM.get_or_init(|| Mutex::new(System::new()))
}

fn networks() -> &'static Mutex<Networks> {
    static NETWORKS: OnceLock<Mutex<Networks>> = OnceLock::new();
    NETWORKS.get_or_init(|| Mutex::new(Networks::new_with_refreshed_list()))
}

struct DiskSample {
    at: Instant,
    read: u64,
    written: u64,
}

fn disks() -> &'static Mutex<sysinfo::Disks> {
    static DISKS: OnceLock<Mutex<sysinfo::Disks>> = OnceLock::new();
    DISKS.get_or_init(|| Mutex::new(sysinfo::Disks::new_with_refreshed_list()))
}

fn last_disk_sample() -> &'static Mutex<Option<DiskSample>> {
    static LAST: OnceLock<Mutex<Option<DiskSample>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn last_sample() -> &'static Mutex<Option<NetSample>> {
    static LAST: OnceLock<Mutex<Option<NetSample>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

#[tauri::command]
pub fn system_stats() -> Result<SystemStats, String> {
    let (cpu, memory_used, memory_total) = {
        let mut sys = system()
            .lock()
            .map_err(|_| "system stats lock poisoned".to_string())?;
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        (sys.global_cpu_usage(), sys.used_memory(), sys.total_memory())
    };

    let (rx_bytes_per_sec, tx_bytes_per_sec) = {
        let mut nets = networks()
            .lock()
            .map_err(|_| "network stats lock poisoned".to_string())?;
        nets.refresh(true);
        let rx: u64 = nets.iter().map(|(_, data)| data.total_received()).sum();
        let tx: u64 = nets.iter().map(|(_, data)| data.total_transmitted()).sum();

        let mut last = last_sample()
            .lock()
            .map_err(|_| "network sample lock poisoned".to_string())?;
        let now = Instant::now();
        let rates = match last.as_ref() {
            Some(prev) => {
                let elapsed = now.duration_since(prev.at).as_secs_f64();
                if elapsed < 0.2 {
                    // 两次调用挤在一起（切换档位时常见）：沿用 0，避免把 1ms 的抖动放大成巨值
                    (0, 0)
                } else {
                    (
                        ((rx.saturating_sub(prev.rx)) as f64 / elapsed) as u64,
                        ((tx.saturating_sub(prev.tx)) as f64 / elapsed) as u64,
                    )
                }
            }
            None => (0, 0),
        };
        *last = Some(NetSample { at: now, rx, tx });
        rates
    };

    let (disk_read_bytes_per_sec, disk_write_bytes_per_sec) = {
        let mut list = disks()
            .lock()
            .map_err(|_| "disk stats lock poisoned".to_string())?;
        list.refresh(true);
        // ⚠ 用 `total_*`（自开机累计），不是 `read_bytes/written_bytes`
        //   —— 后两个是"距上次 refresh 的增量"，拿来做差分会变成 0。
        let read: u64 = list.iter().map(|disk| disk.usage().total_read_bytes).sum();
        let written: u64 = list
            .iter()
            .map(|disk| disk.usage().total_written_bytes)
            .sum();

        let mut last = last_disk_sample()
            .lock()
            .map_err(|_| "disk sample lock poisoned".to_string())?;
        let now = Instant::now();
        let rates = match last.as_ref() {
            Some(prev) => {
                let elapsed = now.duration_since(prev.at).as_secs_f64();
                if elapsed < 0.2 {
                    (0, 0)
                } else {
                    (
                        ((read.saturating_sub(prev.read)) as f64 / elapsed) as u64,
                        ((written.saturating_sub(prev.written)) as f64 / elapsed) as u64,
                    )
                }
            }
            None => (0, 0),
        };
        *last = Some(DiskSample { at: now, read, written });
        rates
    };

    let (gpu, gpu_name) = gpu_usage();

    Ok(SystemStats {
        cpu,
        memory_used,
        memory_total,
        rx_bytes_per_sec,
        tx_bytes_per_sec,
        disk_read_bytes_per_sec,
        disk_write_bytes_per_sec,
        gpu,
        gpu_name,
    })
}


/// GPU 占用：Windows 上没有 sysinfo 这类免依赖 API，NVIDIA 机器可以直接问 `nvidia-smi`。
///
/// ⚠ 起进程比读计数器贵一个数量级，所以**至少间隔 3 秒**才真的问一次，
/// 期间返回上一次的结果。非 NVIDIA 机器上它恒为 `None`（界面显示 `—`）。
fn gpu_usage() -> (Option<f32>, Option<String>) {
    static CACHE: OnceLock<Mutex<Option<(Instant, Option<f32>, Option<String>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let now = Instant::now();

    if let Ok(guard) = cache.lock() {
        if let Some((at, util, name)) = guard.as_ref() {
            if now.duration_since(*at).as_secs_f64() < 3.0 {
                return (*util, name.clone());
            }
        }
    }

    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,name",
            "--format=csv,noheader,nounits",
        ])
        .output();

    let parsed = match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let first = text.lines().next().unwrap_or("").trim();
            let mut parts = first.split(',').map(|part| part.trim());
            let util = parts.next().and_then(|v| v.parse::<f32>().ok());
            let name = parts.next().map(|v| v.to_string());
            (util, name)
        }
        _ => (None, None),
    };

    if let Ok(mut guard) = cache.lock() {
        *guard = Some((now, parsed.0, parsed.1.clone()));
    }
    parsed
}
