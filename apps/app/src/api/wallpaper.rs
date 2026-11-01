//! 壁纸相关的 Tauri 命令（插件名 `wallpaper`）。
//!
//! 前端只需要 invoke 这九个命令：
//!
//! | 命令 | 作用 |
//! | --- | --- |
//! | `wallpaper_list` | 列出已安装的包 |
//! | `wallpaper_import` | 导入 `.mpkg` / `.pkg` / 目录 / 工坊目录 |
//! | `wallpaper_remove` | 删除包（幂等） |
//! | `wallpaper_select` | 选中 / 取消选中 |
//! | `wallpaper_config_get` | 读 `config.json` |
//! | `wallpaper_config_set` | 写 `config.json`（原子） |
//! | `wallpaper_steam_scan` | 扫本机 Steam 库里 WE 的工坊壁纸（纯本地、只读） |
//! | `wallpaper_steam_import` | 导入扫到的那一条（只认本次扫描结果） |
//! | `wallpaper_steam_import_url` | 粘工坊链接 / 纯 id 一键导入（本机没下就报 `not_downloaded`） |
//!
//! 所有失败都只回一句中文（`{ fieldName, code, message }`，与其它命令的错误形状
//! 一致）：原始错误里的绝对路径与堆栈只写日志，不进返回值。

use serde::Serialize;
use tauri::Runtime;

/// 命令进入/失败的痕迹。排查「点了没反应」时先看这里。
const LOG_TARGET: &str = "theseus_gui::wallpaper";

use theseus::wallpaper::steam::{
    self, SteamError, SteamScan,
};
use theseus::wallpaper::store::{ImportOptions, ImportOutcome, WallpaperStore};
use theseus::wallpaper::{
    WallpaperConfig, WallpaperError, WallpaperMeta,
};

/// 命令错误。形状与 `TheseusSerializableError` 的序列化结果一致，
/// 前端那套 `error.message` 的处理不用改。
///
/// `code` 是**新增的**机器可读细分原因，老字段一个没动（向后兼容）：
/// 只读 `message` 的老代码照旧能显示，新代码可以按 `code` 决定给哪个按钮。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperCommandError {
    pub field_name: &'static str,
    /// 机器可读原因：`wallpaper` / `steam` / `not_downloaded` /
    /// `invalid_url` / `interrupted` / `no_root`。
    pub code: &'static str,
    pub message: String,
}

impl WallpaperCommandError {
    /// 把领域错误翻成一句中文，同时把原文（含路径）留给日志。
    fn from_wallpaper(error: WallpaperError) -> Self {
        tracing::warn!(error = ?error, "wallpaper 命令失败");
        Self {
            field_name: "Wallpaper",
            code: "wallpaper",
            message: error.user_message(),
        }
    }

    /// 同上，Steam 扫描 / 导入那条链路。
    fn from_steam(error: SteamError) -> Self {
        tracing::warn!(error = ?error, "wallpaper steam 命令失败");
        Self {
            field_name: "Wallpaper",
            code: "steam",
            message: error.user_message(),
        }
    }

    /// 「粘贴链接导入」那条链路：把「本机没有这个条目」单拎成一个 code，
    /// 前端据此给「在 Steam 中打开」按钮，而不是让用户对着「请重新扫描」发呆。
    fn from_steam_url(error: SteamError) -> Self {
        match error {
            // 没扫到、或者扫描之后目录被删（= 在 Steam 里取消订阅了），
            // 对用户来说都是「本机没有」，动作都是「去 Steam 订阅」。
            SteamError::NotScanned | SteamError::SourceGone => {
                tracing::info!(?error, "工坊条目不在本机，提示去 Steam 订阅");
                Self {
                    field_name: "Wallpaper",
                    code: "not_downloaded",
                    message: "这个壁纸还没在 Steam 里下载，先在 Steam 中订阅它"
                        .to_string(),
                }
            }
            other => Self::from_steam(other),
        }
    }

    /// 粘进来的东西不是工坊链接，也不是工坊 id。
    fn invalid_url() -> Self {
        Self {
            field_name: "Wallpaper",
            code: "invalid_url",
            message: "无法识别的创意工坊链接".to_string(),
        }
    }

    /// 后台任务被取消 / panic 时的兜底。
    fn interrupted() -> Self {
        Self {
            field_name: "Wallpaper",
            code: "interrupted",
            message: "壁纸操作被中断，请重试".to_string(),
        }
    }

    /// 连壁纸目录都算不出来（几乎没有可能）。
    fn no_root() -> Self {
        Self {
            field_name: "Wallpaper",
            code: "no_root",
            message: "找不到壁纸目录".to_string(),
        }
    }
}

/// 解析 `wallpapers/` 根目录并构造 store。
fn store_for<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<WallpaperStore, WallpaperCommandError> {
    crate::we_protocol::root_for(app)
        .map(WallpaperStore::new)
        .ok_or_else(WallpaperCommandError::no_root)
}

/// 列出所有已安装的壁纸。损坏的也会列出来（`damaged: true`）。
#[tauri::command]
pub async fn wallpaper_list<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<WallpaperMeta>, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_list", "invoked");
    let store = store_for(&app)?;
    // 内置壁纸只在首次落地（21MB 复制 + 一次性默认选中），之后这里只剩一次
    // `is_dir()` 判断。放在 list 之前，前端读列表时就一定能看到它。
    let bundled = crate::api::bundled_wallpaper::source_dir(&app);
    // 扫目录 + 读每个 .tex 的 64 字节头是同步 IO，不能压在 async 线程上。
    tauri::async_runtime::spawn_blocking(move || {
        crate::api::bundled_wallpaper::seed(&store, bundled.as_deref());
        store.list()
    })
    .await
    .map_err(|_| WallpaperCommandError::interrupted())?
    .map_err(WallpaperCommandError::from_wallpaper)
}

/// 读取一个壁纸包里 `project.json` 的 `general.properties`。
///
/// **原样返回**，前端自己按 `type` 渲染：`slider`(min/max/step/precision/fraction)、
/// `bool`、`color`（`"1 1 1"` 空格分隔 RGB 0~1）、`group`（分组标题），
/// 并按 `order` 排序、按 `condition`（如 `mediaplayertoggle.value == true`）做条件显隐。
///
/// 不在后端解释 schema：WE 的属性格式是壁纸作者自由定义的，后端只当搬运工，
/// 免得每遇到一种新 type 就要改一次 Rust。
#[tauri::command]
pub async fn wallpaper_properties<R: Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<serde_json::Value, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_properties", %id, "invoked");
    let store = store_for(&app)?;
    let dir = store
        .package_dir(&id)
        .map_err(WallpaperCommandError::from_wallpaper)?;
    let project_path = dir.join("project.json");

    let raw = tokio::fs::read_to_string(&project_path)
        .await
        .map_err(|error| {
            tracing::warn!(?error, path = %project_path.display(), "读 project.json 失败");
            WallpaperCommandError {
                field_name: "Wallpaper",
                code: "wallpaper",
                message: "这个壁纸包里没有可读的 project.json。".to_string(),
            }
        })?;

    let json: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        tracing::warn!(?error, "project.json 解析失败");
        WallpaperCommandError {
            field_name: "Wallpaper",
            code: "wallpaper",
            message: "project.json 格式不对，无法读取可调项。".to_string(),
        }
    })?;

    Ok(json
        .pointer("/general/properties")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

/// 导入一个壁纸包。
///
/// `path` 可以是：`.mpkg` 文件、只有场景的 `.pkg` 文件、松散工程目录、
/// 带 `scene.pkg` 的工坊目录。
#[tauri::command]
pub async fn wallpaper_import<R: Runtime>(
    app: tauri::AppHandle<R>,
    path: String,
) -> Result<ImportOutcome, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_import", "invoked");
    let store = store_for(&app)?;
    let options = ImportOptions::default();
    tauri::async_runtime::spawn_blocking(move || {
        store.import(std::path::Path::new(&path), &options)
    })
    .await
    .map_err(|_| WallpaperCommandError::interrupted())?
    .map_err(WallpaperCommandError::from_wallpaper)
}

/// 删除一个包。目录本来就不在也算成功。
#[tauri::command]
pub async fn wallpaper_remove<R: Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<(), WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_remove", "invoked");
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || store.remove(&id))
        .await
        .map_err(|_| WallpaperCommandError::interrupted())?
        .map_err(WallpaperCommandError::from_wallpaper)
}

/// 修复容器形态的旧包（`scene.pkg` 未展开）。
///
/// 早期版本导入的工坊目录会留下这种形态：`project.json` 指向 `scene.json`，
/// 但那个文件只在 `.pkg` 里。界面会把它标成损坏、按钮禁用。
/// 展开之后返回 `true`，前端重新拉一次列表即可正常使用。
#[tauri::command]
pub async fn wallpaper_repair<R: Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<bool, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_repair", "invoked");
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || store.repair(&id))
        .await
        .map_err(|_| WallpaperCommandError::interrupted())?
        .map_err(WallpaperCommandError::from_wallpaper)
}

/// 选中（`id` 为 `Some`）或取消选中（`None`）。
#[tauri::command]
pub async fn wallpaper_select<R: Runtime>(
    app: tauri::AppHandle<R>,
    id: Option<String>,
) -> Result<(), WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_select", "invoked");
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        store.select(id.as_deref())
    })
    .await
    .map_err(|_| WallpaperCommandError::interrupted())?
    .map_err(WallpaperCommandError::from_wallpaper)
}

/// 读 `config.json`。文件不存在时返回默认值。
#[tauri::command]
pub async fn wallpaper_config_get<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<WallpaperConfig, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_config_get", "invoked");
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || store.read_config())
        .await
        .map_err(|_| WallpaperCommandError::interrupted())?
        .map_err(WallpaperCommandError::from_wallpaper)
}

/// 覆盖写 `config.json`（临时文件 + 原子 rename）。
#[tauri::command]
pub async fn wallpaper_config_set<R: Runtime>(
    app: tauri::AppHandle<R>,
    config: WallpaperConfig,
) -> Result<(), WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_config_set", "invoked");
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || store.write_config(&config))
        .await
        .map_err(|_| WallpaperCommandError::interrupted())?
        .map_err(WallpaperCommandError::from_wallpaper)
}

/// 扫描本机 Steam 库里的 Wallpaper Engine 工坊壁纸。
///
/// 纯本地只读：注册表 → `libraryfolders.vdf` → `workshop/content/431960/*`。
/// 不登录、不联网、不写 Steam 目录。
///
/// 副作用：`steam::scan` 内部**无条件**把这次结果登记进进程内的目录表，
/// `wallpaper_steam_import` 只认那张表 —— 所以「扫描 → 导入」必须先扫一次。
#[tauri::command]
pub async fn wallpaper_steam_scan<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<SteamScan, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_steam_scan", "invoked");
    let store = store_for(&app)?;
    // 走几十个目录 + 递归统计体积是同步 IO，不能压在 async 线程上。
    tauri::async_runtime::spawn_blocking(move || steam::scan(&store))
        .await
        .map_err(|_| WallpaperCommandError::interrupted())?
        .map_err(WallpaperCommandError::from_steam)
}

/// 导入一个**已扫到**的工坊壁纸。
///
/// `workshop_id` 必须出现在最近一次 `wallpaper_steam_scan` 的结果里；
/// 否则返回「请重新扫描」—— 这条校验是「不许任意路径读盘」的唯一闸门。
#[tauri::command]
pub async fn wallpaper_steam_import<R: Runtime>(
    app: tauri::AppHandle<R>,
    workshop_id: String,
) -> Result<ImportOutcome, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_steam_import", "invoked");
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        steam::import(&store, &workshop_id)
    })
    .await
    .map_err(|_| WallpaperCommandError::interrupted())?
    .map_err(WallpaperCommandError::from_steam)
}

/// 粘贴工坊链接 / 纯 id 一键导入。
///
/// 认 `https://steamcommunity.com/sharedfiles/filedetails/?id=…`、
/// `…/workshop/filedetails/?id=…`、`steam://url/CommunityFilePage/…`
/// 和纯数字 id（见 [`steam::workshop_id_from_input`]）。
///
/// **不新开下载通道**：只把 id 拿去查本进程最近一次扫描的目录表。
/// 查不到就返回 `code = "not_downloaded"`（文案提示去 Steam 订阅），
/// 由前端给「在 Steam 中打开」（链接见 [`steam::workshop_page_url`]）。
#[tauri::command]
pub async fn wallpaper_steam_import_url<R: Runtime>(
    app: tauri::AppHandle<R>,
    url: String,
) -> Result<ImportOutcome, WallpaperCommandError> {
    tracing::info!(target: LOG_TARGET, cmd = "wallpaper_steam_import_url", "invoked");
    let workshop_id =
        steam::workshop_id_from_input(&url).ok_or_else(WallpaperCommandError::invalid_url)?;
    let store = store_for(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        steam::import(&store, &workshop_id)
    })
    .await
    .map_err(|_| WallpaperCommandError::interrupted())?
    .map_err(WallpaperCommandError::from_steam_url)
}

/// 注册插件。名字用连字符（与 `content-search` 等保持一致）。
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("wallpaper")
        .invoke_handler(tauri::generate_handler![
            wallpaper_list,
            wallpaper_properties,
            wallpaper_import,
            wallpaper_remove,
            wallpaper_repair,
            wallpaper_select,
            wallpaper_config_get,
            wallpaper_config_set,
            wallpaper_steam_scan,
            wallpaper_steam_import,
            wallpaper_steam_import_url
        ])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_a_single_chinese_sentence() {
        let error = WallpaperCommandError::from_wallpaper(
            WallpaperError::MissingProject,
        );

        assert_eq!(error.message, "壁纸包里没有 project.json，无法识别");
        assert!(!error.message.contains('\n'));
        assert_eq!(error.field_name, "Wallpaper");
    }

    #[test]
    fn interrupted_and_missing_root_have_plain_messages() {
        assert_eq!(
            WallpaperCommandError::interrupted().message,
            "壁纸操作被中断，请重试"
        );
        assert_eq!(WallpaperCommandError::no_root().message, "找不到壁纸目录");
    }

    #[test]
    fn the_error_shape_matches_the_rest_of_the_app() {
        let error = WallpaperCommandError::no_root();
        let json = serde_json::to_value(&error).expect("serialise");

        assert_eq!(json["fieldName"], "Wallpaper");
        assert_eq!(json["message"], "找不到壁纸目录");
        assert!(json.get("field_name").is_none());
        // `code` 是新增字段，老的两个字段必须原样还在（向后兼容）。
        assert_eq!(json["code"], "no_root");
        assert!(json.get("fieldName").is_some());
        assert!(json.get("message").is_some());
    }

    #[test]
    fn steam_errors_are_flattened_to_one_chinese_sentence() {
        let error =
            WallpaperCommandError::from_steam(SteamError::NotScanned);
        assert_eq!(
            error.message,
            "这个壁纸不在最近一次扫描结果里，请重新扫描"
        );
        assert_eq!(error.field_name, "Wallpaper");
        assert_eq!(error.code, "steam");
        assert!(!error.message.contains('\n'));

        let error =
            WallpaperCommandError::from_steam(SteamError::SteamNotFound);
        assert_eq!(error.message, "找不到 Steam 安装位置，可能没有装 Steam");
        assert_eq!(error.code, "steam");
    }

    #[test]
    fn every_error_path_has_a_distinct_code() {
        let codes = [
            WallpaperCommandError::from_wallpaper(WallpaperError::MissingProject).code,
            WallpaperCommandError::from_steam(SteamError::SteamNotFound).code,
            WallpaperCommandError::interrupted().code,
            WallpaperCommandError::no_root().code,
            WallpaperCommandError::invalid_url().code,
            WallpaperCommandError::from_steam_url(SteamError::NotScanned).code,
        ];
        assert_eq!(
            codes,
            [
                "wallpaper",
                "steam",
                "interrupted",
                "no_root",
                "invalid_url",
                "not_downloaded",
            ]
        );
    }

    #[test]
    fn a_url_import_that_is_not_on_disk_is_distinguishable() {
        // 「没下载」必须能和「别的失败」分开：前端据此给「在 Steam 中打开」。
        for error in [
            WallpaperCommandError::from_steam_url(SteamError::NotScanned),
            // 扫描之后目录被删（= 取消订阅）对用户是同一件事。
            WallpaperCommandError::from_steam_url(SteamError::SourceGone),
        ] {
            assert_eq!(error.code, "not_downloaded");
            assert_eq!(
                error.message,
                "这个壁纸还没在 Steam 里下载，先在 Steam 中订阅它"
            );
            assert!(!error.message.contains(":\\"), "不能带路径");
        }

        // 其他失败仍然走 `steam`，不要把 code 混掉。
        assert_eq!(
            WallpaperCommandError::from_steam_url(SteamError::SteamNotFound).code,
            "steam"
        );
    }

    #[test]
    fn an_unrecognised_link_is_reported_as_invalid_url() {
        let error = WallpaperCommandError::invalid_url();
        assert_eq!(error.code, "invalid_url");
        assert_eq!(error.message, "无法识别的创意工坊链接");

        let json = serde_json::to_value(&error).expect("serialise");
        assert_eq!(json["code"], "invalid_url");
        assert_eq!(json["fieldName"], "Wallpaper");
    }
}
