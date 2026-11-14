//! 内存自动优化：配置读写 + 后台任务（定时 / 按占用）。
//!
//! 为什么配置不放进 `Settings`（数据库那张列式大表）：
//! 那张表是一列一个字段 + `sqlx::query!` 编译期校验，加字段要同时改建表 SQL、
//! SELECT、UPDATE 并刷新 `.sqlx` 离线缓存 —— 为三个开关动这块，风险和成本都不划算。
//! 这里单独存一份小 JSON（`<设置目录>/memory-auto.json`），读写都在 Rust 侧，
//! 前端只需调两个命令。
//!
//! 为什么任务放在 Rust 侧而不是前端定时器：启动游戏后启动器会进**轻量模式**，
//! 主窗口的 WebView 被换掉，前端定时器全部停摆 —— 而这恰恰是最需要清内存的时候
//! （游戏刚把几个 G 拿走）。后台任务跟着进程活，轻量模式下照常跑。
//!
//! ⚠ 只调 `optimize_silent()`：不带提权的版本。自动流程绝不能弹 UAC。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// 检查间隔：30 秒看一次条件，够及时又不浪费唤醒。
const TICK: Duration = Duration::from_secs(30);
/// 两次自动优化之间的最小间隔。
const COOLDOWN: Duration = Duration::from_secs(300);
/// 配置文件放在设置目录下。
const CONFIG_FILE: &str = "memory-auto.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoMemoryConfig {
    /// 总开关。
    pub enabled: bool,
    /// 每多少分钟清一次；0 = 不定时。
    pub interval_minutes: u32,
    /// 已用内存达到这个百分比就清一次；0 = 按占用不触发。
    pub threshold_percent: u32,
}

impl Default for AutoMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 30,
            threshold_percent: 0,
        }
    }
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let identifier = app.config().identifier.clone();
    theseus::DirectoryInfo::initial_settings_dir_path(&identifier)
        .map(|dir| dir.join(CONFIG_FILE))
}

fn load_config<R: Runtime>(app: &AppHandle<R>) -> AutoMemoryConfig {
    let Some(path) = config_path(app) else {
        return AutoMemoryConfig::default();
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => AutoMemoryConfig::default(),
    }
}

fn store_config<R: Runtime>(app: &AppHandle<R>, config: &AutoMemoryConfig) -> Result<(), String> {
    let path = config_path(app).ok_or_else(|| "settings directory unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let encoded = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    std::fs::write(&path, encoded).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn memory_auto_get<R: Runtime>(app: AppHandle<R>) -> AutoMemoryConfig {
    load_config(&app)
}

#[tauri::command]
pub fn memory_auto_set<R: Runtime>(
    app: AppHandle<R>,
    config: AutoMemoryConfig,
) -> Result<(), String> {
    store_config(&app, &config)
}

/// 启动后台循环。整个进程生命周期只该调一次。
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        // 上次真正执行的时间；`None` 表示还没跑过。
        let mut last_run: Option<Instant> = None;
        // 定时轴的起点：关掉开关时会被重置。
        let mut interval_mark = Instant::now();
        // 配置变了就重置定时轴（60 分钟改成 5 分钟不该再等满 60）。
        let mut last_config = load_config(&app);

        loop {
            tokio::time::sleep(TICK).await;

            let config = load_config(&app);

            if config.enabled != last_config.enabled
                || config.interval_minutes != last_config.interval_minutes
                || config.threshold_percent != last_config.threshold_percent
            {
                interval_mark = Instant::now();
                last_config = config.clone();
            }

            if !config.enabled {
                continue;
            }

            if let Some(last) = last_run {
                if last.elapsed() < COOLDOWN {
                    continue;
                }
            }

            let mut reason: Option<&'static str> = None;

            if config.interval_minutes > 0 {
                let interval = Duration::from_secs(u64::from(config.interval_minutes) * 60);
                if interval_mark.elapsed() >= interval {
                    reason = Some("interval");
                }
            }

            if reason.is_none() && config.threshold_percent > 0 {
                if let Some(used) = used_percent().await {
                    if used >= f64::from(config.threshold_percent) {
                        reason = Some("threshold");
                    }
                }
            }

            let Some(reason) = reason else {
                continue;
            };

            match theseus::memory::optimize_silent().await {
                Ok(result) => {
                    tracing::info!(
                        reason,
                        reclaimed_mb = result.reclaimed_bytes / 1_048_576,
                        "auto memory optimization finished"
                    );
                    last_run = Some(Instant::now());
                    if reason == "interval" {
                        interval_mark = Instant::now();
                    }
                }
                Err(error) => {
                    // 拿不到权限/系统拒绝都算正常情况：静默跳过，等下一轮。
                    tracing::debug!(reason, %error, "auto memory optimization skipped");
                    last_run = Some(Instant::now());
                }
            }
        }
    });
}

/// 系统已用内存百分比（0–100）。读不到就返回 None。
async fn used_percent() -> Option<f64> {
    let status = theseus::jre::get_memory_status(None, 0, true).await.ok()?;
    if status.total_bytes == 0 {
        return None;
    }
    let used = status.total_bytes.saturating_sub(status.available_bytes);
    Some((used as f64 / status.total_bytes as f64) * 100.0)
}
