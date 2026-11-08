//! 启动动画（Splash）的自定义素材与开关。
//!
//! 存储刻意**不走数据库**：启动画面在 `State` 初始化之前就要渲染，那时候
//! SQLite 连接还没建好。一份 `splash/config.json` 加一个媒体文件，读起来没有
//! 任何前置依赖，与 `wallpapers/config.json` 的做法一致。
//!
//! 素材一律**复制**到 `<settings>/splash/`，不是引用原路径 —— 用户把桌面上的
//! 视频挪走之后，启动画面不该变成一块黑屏。
//!
//! 路径：`<settings_dir>/splash/config.json` + `media.<ext>`

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Runtime;

/// 素材上限。启动画面每次冷启动都要解码，几百 MB 的视频会让它比主界面还慢。
const MAX_MEDIA_BYTES: u64 = 200 * 1024 * 1024;

const MEDIA_STEM: &str = "media";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SplashMediaKind {
    Image,
    Video,
}

/// 当前自定义素材。`file` 是 `splash/` 下的文件名，不是绝对路径 ——
/// 这样整个目录可以连同配置一起搬走。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplashMedia {
    pub kind: SplashMediaKind,
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SplashConfig {
    /// 没有自定义素材时是 `None`，此时用内置粒子背景。
    pub media: Option<SplashMedia>,
    pub show_icon: bool,
    pub show_tagline: bool,
    pub show_progress: bool,
    /// 自定义标语。空串表示用内置那句。
    pub tagline: String,
}

impl Default for SplashConfig {
    fn default() -> Self {
        Self {
            media: None,
            show_icon: true,
            show_tagline: true,
            show_progress: true,
            tagline: String::new(),
        }
    }
}

fn splash_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    let identifier = app.config().identifier.clone();
    let dir = theseus::DirectoryInfo::initial_settings_dir_path(&identifier)?
        .join("splash");
    Some(dir)
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

fn read_config(dir: &Path) -> SplashConfig {
    let Ok(raw) = fs::read_to_string(config_path(dir)) else {
        return SplashConfig::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn write_config(dir: &Path, config: &SplashConfig) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(config)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    // 先写临时文件再改名：启动画面每次冷启动都会读它，读到半个 JSON 会静默退回默认值。
    let tmp = config_path(dir).with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, config_path(dir))
}

fn kind_of(path: &Path) -> Option<SplashMediaKind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "avif" => {
            Some(SplashMediaKind::Image)
        }
        "mp4" | "webm" | "mov" | "m4v" => Some(SplashMediaKind::Video),
        _ => None,
    }
}

fn remove_existing_media(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(MEDIA_STEM) && name != "config.json" {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[tauri::command]
pub fn splash_get_config<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> SplashConfig {
    match splash_dir(&app) {
        Some(dir) => {
            let config = read_config(&dir);
            // 配置说有素材、盘上却没有（被手工删了）⇒ 当成没有，别让启动画面黑屏。
            if let Some(media) = &config.media {
                if !dir.join(&media.file).is_file() {
                    tracing::warn!(file = %media.file, "启动动画素材缺失，回退到内置背景");
                    return SplashConfig { media: None, ..config };
                }
            }
            config
        }
        None => SplashConfig::default(),
    }
}

#[tauri::command]
pub fn splash_set_config<R: Runtime>(
    app: tauri::AppHandle<R>,
    config: SplashConfig,
) -> Result<SplashConfig, String> {
    let dir = splash_dir(&app).ok_or_else(|| "找不到启动器数据目录".to_string())?;
    write_config(&dir, &config).map_err(|error| format!("写入启动动画配置失败：{error}"))?;
    Ok(config)
}

/// 把用户选的媒体文件复制进 `splash/`，并更新配置。
///
/// 换素材时旧的会被删掉 —— 留着只会让数据目录里堆一串没人认领的视频。
#[tauri::command]
pub fn splash_set_media<R: Runtime>(
    app: tauri::AppHandle<R>,
    path: String,
) -> Result<SplashConfig, String> {
    let source = PathBuf::from(&path);
    let kind = kind_of(&source)
        .ok_or_else(|| "只支持图片（png/jpg/webp/gif）与视频（mp4/webm/mov）".to_string())?;

    let meta = fs::metadata(&source).map_err(|error| format!("读不到这个文件：{error}"))?;
    if meta.len() > MAX_MEDIA_BYTES {
        return Err(format!(
            "文件太大了（{:.0} MB），启动画面素材请控制在 200 MB 以内",
            meta.len() as f64 / 1024.0 / 1024.0
        ));
    }

    let dir = splash_dir(&app).ok_or_else(|| "找不到启动器数据目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|error| format!("建目录失败：{error}"))?;

    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "bin".to_string());
    let file_name = format!("{MEDIA_STEM}.{extension}");

    remove_existing_media(&dir);
    fs::copy(&source, dir.join(&file_name))
        .map_err(|error| format!("复制素材失败：{error}"))?;

    let mut config = read_config(&dir);
    config.media = Some(SplashMedia { kind, file: file_name });
    write_config(&dir, &config).map_err(|error| format!("写入配置失败：{error}"))?;
    Ok(config)
}

#[tauri::command]
pub fn splash_clear_media<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<SplashConfig, String> {
    let dir = splash_dir(&app).ok_or_else(|| "找不到启动器数据目录".to_string())?;
    remove_existing_media(&dir);
    let mut config = read_config(&dir);
    config.media = None;
    write_config(&dir, &config).map_err(|error| format!("写入配置失败：{error}"))?;
    Ok(config)
}

/// 素材的绝对路径，给前端拼 `convertFileSrc` 用。没有素材时返回 `None`。
#[tauri::command]
pub fn splash_media_path<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Option<String> {
    let dir = splash_dir(&app)?;
    let config = read_config(&dir);
    let media = config.media?;
    let path = dir.join(&media.file);
    path.is_file().then(|| path.to_string_lossy().into_owned())
}
