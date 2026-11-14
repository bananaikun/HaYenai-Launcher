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
    /// Wallpaper Engine 场景：启动画面里**真跑** webwallgl 渲染，不是抓帧。
    /// `file` 字段这时存的是壁纸 id。
    Scene,
}

/// 当前自定义素材。`file` 是 `splash/` 下的文件名，不是绝对路径 ——
/// 这样整个目录可以连同配置一起搬走。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplashMedia {
    pub kind: SplashMediaKind,
    pub file: String,
}

/// 内置启动动画的可调参数。
///
/// 全部带默认值：老 `config.json` 里没有 `animation` 这一段也能正常读，
/// 读出来就是默认外观，不会因为升级后字段变多而丢配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplashAnimation {
    /// `auto`（有 WebGPU 就用碎片，否则粒子）／`shards`／`particles`／`static`
    pub style: String,
    /// 碎片或粒子的数量。
    pub count: u32,
    /// 运动速度倍率。
    pub speed: f32,
    /// 发光强度 0~1。
    pub glow: f32,
    /// 主色，`#rrggbb`。
    pub shard_color: String,
    /// 辅色，`#rrggbb`。
    pub accent_color: String,
    /// 鼠标交互（视差 / 按住聚拢 / 点击脉冲）。
    pub mouse_interaction: bool,
}

impl Default for SplashAnimation {
    fn default() -> Self {
        Self {
            style: "auto".to_string(),
            count: 900,
            speed: 0.8,
            glow: 0.35,
            shard_color: "#7aa2ff".to_string(),
            accent_color: "#9fd8ff".to_string(),
            mouse_interaction: true,
        }
    }
}

impl SplashAnimation {
    /// 把越界值夹回可用范围。前端滑块正常不会越界，但配置文件是可以手改的。
    fn sanitized(mut self) -> Self {
        const STYLES: [&str; 4] = ["auto", "shards", "particles", "static"];
        if !STYLES.contains(&self.style.as_str()) {
            self.style = "auto".to_string();
        }
        self.count = self.count.clamp(60, 1200);
        self.speed = self.speed.clamp(0.2, 2.0);
        self.glow = self.glow.clamp(0.0, 1.0);
        if !is_hex_color(&self.shard_color) {
            self.shard_color = "#7aa2ff".to_string();
        }
        if !is_hex_color(&self.accent_color) {
            self.accent_color = "#9fd8ff".to_string();
        }
        self
    }
}

/// `#rgb` / `#rrggbb` 之外的一律当无效，避免把任意字符串拼进内联样式。
fn is_hex_color(value: &str) -> bool {
    let bytes = value.strip_prefix('#').unwrap_or("");
    matches!(bytes.len(), 3 | 6) && bytes.chars().all(|c| c.is_ascii_hexdigit())
}

/// 自定义调整：压暗 / 模糊 / 进度条样式 / 标语与进度条的位置。
///
/// 坐标是 0~1 的相对值（相对整屏宽高），这样换分辨率也不会跑位。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplashLayout {
    /// 压暗强度 0~1。0 = 完全不压暗（默认），1 = 上面盖一层全黑渐变。
    pub scrim: f32,
    /// 背景模糊，像素。
    pub blur: f32,
    /// `bar` / `dots` / `ring` / `none`
    pub progress_style: String,
    pub tagline_x: f32,
    pub tagline_y: f32,
    pub progress_x: f32,
    pub progress_y: f32,
    /// 进度条渐变的起止色。空串 = 用内置那对蓝。
    pub accent_a: String,
    pub accent_b: String,
}

impl Default for SplashLayout {
    fn default() -> Self {
        Self {
            // 主人 2026-11-14：「启动动画壁纸有黑底压暗」——原先写死 28%~62% 的黑色渐变，
            // 默认改成 0，想要压暗自己在设置里拉。
            scrim: 0.0,
            blur: 0.0,
            progress_style: "bar".to_string(),
            tagline_x: 0.5,
            tagline_y: 0.66,
            progress_x: 0.5,
            progress_y: 0.84,
            accent_a: String::new(),
            accent_b: String::new(),
        }
    }
}

impl SplashLayout {
    fn sanitized(mut self) -> Self {
        // bar 条形 / dots 三点 / ring 环形 / stripes 斜纹流动 / pulse 呼吸条 / wave 波浪 / none 隐藏
        const STYLES: [&str; 7] = ["bar", "dots", "ring", "stripes", "pulse", "wave", "none"];
        if !STYLES.contains(&self.progress_style.as_str()) {
            self.progress_style = "bar".to_string();
        }
        self.scrim = self.scrim.clamp(0.0, 1.0);
        self.blur = self.blur.clamp(0.0, 40.0);
        self.tagline_x = self.tagline_x.clamp(0.0, 1.0);
        self.tagline_y = self.tagline_y.clamp(0.0, 1.0);
        self.progress_x = self.progress_x.clamp(0.0, 1.0);
        self.progress_y = self.progress_y.clamp(0.0, 1.0);
        // 空串合法（= 用内置色），非空则必须是合法 hex。
        if !self.accent_a.is_empty() && !is_hex_color(&self.accent_a) {
            self.accent_a.clear();
        }
        if !self.accent_b.is_empty() && !is_hex_color(&self.accent_b) {
            self.accent_b.clear();
        }
        self
    }
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
    /// 内置动画的可调参数。
    pub animation: SplashAnimation,
    /// 自定义调整（压暗/模糊/位置/进度条样式）。
    pub layout: SplashLayout,
}

impl Default for SplashConfig {
    fn default() -> Self {
        Self {
            media: None,
            show_icon: true,
            show_tagline: true,
            show_progress: true,
            tagline: String::new(),
            animation: SplashAnimation::default(),
            layout: SplashLayout::default(),
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
    // 手改过的 config.json 不能把越界值带进渲染：读的时候夹一次。
    let parsed: SplashConfig = serde_json::from_str(&raw).unwrap_or_default();
    SplashConfig {
        animation: parsed.animation.sanitized(),
        layout: parsed.layout.sanitized(),
        ..parsed
    }
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
            //
            // ⚠ 场景型必须跳过这个检查：它的 `file` 是**壁纸 id**（`ws-...`），
            // 不是 splash 目录下的文件名 —— 按文件名找必然「不存在」，
            // 结果就是刚设好的实时场景被当成素材丢失，读回来永远是 null。
            if let Some(media) = &config.media {
                let missing = match media.kind {
                    SplashMediaKind::Scene => false,
                    _ => !dir.join(&media.file).is_file(),
                };
                if missing {
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
    let config = SplashConfig {
        animation: config.animation.sanitized(),
        layout: config.layout.sanitized(),
        ..config
    };
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
    store_media(&app, &PathBuf::from(&path))
}

/// 把一张已安装壁纸的**海报图**设成启动动画背景。
///
/// 壁纸包在 `wallpapers/packages/<id>/` 下，海报就是 `project.json` 里 `preview`
/// 指向的那个文件；前端拿不到本地路径（只有取流 URL），所以这一步必须走 Rust。
#[tauri::command]
pub fn splash_set_media_from_wallpaper<R: Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<SplashConfig, String> {
    let root = crate::we_protocol::root_for(&app)
        .ok_or_else(|| "找不到壁纸目录".to_string())?;
    let store = theseus::wallpaper::store::WallpaperStore::new(root);
    let metas = store
        .list()
        .map_err(|error| format!("读壁纸列表失败：{error}"))?;
    let meta = metas
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "这张壁纸已经不在了".to_string())?;
    let preview = meta
        .preview
        .ok_or_else(|| "这张壁纸没有海报图，换一张试试".to_string())?;

    let source = store.packages_dir().join(&id).join(&preview);
    if !source.is_file() {
        return Err("海报文件不在磁盘上，可能壁纸包损坏了".to_string());
    }
    store_media(&app, &source)
}

/// 把某个 WE 场景壁纸设成启动背景 —— **实时渲染**，不是抓帧。
///
/// 只记 `{kind: scene, file: <wallpaperId>}`，不复制任何文件：
/// 启动画面会通过 `hayenai-we://` 自定义协议直接读包，
/// 那个协议在 Tauri 注册时就生效，比数据库初始化还早。
#[tauri::command]
pub fn splash_set_media_from_scene<R: Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<SplashConfig, String> {
    let root = crate::we_protocol::root_for(&app)
        .ok_or_else(|| "找不到壁纸目录".to_string())?;
    let store = theseus::wallpaper::store::WallpaperStore::new(root);
    let items = store.list().map_err(|error| format!("读壁纸列表失败：{error}"))?;
    if !items.iter().any(|item| item.id == id) {
        return Err("壁纸库里没有这张壁纸".to_string());
    }

    let dir = splash_dir(&app).ok_or_else(|| "找不到启动器数据目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|error| format!("建目录失败：{error}"))?;

    let mut config = read_config(&dir);
    // 之前可能存着抓帧图片，留着只会让人分不清当前用的是哪种。
    remove_existing_media(&dir);
    config.media = Some(SplashMedia {
        kind: SplashMediaKind::Scene,
        file: id,
    });
    write_config(&dir, &config).map_err(|error| format!("写入配置失败：{error}"))?;
    Ok(config)
}

/// 用前端截下来的**一帧真实渲染画面**当启动背景。
///
/// 为什么要这条：壁纸是 WE 实时场景，冷启动时还没法渲染 3D；
/// 所以在设置壁纸后，由主窗口把场景刚渲染出的一帧抓下来存成 `media.png`，
/// 启动画面看到的就是你当时看到的那一帧，比海报缩略图真实得多。
#[tauri::command]
pub fn splash_set_media_from_bytes<R: Runtime>(
    app: tauri::AppHandle<R>,
    data: Vec<u8>,
) -> Result<SplashConfig, String> {
    if data.is_empty() {
        return Err("画面数据是空的".to_string());
    }
    if data.len() > MAX_MEDIA_BYTES as usize {
        return Err("画面太大了".to_string());
    }

    let dir = splash_dir(&app).ok_or_else(|| "找不到启动器数据目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|error| format!("建目录失败：{error}"))?;

    // 先落临时文件，再复用 store_media 的复制/清理逻辑，避免两套写盘代码。
    let staging = dir.join("frame-cache.png");
    fs::write(&staging, &data).map_err(|error| format!("写临时文件失败：{error}"))?;
    let result = store_media(&app, &staging);
    let _ = fs::remove_file(&staging);
    result
}

/// `splash_set_media` 与 `…_from_wallpaper` 的共同落盘逻辑。
fn store_media<R: Runtime>(app: &tauri::AppHandle<R>, source: &Path) -> Result<SplashConfig, String> {
    let kind = kind_of(source)
        .ok_or_else(|| "只支持图片（png/jpg/webp/gif）与视频（mp4/webm/mov）".to_string())?;

    let meta = fs::metadata(source).map_err(|error| format!("读不到这个文件：{error}"))?;
    if meta.len() > MAX_MEDIA_BYTES {
        return Err(format!(
            "文件太大了（{:.0} MB），启动画面素材请控制在 200 MB 以内",
            meta.len() as f64 / 1024.0 / 1024.0
        ));
    }

    let dir = splash_dir(app).ok_or_else(|| "找不到启动器数据目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|error| format!("建目录失败：{error}"))?;

    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "bin".to_string());
    let file_name = format!("{MEDIA_STEM}.{extension}");

    remove_existing_media(&dir);
    fs::copy(source, dir.join(&file_name)).map_err(|error| format!("复制素材失败：{error}"))?;

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
