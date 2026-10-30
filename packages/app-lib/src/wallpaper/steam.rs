/*!
本机 Steam 库里 Wallpaper Engine 工坊壁纸的扫描与导入。

**纯本地、零凭据**：只读注册表里的 Steam 安装路径、`libraryfolders.vdf`、
`appworkshop_431960.acf` 和 `workshop/content/431960/<id>/project.json`，
不碰 Steam 账号、不发网络请求、不写 Steam 目录里任何东西。

```text
HKCU\Software\Valve\Steam  SteamPath = "d:/steam"
└─ steamapps/libraryfolders.vdf      "0" { "path" "D:\\steam" }
   └─ steamapps/workshop/
      ├─ appworkshop_431960.acf      WorkshopItemsInstalled.<id>{size,timeupdated,manifest}
      └─ content/431960/<id>/        每个订阅条目一个目录
         ├─ project.json             title / type / preview / file / workshopid
         ├─ scene.pkg                （场景包，`store::import` 会就地展开）
         └─ preview.gif|jpg|png      缩略图
```

四条硬规则：

1. **单条坏目录不能拖垮整次扫描**：解析失败的目录只往 [`SteamScan::skipped`]
   里记一条原因，其余照常返回。没有壁纸内容的空壳目录（缺 `project.json`、
   `file` 为空、入口和 `scene.pkg` 都不在）也走这条路 —— 它们不该出现在列表里。
2. **`workshopid` 兜底用目录名**：实测 65 个条目里 24 个 `project.json`
   **没有** `workshopid`（工坊包是 Steam 装的，不是 WE 自己导出的）。
3. **`type` 必须归一化**：真实数据里 `scene` / `Scene` / `SCENE` 混着出现。
   未识别的 `type` 只标「不支持」，**不**过滤掉。
4. **导入只认本次扫描结果**：`wallpaper_steam_import` /
   `wallpaper_steam_import_url` 收到的 `workshop_id` 必须先在本进程最近一次
   扫描的目录表里出现过，否则一律拒绝 —— 否则前端（或任何能调命令的东西）
   就能让启动器读任意路径。
*/

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

use super::WallpaperType;
use super::pkg::safe_rel;
use super::store::{
    self, ImportOptions, ImportOutcome, WallpaperStore,
};

/// Wallpaper Engine 在 Steam 上的 AppID。
pub const WALLPAPER_ENGINE_APP_ID: &str = "431960";

/// `project.json` 的体积上限。实测几 KB，超过 1 MiB 的不是清单。
const MAX_PROJECT_BYTES: u64 = 1024 * 1024;
/// `project.json` 的候选文件名（WE 一直用这个名字，留个常量便于对照）。
const PROJECT_FILE_NAME: &str = "project.json";
/// 场景包文件名。
///
/// 工坊里的场景壁纸几乎都是「打包」形态：`project.json.file` 写的是
/// `scene.json`，而磁盘上**只有** `scene.pkg`（`scene.json` 在包里）。
/// 所以「入口文件不存在」不能直接判死，这是 [`read_item`] 的唯一例外。
const SCENE_PACKAGE_NAME: &str = "scene.pkg";
/// 找不到 `project.json.preview` 时按顺序探测的缩略图文件名。
///
/// 顺序有意义：`.gif` 在前，WE 自己的库也是优先用动图当封面。
const PREVIEW_CANDIDATES: [&str; 5] = [
    "preview.gif",
    "preview.jpg",
    "preview.png",
    "preview.jpeg",
    "preview.webp",
];
/// 工坊 id 的最大长度。Steam 现在发的是 10 位，留足余量。
const MAX_WORKSHOP_ID_LEN: usize = 20;
/// VDF 嵌套深度上限。
const MAX_VDF_DEPTH: usize = 32;

/// Steam 相关操作的错误。`Display` 即用户可见文案（中文单句，无路径、无堆栈）。
#[derive(Debug, thiserror::Error)]
pub enum SteamError {
    /// 注册表里没有 Steam，或值读不出来。
    #[error("找不到 Steam 安装位置，可能没有装 Steam")]
    SteamNotFound,

    /// 扫描结果里没有这个工坊 id（可能是过期结果，或者根本没扫到）。
    #[error("这个壁纸不在最近一次扫描结果里，请重新扫描")]
    NotScanned,

    /// 扫描时还在，导入时目录已经没了（用户在 Steam 里取消订阅）。
    #[error("这个壁纸的文件夹不见了，可能已经在 Steam 里取消订阅")]
    SourceGone,

    /// 文件读写失败。**不**在文案里带上裸 `io::Error`：它含绝对路径。
    #[error("读写 Steam 工坊文件失败")]
    Io(#[from] std::io::Error),

    /// 导入本身失败，原样带上壁纸模块那句中文。
    #[error("{0}")]
    Import(super::WallpaperError),
}

impl SteamError {
    /// 用户可见文案。与 `Display` 相同，单独留一个名字是为了调用方读起来明确。
    pub fn user_message(&self) -> String {
        self.to_string()
    }
}

/// 一个工坊壁纸条目。字段全部是前端渲染卡片需要的，没有一个字来自网络。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamWallpaper {
    /// 工坊 id。目录名优先；`project.json` 里写了 `workshopid` 则以它为准。
    pub workshop_id: String,
    /// 磁盘上的绝对路径（只用于显示与日志；取缩略图走 `/steam/<id>/<rel>`）。
    pub dir: String,
    /// 标题。`project.json.title` 为空时退回目录名。
    pub title: String,
    /// `scene` / `video` / `web` / `application` / `other`（已归一化成小写）。
    pub kind: String,
    /// 包内缩略图相对路径（`preview.gif` 之类），没找到就是 `None`。
    pub preview: Option<String>,
    /// 目录递归体积（字节）。
    pub bytes: u64,
    /// 安装时间，Unix **毫秒**（[`appworkshop_431960.acf`] 里是秒，这里 ×1000）；
    /// acf 没记录时为 `None`。
    pub installed_at: Option<i64>,
    /// `installed`（acf 里有）/ `orphan`（磁盘有、acf 没记）/ `unknown`（读不到 acf）。
    pub item_state: String,
    /// 已经在启动器壁纸库里时，给出那个包的 id。
    pub imported_id: Option<String>,
}

/// 一条被跳过的目录，以及原因（中文单句）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamSkipped {
    pub dir: String,
    pub reason: String,
}

/// 一次扫描的完整结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamScan {
    /// 扫到的条目，按标题排序（大小写不敏感）。
    pub items: Vec<SteamWallpaper>,
    /// 跳过的目录。**不让单条失败影响整体**，所以这里只是清单，不是错误。
    pub skipped: Vec<SteamSkipped>,
    /// 实际扫过的 `workshop/content/431960` 目录（已经去重）。
    pub roots: Vec<String>,
    /// Steam 安装根目录（注册表读到的那个），读不到就是 `None`。
    pub steam_root: Option<String>,
}

impl SteamScan {
    /// 按 `kind` 数条数，报告与界面都用得上。
    pub fn kind_counts(&self) -> HashMap<String, usize> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for item in &self.items {
            *counts.entry(item.kind.clone()).or_insert(0) += 1;
        }
        counts
    }
}

/// `appworkshop_431960.acf` 里记的一条安装信息。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AcfEntry {
    /// Steam 记的体积（字节）。与磁盘实际体积可能不一致，仅供参考。
    pub size: u64,
    /// 最后更新时间，Unix 秒。
    pub time_updated: i64,
}

/// 扫描需要的额外输入：已导入映射 + acf 安装记录。
///
/// `installed` 为 `None` 表示「acf 读不到」，此时所有条目的
/// [`SteamWallpaper::item_state`] 都是 `unknown` —— 不知道就别装作知道。
#[derive(Debug, Clone, Default)]
pub struct ScanContext {
    /// 启动器壁纸库里已有的：`workshop_id` → 壁纸包 id。
    pub imported: HashMap<String, String>,
    /// acf 里的安装记录：`workshop_id` → 安装信息。
    pub installed: Option<HashMap<String, AcfEntry>>,
}

// ---------------------------------------------------------------------------
// VDF 解析
// ---------------------------------------------------------------------------

/// VDF 解析失败。真实文件基本都是合法的，这两个分支防的是被截断 / 被喂垃圾。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VdfError {
    /// 引号或花括号没闭合。
    #[error("VDF 结构不完整")]
    Unterminated,
    /// 嵌套超过 [`MAX_VDF_DEPTH`] 层。
    #[error("VDF 嵌套过深")]
    TooDeep,
}

/// 一个 VDF 值：要么是标量字符串，要么是一组键值对（块）。
///
/// 刻意用 `Vec<(String, VdfValue)>` 而不是 `HashMap`：Steam 的文件顺序即语义
/// （`"0"`、`"1"` 下标就是库序号），而且体积很小，线性查找完全够用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdfValue {
    /// `"k" "v"`。
    Str(String),
    /// `"k" { … }`。
    Block(Vec<(String, VdfValue)>),
}

impl VdfValue {
    /// 标量内容；是块则 `None`。
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text.as_str()),
            Self::Block(_) => None,
        }
    }

    /// 子键值对；是标量则 `None`。
    pub fn as_block(&self) -> Option<&[(String, VdfValue)]> {
        match self {
            Self::Str(_) => None,
            Self::Block(pairs) => Some(pairs.as_slice()),
        }
    }

    /// 大小写不敏感地取一个键。VDF 的键大小写不统一（`LibraryFolders` vs
    /// `libraryfolders`），所以不能直接比字节。
    pub fn get(&self, key: &str) -> Option<&VdfValue> {
        self.as_block()?
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    /// 取一个键的标量值。
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(VdfValue::as_str)
    }

    /// 取一个键的块。
    pub fn get_block(&self, key: &str) -> Option<&[(String, VdfValue)]> {
        self.get(key).and_then(VdfValue::as_block)
    }
}

/// 词法单元。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// `{`
    Open,
    /// `}`
    Close,
    /// 带引号或不带引号的字符串
    Str(String),
}

/// 解析 VDF 文本，返回顶层键值对。
///
/// 只支持我们真正需要的那一丁点语法：`"k" "v"`、`"k" { … }`、`//` 行注释。
/// 额外的宽容之处（都能在真实 Steam 文件里遇到）：
///
/// * 裸 token（不带引号）当字符串 —— 老版本 `libraryfolders.vdf` 会这么写；
/// * `[$WIN32]` 这类平台条件标签整段跳过；
/// * 顶层多余的 `}` 跳过而不是报错。
pub fn parse_vdf(text: &str) -> Result<Vec<(String, VdfValue)>, VdfError> {
    let tokens = tokenize(text)?;
    let mut index = 0_usize;
    parse_block(&tokens, &mut index, 0)
}

/// 把文本切成词法单元。
fn tokenize(text: &str) -> Result<Vec<Token>, VdfError> {
    let bytes = text.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0_usize;

    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c' => index += 1,
            // 行注释：到行尾为止。Steam 的 vdf 里确实会写注释。
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'{' => {
                tokens.push(Token::Open);
                index += 1;
            }
            b'}' => {
                tokens.push(Token::Close);
                index += 1;
            }
            // 平台条件标签 `[$WIN32]`：不影响取值，整段丢弃。
            b'[' => {
                while index < bytes.len() && bytes[index] != b']' {
                    index += 1;
                }
                index = (index + 1).min(bytes.len());
            }
            b'"' => {
                let (value, next) = read_quoted(text, index)?;
                tokens.push(Token::Str(value));
                index = next;
            }
            _ => {
                let start = index;
                while index < bytes.len() {
                    let inner = bytes[index];
                    if inner.is_ascii_whitespace()
                        || inner == b'{'
                        || inner == b'}'
                        || inner == b'"'
                    {
                        break;
                    }
                    index += 1;
                }
                if index > start {
                    tokens.push(Token::Str(text[start..index].to_string()));
                } else {
                    // 理论上到不了（说明上面某个分支漏了字符）：前进一格防死循环。
                    index += 1;
                }
            }
        }
    }

    Ok(tokens)
}

/// 读一段带引号的字符串，返回（内容，闭合引号之后的下标）。
fn read_quoted(text: &str, start: usize) -> Result<(String, usize), VdfError> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut index = start + 1;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => return Ok((out, index + 1)),
            b'\\' => {
                let escaped = *bytes.get(index + 1).ok_or(VdfError::Unterminated)?;
                match escaped {
                    b'\\' => {
                        out.push('\\');
                        index += 2;
                    }
                    b'"' => {
                        out.push('"');
                        index += 2;
                    }
                    b'n' => {
                        out.push('\n');
                        index += 2;
                    }
                    b't' => {
                        out.push('\t');
                        index += 2;
                    }
                    b'r' => {
                        out.push('\r');
                        index += 2;
                    }
                    _ => {
                        // 不认识的转义：原样保留反斜杠与那个字符。
                        // 真实文件里 `"C:\Steam"` 这种写法并不罕见，丢掉反斜杠
                        // 会把路径毁掉，所以这里必须宽容。
                        let character = text[index + 1..]
                            .chars()
                            .next()
                            .ok_or(VdfError::Unterminated)?;
                        out.push('\\');
                        out.push(character);
                        index += 1 + character.len_utf8();
                    }
                }
            }
            _ => {
                let character = text[index..]
                    .chars()
                    .next()
                    .ok_or(VdfError::Unterminated)?;
                out.push(character);
                index += character.len_utf8();
            }
        }
    }

    Err(VdfError::Unterminated)
}

/// 递归下降：从 `*index` 开始读一组键值对，遇到 `}` 或输入结束为止。
fn parse_block(
    tokens: &[Token],
    index: &mut usize,
    depth: usize,
) -> Result<Vec<(String, VdfValue)>, VdfError> {
    if depth > MAX_VDF_DEPTH {
        return Err(VdfError::TooDeep);
    }

    let mut pairs: Vec<(String, VdfValue)> = Vec::new();
    while *index < tokens.len() {
        match &tokens[*index] {
            Token::Close => {
                *index += 1;
                if depth == 0 {
                    // 顶层多余的 `}`：跳过，不报错。
                    continue;
                }
                return Ok(pairs);
            }
            Token::Open => {
                // 无键的 `{`：没有地方挂，跳过。
                *index += 1;
            }
            Token::Str(key) => {
                let key = key.clone();
                *index += 1;
                match tokens.get(*index) {
                    Some(Token::Open) => {
                        *index += 1;
                        let block = parse_block(tokens, index, depth + 1)?;
                        pairs.push((key, VdfValue::Block(block)));
                    }
                    Some(Token::Str(value)) => {
                        let value = value.clone();
                        *index += 1;
                        pairs.push((key, VdfValue::Str(value)));
                    }
                    // `"k"` 后面既不是值也不是块：当成空串，`}` 留给外层处理。
                    Some(Token::Close) | None => {
                        pairs.push((key, VdfValue::Str(String::new())));
                    }
                }
            }
        }
    }

    if depth > 0 {
        // 块没闭合（文件被截断）。
        return Err(VdfError::Unterminated);
    }
    Ok(pairs)
}

// ---------------------------------------------------------------------------
// Steam 库定位
// ---------------------------------------------------------------------------

/// 从注册表读 Steam 安装目录。
///
/// 先看 `HKCU\Software\Valve\Steam\SteamPath`（Steam 自己写的，最准），
/// 再退回 `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath`（老机器 / 新用户
/// 还没跑过 Steam 时只有这一份）。读不到就是 `None`，不报错 ——
/// 「这台机器没装 Steam」是正常状态，不是故障。
#[cfg(windows)]
pub fn steam_root() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    fn read(
        hive: winreg::HKEY,
        path: &str,
        name: &str,
    ) -> Option<String> {
        let key = RegKey::predef(hive)
            .open_subkey_with_flags(path, KEY_READ)
            .ok()?;
        let value: String = key.get_value(name).ok()?;
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    let found = read(
        HKEY_CURRENT_USER,
        r"Software\Valve\Steam",
        "SteamPath",
    )
    .or_else(|| {
        read(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        )
    })
    .or_else(|| {
        read(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Valve\Steam",
            "InstallPath",
        )
    })?;

    let path = PathBuf::from(found);
    path.is_dir().then_some(path)
}

/// 非 Windows 平台没有注册表，也没有 Steam 的固定安装位置。
#[cfg(not(windows))]
pub fn steam_root() -> Option<PathBuf> {
    None
}

/// 解析 `libraryfolders.vdf` 文本，返回所有库根目录。
///
/// 两种历史格式都认：
///
/// ```text
/// "libraryfolders" { "0" { "path" "D:\\steam" } }   // 新格式
/// "LibraryFolders" { "1"   "D:\\SteamLibrary"    }   // 老格式（id → 路径）
/// ```
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    let Ok(pairs) = parse_vdf(text) else {
        return Vec::new();
    };
    let Some((_, root)) = pairs
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("libraryfolders"))
    else {
        return Vec::new();
    };
    let Some(entries) = root.as_block() else {
        return Vec::new();
    };

    let mut folders: Vec<PathBuf> = Vec::new();
    for (key, value) in entries {
        match value {
            // 新格式：`"0" { "path" "D:\\steam" "label" "" "apps" { … } }`。
            // **只**认 `path`：块里其它子键（`label`、`contentid`、`apps`）
            // 都不是库路径；没有 `path` 的块直接跳过。
            VdfValue::Block(_) => {
                if let Some(path) = value.get_str("path") {
                    push_folder(&mut folders, path);
                }
            }
            // 老格式：`"1" "D:\\SteamLibrary"` —— 键是库序号，值是路径。
            // 同一个块里还混着 `"TimeNextStatsReport" "1234"` 这种计数项，
            // 靠「键必须是纯数字」把它们排除掉（否则 "1234" 会被当成一个库）。
            VdfValue::Str(path) => {
                if is_library_index(key) {
                    push_folder(&mut folders, path);
                }
            }
        }
    }
    folders
}

/// 老格式里的库序号键：`"0"`、`"1"`、`"2"`…
fn is_library_index(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_digit())
}

fn push_folder(folders: &mut Vec<PathBuf>, raw: &str) {
    let trimmed = raw.trim();
    // 空串、以及纯数字（老格式里的计数项）都不是路径。
    if trimmed.is_empty()
        || trimmed.bytes().all(|byte| byte.is_ascii_digit())
    {
        return;
    }
    let path = PathBuf::from(trimmed);
    if folders.iter().any(|existing| existing == &path) {
        return;
    }
    folders.push(path);
}

/// 读出这台机器的所有 Steam 库根目录（`steam_root` 自己一定在里面）。
pub fn library_folders(steam_root: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = fs::read_to_string(vdf) {
        for folder in parse_library_folders(&text) {
            push_folder(&mut folders, &folder.to_string_lossy());
        }
    }
    folders
}

/// 每个库里是否存在 Wallpaper Engine 的工坊内容目录，存在就收进来。
pub fn workshop_roots(libraries: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for library in libraries {
        let candidate = library
            .join("steamapps")
            .join("workshop")
            .join("content")
            .join(WALLPAPER_ENGINE_APP_ID);
        if !candidate.is_dir() {
            continue;
        }
        let key = candidate.to_string_lossy().to_ascii_lowercase();
        if seen.insert(key) {
            roots.push(candidate);
        }
    }
    roots
}

/// 从任意一个库里读 `appworkshop_431960.acf`，解析出安装记录。
///
/// 返回 `None` 表示「哪个库都没读到 / 读到的不是合法 VDF」，
/// 此时扫描结果的 `item_state` 一律是 `unknown`。
pub fn read_acf(libraries: &[PathBuf]) -> Option<HashMap<String, AcfEntry>> {
    for library in libraries {
        let path = library
            .join("steamapps")
            .join("workshop")
            .join(format!("appworkshop_{WALLPAPER_ENGINE_APP_ID}.acf"));
        match fs::read_to_string(&path) {
            Ok(text) => return parse_acf(&text),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::warn!(%error, "读不了 Steam 工坊 acf，跳过这个库");
                continue;
            }
        }
    }
    None
}

/// 解析 acf 文本里的 `WorkshopItemsInstalled` 块。
pub fn parse_acf(text: &str) -> Option<HashMap<String, AcfEntry>> {
    let pairs = parse_vdf(text).ok()?;
    // 顶层只有一个键 `"AppWorkshop"`。
    let root = pairs
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("AppWorkshop"))
        .map(|(_, value)| value)?;
    let installed = root.get_block("WorkshopItemsInstalled")?;

    let mut map: HashMap<String, AcfEntry> = HashMap::new();
    for (id, value) in installed {
        if !is_valid_workshop_id(id) {
            continue;
        }
        let Some(block) = value.as_block() else {
            continue;
        };
        let entry = AcfEntry {
            size: field_u64(block, "size"),
            time_updated: field_i64(block, "timeupdated"),
        };
        map.insert(id.clone(), entry);
    }
    // 一个都没有时给 `None`：那和「acf 读不到」对界面是一个意思。
    (!map.is_empty()).then_some(map)
}

fn field_u64(block: &[(String, VdfValue)], name: &str) -> u64 {
    block
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn field_i64(block: &[(String, VdfValue)], name: &str) -> i64 {
    block
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
        .and_then(|text| text.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 扫描
// ---------------------------------------------------------------------------

/// 扫这台机器上所有 Steam 库里的 WE 工坊壁纸。
///
/// `store` 用来回答「哪些已经在启动器里了」（按 `workshop_id` 匹配）。
///
/// **无论有没有扫到东西，结果都会写进进程内的目录表**（[`remember`]）：
/// [`import`] 只认这张表，所以「扫描 → 导入」必须打到同一张表上。
/// 调用方不需要（也不应该）再自己登记一遍。
pub fn scan(store: &WallpaperStore) -> Result<SteamScan, SteamError> {
    let root = steam_root().ok_or(SteamError::SteamNotFound)?;
    let libraries = library_folders(&root);
    let roots = workshop_roots(&libraries);
    let installed = read_acf(&libraries);

    let context = ScanContext {
        imported: import_index(store),
        installed,
    };

    let mut scan = scan_roots(&roots, &context);
    scan.steam_root = Some(root.to_string_lossy().into_owned());
    // 先登记再返回：哪怕上面只扫到 0 条，也要把上一批的残留清掉。
    remember(&scan);
    Ok(scan)
}

/// 启动器壁纸库里已有的工坊包：`workshop_id` → 包 id。
pub fn import_index(store: &WallpaperStore) -> HashMap<String, String> {
    let mut index: HashMap<String, String> = HashMap::new();
    let Ok(items) = store.list() else {
        // 列表读不出来不影响扫描：界面最多不知道「已导入」。
        return index;
    };
    for meta in items {
        let workshop_id = meta.workshop_id.trim();
        if workshop_id.is_empty() {
            continue;
        }
        index.entry(workshop_id.to_string()).or_insert(meta.id);
    }
    index
}

/// 对一组 `workshop/content/431960` 目录做扫描。**纯函数**（除了读盘），
/// 单测直接喂临时目录。
pub fn scan_roots(roots: &[PathBuf], context: &ScanContext) -> SteamScan {
    let mut items: Vec<SteamWallpaper> = Vec::new();
    let mut skipped: Vec<SteamSkipped> = Vec::new();
    let mut scanned: Vec<String> = Vec::new();

    for root in roots {
        let read_dir = match fs::read_dir(root) {
            Ok(read_dir) => read_dir,
            Err(error) => {
                tracing::warn!(%error, "读不了 Steam 工坊目录");
                skipped.push(SteamSkipped {
                    dir: root.to_string_lossy().into_owned(),
                    reason: "读不了这个工坊目录".to_string(),
                });
                continue;
            }
        };

        let root_display = root.to_string_lossy().into_owned();
        if !scanned.contains(&root_display) {
            scanned.push(root_display);
        }

        for entry in read_dir {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(%error, "工坊目录里有一条读不出来，跳过");
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            // 只要目录；根下的散装文件（Steam 偶尔会留 .tmp）静默忽略。
            let is_dir = entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            if !is_valid_workshop_id(&name) {
                skipped.push(SteamSkipped {
                    dir: entry.path().to_string_lossy().into_owned(),
                    reason: "目录名不像工坊 id".to_string(),
                });
                continue;
            }

            match read_item(&entry.path(), &name, context) {
                Ok(item) => items.push(item),
                Err(reason) => skipped.push(SteamSkipped {
                    dir: entry.path().to_string_lossy().into_owned(),
                    reason,
                }),
            }
        }
    }

    sort_items(&mut items);
    SteamScan {
        items,
        skipped,
        roots: scanned,
        steam_root: None,
    }
}

/// 按标题（大小写不敏感）再按 id 排序，和 `WallpaperStore::list` 一致。
fn sort_items(items: &mut [SteamWallpaper]) {
    items.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.workshop_id.cmp(&right.workshop_id))
    });
}

/// 读单个工坊目录。失败时返回**原因**（中文单句），由调用方记进 `skipped`。
///
/// 判定顺序（前三条是「没有壁纸内容」的硬条件，第四条不是）：
///
/// 1. 没有 / 读不出 `project.json` → `缺少 project.json`；
/// 2. 有清单但 `file` 为空 → `project.json 没有入口文件`；
/// 3. `file` 指向的文件不存在，且目录里也没有 `scene.pkg`（打包场景的
///    正常形态）→ `入口文件不存在`；
/// 4. `type` 不是 scene/video（`application` / `other` / 未知）**保留**：
///    那是「不支持」而不是「不是壁纸」，前端要显示出来并标注，藏起来更糟。
fn read_item(
    dir: &Path,
    dir_name: &str,
    context: &ScanContext,
) -> Result<SteamWallpaper, String> {
    let project_path = dir.join(PROJECT_FILE_NAME);
    if let Ok(metadata) = fs::metadata(&project_path)
        && metadata.len() > MAX_PROJECT_BYTES
    {
        return Err("project.json 大得不像清单".to_string());
    }
    let text = fs::read_to_string(&project_path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "缺少 project.json".to_string()
        } else {
            "读不了 project.json".to_string()
        }
    })?;
    let project: Value = serde_json::from_str(&text)
        .map_err(|_| "project.json 不是合法 JSON".to_string())?;
    if !project.is_object() {
        return Err("project.json 不是对象".to_string());
    }

    // 只有「真的有入口」的目录才算壁纸。
    //
    // 工坊目录里会留下各种没有内容的壳：Steam 删不干净的残留、只下了一半的
    // 订阅、乃至用户手动塞进来的空文件夹。它们出现在「Steam 库」列表里
    // 只会让人以为能点，点下去才发现装不上。
    let entry = project
        .get("file")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if entry.is_empty() {
        return Err("project.json 没有入口文件".to_string());
    }
    // 打包形态的场景壁纸：`file` 写 `scene.json`，磁盘上只有 `scene.pkg`。
    // 这不是坏包，是真·工坊形态，必须放行。
    let entry_on_disk = safe_rel(entry)
        .map(|rel| dir.join(&rel).is_file())
        .unwrap_or(false);
    if !entry_on_disk && !dir.join(SCENE_PACKAGE_NAME).is_file() {
        return Err("入口文件不存在".to_string());
    }

    let workshop_id = workshop_id_of(&project, dir_name);
    let item_state = item_state_of(&workshop_id, context);
    let installed_at = context
        .installed
        .as_ref()
        .and_then(|installed| installed.get(&workshop_id))
        .map(|entry| entry.time_updated.saturating_mul(1000));

    Ok(SteamWallpaper {
        imported_id: context.imported.get(&workshop_id).cloned(),
        title: title_of(&project, dir_name),
        kind: kind_of(
            project
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .to_string(),
        preview: preview_of(dir, &project),
        bytes: store::dir_stat(dir).map(|stat| stat.bytes).unwrap_or(0),
        installed_at,
        item_state,
        workshop_id,
        dir: dir.to_string_lossy().into_owned(),
    })
}

/// 工坊 id：`project.json.workshopid` 优先，否则退回目录名。
fn workshop_id_of(project: &Value, dir_name: &str) -> String {
    let from_project = project
        .get("workshopid")
        .map(json_scalar)
        .unwrap_or_default();
    let from_project = from_project.trim();
    if is_valid_workshop_id(from_project) {
        from_project.to_string()
    } else {
        dir_name.to_string()
    }
}

/// `installed` / `orphan` / `unknown`。
fn item_state_of(workshop_id: &str, context: &ScanContext) -> String {
    match &context.installed {
        None => "unknown".to_string(),
        Some(installed) if installed.contains_key(workshop_id) => {
            "installed".to_string()
        }
        Some(_) => "orphan".to_string(),
    }
}

/// 标题：`project.json.title` 去空白后非空才用，否则退回目录名。
fn title_of(project: &Value, fallback: &str) -> String {
    project
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

/// `project.json.type` → 归一小写名字。真实数据里大小写混用。
fn kind_of(raw: &str) -> &'static str {
    WallpaperType::from_project_type(raw).as_str()
}

/// 缩略图：先信 `project.json.preview`，再按 [`PREVIEW_CANDIDATES`] 探测。
fn preview_of(dir: &Path, project: &Value) -> Option<String> {
    if let Some(raw) = project.get("preview").and_then(Value::as_str)
        && let Some(rel) = safe_rel(raw.trim())
        && is_preview_name(&rel)
        && dir.join(&rel).is_file()
    {
        return Some(rel);
    }
    PREVIEW_CANDIDATES
        .iter()
        .find(|candidate| dir.join(candidate).is_file())
        .map(|candidate| (*candidate).to_string())
}

fn is_preview_name(rel: &str) -> bool {
    let extension = Path::new(rel)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "gif" | "jpg" | "jpeg" | "png" | "webp"
    )
}

/// `project.json` 里的标量可能是字符串也可能是数字。
fn json_scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

/// 工坊 id 的形状：纯数字，1～[`MAX_WORKSHOP_ID_LEN`] 位。
///
/// 这条校验同时是路径白名单：id 会被拼进磁盘路径和取流 URL。
pub fn is_valid_workshop_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_WORKSHOP_ID_LEN
        && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// 从用户粘贴的内容里抽出工坊 id。**纯字符串处理，不联网。**
///
/// 认这些形态（大小写、首尾空格、`#comments` 之类都容错）：
///
/// ```text
/// 1960285432
/// https://steamcommunity.com/sharedfiles/filedetails/?id=1960285432
/// https://steamcommunity.com/workshop/filedetails/?id=1960285432&searchtext=x
/// steam://url/CommunityFilePage/1960285432
/// steamcommunity.com/sharedfiles/filedetails/?id=1960285432   // 少了 scheme 也收
/// ```
///
/// 只抽 id：真正「这个 id 本机有没有」由 [`remembered_dir`] 回答，
/// 所以这里不可能把用户导向任何网络请求。
pub fn workshop_id_from_input(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // `#comments` / `#` 之后不参与解析（`?id=123#comments` 很常见）。
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed).trim();
    if without_fragment.is_empty() {
        return None;
    }

    // 1. 纯 id。
    if is_valid_workshop_id(without_fragment) {
        return Some(without_fragment.to_string());
    }

    let lower = without_fragment.to_ascii_lowercase();

    // 2. `steam://url/CommunityFilePage/<id>`：最后一段就是 id。
    if let Some(rest) = lower.strip_prefix("steam://") {
        let last = rest.rsplit('/').next().unwrap_or_default();
        return is_valid_workshop_id(last).then(|| last.to_string());
    }

    // 3. `…steamcommunity.com/…?id=<id>`：从 query 里取 `id`。
    if lower.contains("steamcommunity.com") {
        let query = without_fragment.split_once('?')?.1;
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case("id")
                && is_valid_workshop_id(value.trim())
            {
                return Some(value.trim().to_string());
            }
        }
        return None;
    }

    None
}

/// 给用户看的「在 Steam 里打开这个条目」链接。
///
/// 只有在 [`is_valid_workshop_id`] 通过时才拼得出来，所以调用方拿到 `Some`
/// 就可以直接交给浏览器 —— 里面不可能塞进别的东西。
pub fn workshop_page_url(workshop_id: &str) -> Option<String> {
    is_valid_workshop_id(workshop_id).then(|| {
        format!(
            "https://steamcommunity.com/sharedfiles/filedetails/?id={workshop_id}"
        )
    })
}

// ---------------------------------------------------------------------------
// 进程内的「最近一次扫描结果」
// ---------------------------------------------------------------------------

/// `workshop_id` → 目录。只由 [`remember`] 写，只由 [`remembered_dir`] 读。
///
/// 用途只有一个：`wallpaper_steam_import(workshop_id)` 必须能证明这个 id
/// 来自本进程刚扫过的目录，而不是前端随手编的一个路径。
fn remembered() -> &'static Mutex<HashMap<String, PathBuf>> {
    static REMEMBERED: OnceLock<Mutex<HashMap<String, PathBuf>>> =
        OnceLock::new();
    REMEMBERED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 锁住目录表。互斥锁中毒不该让功能整体失效，所以直接取回内部数据。
fn lock_remembered() -> std::sync::MutexGuard<'static, HashMap<String, PathBuf>>
{
    remembered()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 把一次扫描结果记进目录表（覆盖上一次）。
pub fn remember(scan: &SteamScan) {
    let mut table = lock_remembered();
    table.clear();
    for item in &scan.items {
        table.insert(item.workshop_id.clone(), PathBuf::from(&item.dir));
    }
}

/// 最近一次扫描结果里这个工坊 id 对应的目录。
pub fn remembered_dir(workshop_id: &str) -> Option<PathBuf> {
    if !is_valid_workshop_id(workshop_id) {
        return None;
    }
    lock_remembered().get(workshop_id).cloned()
}

/// 清空目录表。目前只有单测用得到（避免测试之间互相污染）。
pub fn forget_all() {
    lock_remembered().clear();
}

/// 摘掉一条。目录已经不在时用，免得表里留一个指向空气的条目。
fn forget_entry(workshop_id: &str) {
    lock_remembered().remove(workshop_id);
}

// ---------------------------------------------------------------------------
// 导入
// ---------------------------------------------------------------------------

/// 导入一个**已经扫到**的工坊壁纸。
///
/// `workshop_id` 必须先出现在本进程最近一次 [`remember`] 的目录表里；
/// 否则返回 [`SteamError::NotScanned`]。这是「不许任意路径读盘」的唯一闸门，
/// 所以不要绕开 [`remembered_dir`] 直接拼路径。
pub fn import(
    store: &WallpaperStore,
    workshop_id: &str,
) -> Result<ImportOutcome, SteamError> {
    let dir = remembered_dir(workshop_id).ok_or(SteamError::NotScanned)?;
    if !dir.is_dir() {
        // 扫描时还在，现在没了：用户在 Steam 里取消了订阅。
        // 顺手把这条从目录表里摘掉，表里不留指向空气的条目。
        forget_entry(workshop_id);
        return Err(SteamError::SourceGone);
    }
    store
        .import(
            &dir,
            &ImportOptions {
                workshop_id: Some(workshop_id.to_string()),
            },
        )
        .map_err(SteamError::Import)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::store::WallpaperStore;

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, bytes).expect("write");
    }

    /// 进程内的目录表（[`remember`] / [`forget_all`]）是**全局**状态，
    /// 动它的测试必须串行：否则一条测试的 `forget_all()` 会把另一条刚
    /// `remember()` 的结果清掉 —— 并行跑测试时这是一次真实竞态
    /// （表现为 `meta.workshop_id` 为空、或 `expect_err` 拿到 `Ok`）。
    fn global_table_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: Mutex<()> = Mutex::new(());
        GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 一个最小的场景类工坊目录。
    fn scene_entry(root: &Path, id: &str, title: &str) {
        write(
            &root.join(id).join("project.json"),
            format!(
                r#"{{"title":"{title}","type":"Scene","file":"scene.json","preview":"preview.gif"}}"#
            )
            .as_bytes(),
        );
        write(&root.join(id).join("scene.json"), b"{\"objects\":[]}");
        write(&root.join(id).join("preview.gif"), b"GIF89a-preview");
    }

    // -- VDF -------------------------------------------------------------

    /// 真实形状：顶层 `"libraryfolders"` 包着编号块，注释、空格路径都有。
    const LIBRARY_FOLDERS_VDF: &str = r#"
// 这是注释，必须跳过
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"	// 行尾注释
		"label"		""
		"contentid"		"7206063193373742935"
		"apps"
		{
			"431960"		"826275581"
		}
	}
	"1"
	{
		"path"		"D:\\Steam Library"
		"label"		"第二块盘"
	}
}
"#;

    #[test]
    fn parses_nested_vdf_with_comments_and_paths_with_spaces() {
        let pairs = parse_vdf(LIBRARY_FOLDERS_VDF).expect("parse");
        assert_eq!(pairs.len(), 1);
        let (key, root) = &pairs[0];
        assert_eq!(key, "libraryfolders");

        let entries = root.as_block().expect("block");
        assert_eq!(entries.len(), 2, "两个库");

        let first = &entries[0].1;
        assert_eq!(
            first.get_str("path"),
            Some(r"C:\Program Files (x86)\Steam"),
            "带空格的路径要完整保留"
        );
        assert_eq!(first.get_str("label"), Some(""));
        // 三层：块里还有块（apps 里是标量）。
        assert_eq!(
            first.get_block("apps").expect("apps")[0].0.as_str(),
            "431960"
        );
        let second = &entries[1].1;
        assert_eq!(second.get_str("path"), Some(r"D:\Steam Library"));
        assert_eq!(second.get_str("label"), Some("第二块盘"));
    }

    #[test]
    fn vdf_keys_are_matched_case_insensitively() {
        let pairs = parse_vdf(r#""LibraryFolders" { "1" "D:\\Lib" }"#).unwrap();
        let (_, root) = &pairs[0];
        assert_eq!(root.get_str("path"), None);
        // 老格式：id → 路径直接是标量。
        assert_eq!(root.get_block("1"), None);
        assert!(matches!(root.get("1"), Some(VdfValue::Str(value)) if value == r"D:\Lib"));
    }

    #[test]
    fn vdf_parser_reports_unterminated_input_and_keeps_bare_tokens() {
        assert_eq!(
            parse_vdf(r#""a" { "b" "c""#).expect_err("must fail"),
            VdfError::Unterminated
        );
        assert_eq!(
            parse_vdf(r#""a" "no closing quote"#).expect_err("must fail"),
            VdfError::Unterminated
        );
        // 裸 token 当字符串（老 vdf 会这么写）。
        let pairs = parse_vdf("key value").expect("parse");
        assert_eq!(pairs[0].1.as_str(), Some("value"));
    }

    #[test]
    fn vdf_parser_survives_a_deeply_nested_file() {
        let mut text = String::new();
        for index in 0..(MAX_VDF_DEPTH + 4) {
            text.push_str(&format!("\"k{index}\" {{ "));
        }
        for _ in 0..(MAX_VDF_DEPTH + 4) {
            text.push('}');
        }
        assert_eq!(parse_vdf(&text), Err(VdfError::TooDeep));
    }

    #[test]
    fn vdf_keeps_the_backslash_of_an_unknown_escape() {
        // `\S`、`\L` 不是合法转义；丢掉反斜杠会把路径毁掉。
        let pairs = parse_vdf(r#""path" "D:\SteamLibrary""#).expect("parse");
        assert_eq!(pairs[0].1.as_str(), Some(r"D:\SteamLibrary"));
    }

    #[test]
    fn library_folders_supports_both_file_formats() {
        let folders = parse_library_folders(LIBRARY_FOLDERS_VDF);
        assert_eq!(
            folders,
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\Steam Library"),
            ]
        );

        let legacy = r#"
"LibraryFolders"
{
	"TimeNextStatsReport"		"1234"
	"1"		"E:\\Games\\SteamLibrary"
}
"#;
        assert_eq!(
            parse_library_folders(legacy),
            vec![PathBuf::from(r"E:\Games\SteamLibrary")]
        );

        // 垃圾输入不许 panic。
        assert!(parse_library_folders("{").is_empty());
        assert!(parse_library_folders("").is_empty());
    }

    #[test]
    fn numeric_vdf_keys_are_not_mistaken_for_library_paths() {
        // 回归：块里只有 `path` 是库；老格式里 `"TimeNextStatsReport" "1234"`
        // 这种计数项曾被当成一个库路径，导致库里多出一条 "1234"。
        let mixed = r#"
"libraryfolders"
{
	"1234"
	{
		"path"		"E:\\Games\\SteamLibrary"
	}
	"5678"
	{
		"label"		"没有 path 的块"
	}
	"9999"		"4321"
}
"#;
        assert_eq!(
            parse_library_folders(mixed),
            vec![PathBuf::from(r"E:\Games\SteamLibrary")],
            "一个库都不该少，一条数字也不该多"
        );

        // 老格式：只有数字键的条目才是库。
        let legacy = r#"
"LibraryFolders"
{
	"TimeNextStatsReport"		"1234"
	"contentstatsid"		"77"
	"0"		"D:\\steam"
	"1"		"E:\\Games\\SteamLibrary"
}
"#;
        assert_eq!(
            parse_library_folders(legacy),
            vec![
                PathBuf::from(r"D:\steam"),
                PathBuf::from(r"E:\Games\SteamLibrary"),
            ]
        );
    }

    #[test]
    fn workshop_roots_only_keep_libraries_that_have_the_content_dir() {
        let temp = tempfile::tempdir().expect("temp");
        let with = temp.path().join("with-workshop");
        let without = temp.path().join("without-workshop");
        fs::create_dir_all(
            with.join("steamapps").join("workshop").join("content").join("431960"),
        )
        .expect("mkdir");
        fs::create_dir_all(without.join("steamapps")).expect("mkdir");

        let roots = workshop_roots(&[without.clone(), with.clone()]);
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0],
            with.join("steamapps").join("workshop").join("content").join("431960")
        );

        // 同一个库重复给两次也只算一个。
        assert_eq!(workshop_roots(&[with.clone(), with]).len(), 1);
    }

    // -- acf -------------------------------------------------------------

    const ACF: &str = r#"
"AppWorkshop"
{
	"appid"		"431960"
	"SizeOnDisk"		"1886162383"
	"WorkshopItemsInstalled"
	{
		"827982449"
		{
			"size"		"2169242"
			"timeupdated"		"1484103069"
			"manifest"		"4233635302081945563"
		}
		"2960476556"
		{
			"size"		"123456"
			"timeupdated"		"1792297489"
			"manifest"		"1"
		}
	}
	"WorkshopItemDetails"
	{
		"827982449"
		{
			"manifest"		"4233635302081945563"
			"timetouched"		"1792170648"
		}
	}
}
"#;

    #[test]
    fn parses_installed_items_out_of_the_acf() {
        let installed = parse_acf(ACF).expect("acf");
        assert_eq!(installed.len(), 2, "只认 WorkshopItemsInstalled");
        let entry = installed.get("827982449").expect("entry");
        assert_eq!(entry.size, 2_169_242);
        assert_eq!(entry.time_updated, 1_484_103_069);
        assert_eq!(installed["2960476556"].time_updated, 1_792_297_489);
    }

    #[test]
    fn a_broken_acf_yields_nothing_instead_of_panicking() {
        assert!(parse_acf("").is_none());
        assert!(parse_acf("\"AppWorkshop\" {").is_none());
        assert!(parse_acf("\"AppWorkshop\" { \"WorkshopItemsInstalled\" { } }").is_none());
    }

    // -- 归一化 -----------------------------------------------------------

    #[test]
    fn normalises_wallpaper_types_case_insensitively() {
        for raw in ["scene", "Scene", "SCENE", " scene ", "sCeNe"] {
            assert_eq!(kind_of(raw), "scene", "{raw:?}");
        }
        for raw in ["video", "Video", "VIDEO"] {
            assert_eq!(kind_of(raw), "video", "{raw:?}");
        }
        assert_eq!(kind_of("web"), "web");
        assert_eq!(kind_of("Web"), "web");
        assert_eq!(kind_of("application"), "application");
        // 未知值 / 缺字段统一落到 other。
        assert_eq!(kind_of(""), "other");
        assert_eq!(kind_of("plantype"), "other");
    }

    #[test]
    fn workshop_id_falls_back_to_the_directory_name() {
        // 实测 65 个条目里 24 个没有这个字段。
        assert_eq!(workshop_id_of(&serde_json::json!({"title": "x"}), "1375031246"), "1375031246");
        // 有就用它（可能是数字也可能是字符串）。
        assert_eq!(
            workshop_id_of(&serde_json::json!({"workshopid": "1234567890"}), "1"),
            "1234567890"
        );
        assert_eq!(
            workshop_id_of(&serde_json::json!({"workshopid": 987654321_u64}), "1"),
            "987654321"
        );
        // 形状不对（非数字 / 空串）也要退回目录名，不能拿去拼路径。
        assert_eq!(workshop_id_of(&serde_json::json!({"workshopid": ""}), "42"), "42");
        assert_eq!(
            workshop_id_of(&serde_json::json!({"workshopid": "../../etc"}), "42"),
            "42"
        );
    }

    #[test]
    fn workshop_ids_must_be_all_digits() {
        assert!(is_valid_workshop_id("431960"));
        assert!(is_valid_workshop_id("2960476556"));
        for bad in [
            "",
            "..",
            "123456789012345678901",
            "abc",
            "12a",
            " 12",
            "12 ",
            "1/2",
            "1\\2",
            "C:12",
        ] {
            assert!(!is_valid_workshop_id(bad), "{bad:?} 不该通过");
        }
    }

    // -- 粘贴链接 → 工坊 id -----------------------------------------------

    static ID: &str = "1960285432";

    #[test]
    fn extracts_a_workshop_id_from_every_link_shape() {
        // 1. 纯 id（用户从 Steam 里复制的那串数字）。
        assert_eq!(workshop_id_from_input(ID).as_deref(), Some(ID));
        assert_eq!(
            workshop_id_from_input(&format!("  {ID}\n")).as_deref(),
            Some(ID)
        );

        // 2. 分享页 / 创意工坊页面。
        for link in [
            format!("https://steamcommunity.com/sharedfiles/filedetails/?id={ID}"),
            format!("https://steamcommunity.com/workshop/filedetails/?id={ID}"),
            format!("http://steamcommunity.com/sharedfiles/filedetails/?id={ID}"),
            // 少了 scheme 也收（有人是从地址栏里拖出来的）。
            format!("steamcommunity.com/sharedfiles/filedetails/?id={ID}"),
            // 带别的 query 参数。
            format!(
                "https://steamcommunity.com/sharedfiles/filedetails/?id={ID}&searchtext=wallpaper"
            ),
            // 带 fragment。
            format!(
                "https://steamcommunity.com/sharedfiles/filedetails/?id={ID}#comments"
            ),
            // 参数顺序换一下。
            format!(
                "https://steamcommunity.com/sharedfiles/filedetails/?searchtext=x&id={ID}"
            ),
            // steam:// 深链。
            format!("steam://url/CommunityFilePage/{ID}"),
            format!("steam://url/communityfilepage/{ID}"),
        ] {
            assert_eq!(
                workshop_id_from_input(&link).as_deref(),
                Some(ID),
                "{link} 该解析出 {ID}"
            );
        }
    }

    #[test]
    fn rejects_everything_that_is_not_a_workshop_link() {
        for bad in [
            "",
            "   ",
            "https://steamcommunity.com/",
            "https://steamcommunity.com/sharedfiles/filedetails/",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=abc",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=12a",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=1;2",
            "https://example.com/?id=1960285432",
            "steam://url/CommunityFilePage/",
            "steam://url/CommunityFilePage/abc",
            "steam://url/CommunityFilePage/123456789012345678901",
            "../1960285432",
            "1960285432/../../etc",
            "id=1960285432",
        ] {
            assert!(
                workshop_id_from_input(bad).is_none(),
                "{bad:?} 不该解析出 id"
            );
        }
    }

    #[test]
    fn the_open_in_steam_link_only_exists_for_a_valid_id() {
        assert_eq!(
            workshop_page_url(ID).as_deref(),
            Some(
                "https://steamcommunity.com/sharedfiles/filedetails/?id=1960285432"
            )
        );
        for bad in ["", "abc", "../x", "1/2"] {
            assert!(workshop_page_url(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_pasted_link_round_trips_through_the_id_parser() {
        // 前端要做「在 Steam 中打开」时用的是 id；这条保证两头的格式对得上。
        let id = workshop_id_from_input(&format!(
            "https://steamcommunity.com/sharedfiles/filedetails/?id={ID}"
        ))
        .expect("id");
        let page = workshop_page_url(&id).expect("url");
        assert_eq!(workshop_id_from_input(&page).as_deref(), Some(ID));
    }

    #[test]
    fn item_state_distinguishes_installed_orphan_and_unknown() {
        let installed = HashMap::from([(
            "1001".to_string(),
            AcfEntry {
                size: 1,
                time_updated: 1,
            },
        )]);
        let with_acf = ScanContext {
            imported: HashMap::new(),
            installed: Some(installed),
        };
        assert_eq!(item_state_of("1001", &with_acf), "installed");
        assert_eq!(item_state_of("1002", &with_acf), "orphan");

        let no_acf = ScanContext::default();
        assert_eq!(item_state_of("1001", &no_acf), "unknown");
    }

    #[test]
    fn preview_prefers_the_project_field_then_probes_known_names() {
        let temp = tempfile::tempdir().expect("temp");
        let dir = temp.path();

        // 什么都没有 ⇒ None。
        assert_eq!(preview_of(dir, &serde_json::json!({})), None);

        // 只有 jpg：探测命中。
        write(&dir.join("preview.jpg"), b"\xff\xd8\xff");
        assert_eq!(
            preview_of(dir, &serde_json::json!({})),
            Some("preview.jpg".to_string())
        );

        // project.json 指了 gif 且存在 ⇒ 用它（动图优先）。
        write(&dir.join("preview.gif"), b"GIF89a");
        assert_eq!(
            preview_of(dir, &serde_json::json!({"preview": "preview.gif"})),
            Some("preview.gif".to_string())
        );

        // project.json 指了越界路径 / 不存在的文件 / 非图片 ⇒ 忽略，退回探测。
        for bad in [
            serde_json::json!({"preview": "../secret.gif"}),
            serde_json::json!({"preview": "missing.gif"}),
            serde_json::json!({"preview": "scene.json"}),
        ] {
            assert_eq!(
                preview_of(dir, &bad),
                Some("preview.gif".to_string()),
                "{bad:?}"
            );
        }
    }

    // -- 扫描 -------------------------------------------------------------

    #[test]
    fn scans_workshop_entries_and_counts_skipped() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        // 1 个 scene（type 写成大写、没有 workshopid）
        scene_entry(&root, "1001", "river");
        // 1 个 video
        write(
            &root.join("1002").join("project.json"),
            br#"{"title":"beach","type":"Video","file":"beach.mp4"}"#,
        );
        write(&root.join("1002").join("beach.mp4"), b"\x00\x00\x00\x18ftypmp42");
        // 1 个坏 json
        write(&root.join("1003").join("project.json"), b"{not json");
        // 1 个不是工坊 id 的目录 + 1 个散装文件
        fs::create_dir_all(root.join("not-an-id")).expect("mkdir");
        write(&root.join("appworkshop.tmp"), b"x");

        let scan = scan_roots(&[root.clone()], &ScanContext::default());

        assert_eq!(scan.items.len(), 2, "坏 json 那条要被跳过");
        assert_eq!(scan.skipped.len(), 2, "坏 json + 非 id 目录");
        assert_eq!(scan.roots.len(), 1);
        assert_eq!(scan.steam_root, None);

        let river = scan
            .items
            .iter()
            .find(|item| item.workshop_id == "1001")
            .expect("1001");
        assert_eq!(river.title, "river");
        assert_eq!(river.kind, "scene", "Scene 要归一化成 scene");
        assert_eq!(river.preview.as_deref(), Some("preview.gif"));
        assert_eq!(river.item_state, "unknown", "没有 acf 就是 unknown");
        assert_eq!(river.installed_at, None);
        assert_eq!(river.imported_id, None);
        assert!(river.bytes > 0, "目录体积要算出来");
        assert!(river.dir.ends_with("1001"));

        let beach = scan
            .items
            .iter()
            .find(|item| item.workshop_id == "1002")
            .expect("1002");
        assert_eq!(beach.kind, "video");
        assert_eq!(beach.preview, None, "没有缩略图就是 None");

        // 排序：beach < river。
        assert_eq!(scan.items[0].workshop_id, "1002");

        let reasons: Vec<&str> = scan
            .skipped
            .iter()
            .map(|item| item.reason.as_str())
            .collect();
        assert!(
            reasons.iter().any(|reason| reason.contains("不是合法 JSON")),
            "坏 json 的原因要写清楚：{reasons:?}"
        );
        assert!(
            reasons.iter().any(|reason| reason.contains("工坊 id")),
            "{reasons:?}"
        );
    }

    #[test]
    fn scan_marks_installed_and_imported_entries() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        scene_entry(&root, "1001", "river");
        scene_entry(&root, "1002", "beach");

        let context = ScanContext {
            imported: HashMap::from([(
                "1001".to_string(),
                "ws-1001-0123456789ab".to_string(),
            )]),
            installed: Some(HashMap::from([(
                "1002".to_string(),
                AcfEntry {
                    size: 10,
                    time_updated: 1_700_000_000,
                },
            )])),
        };

        let scan = scan_roots(&[root], &context);
        assert!(scan.skipped.is_empty());

        let river = scan
            .items
            .iter()
            .find(|item| item.workshop_id == "1001")
            .expect("1001");
        assert_eq!(river.item_state, "orphan", "acf 里没有它");
        assert_eq!(river.imported_id.as_deref(), Some("ws-1001-0123456789ab"));

        let beach = scan
            .items
            .iter()
            .find(|item| item.workshop_id == "1002")
            .expect("1002");
        assert_eq!(beach.item_state, "installed");
        assert_eq!(beach.imported_id, None);
        // acf 是秒，这里统一成毫秒。
        assert_eq!(beach.installed_at, Some(1_700_000_000_000));
    }

    #[test]
    fn a_missing_project_json_is_skipped_not_fatal() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        scene_entry(&root, "1001", "river");
        write(&root.join("1002").join("scene.pkg"), b"PKGV0024");

        let scan = scan_roots(&[root], &ScanContext::default());

        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.skipped.len(), 1);
        assert_eq!(scan.skipped[0].reason, "缺少 project.json");
    }

    #[test]
    fn a_non_object_project_json_is_skipped() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        write(&root.join("1001").join("project.json"), b"[1,2,3]");

        let scan = scan_roots(&[root], &ScanContext::default());
        assert!(scan.items.is_empty());
        assert_eq!(scan.skipped[0].reason, "project.json 不是对象");
    }

    #[test]
    fn skips_directories_without_any_renderable_content() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");

        // 1) `file` 是空串。
        write(
            &root.join("2001").join("project.json"),
            br#"{"title":"empty","type":"scene","file":""}"#,
        );
        // 2) 连 `file` 字段都没有。
        write(
            &root.join("2002").join("project.json"),
            br#"{"title":"nofile","type":"scene"}"#,
        );
        // 3) 入口指向不存在的文件，且没有 scene.pkg：这是空壳。
        write(
            &root.join("2003").join("project.json"),
            br#"{"title":"dangling","type":"video","file":"gone.mp4"}"#,
        );

        // 4) 打包形态的场景：`file` 写 scene.json 但磁盘上只有 scene.pkg。
        //    本机 65 个目录里 48 个长这样，**必须**保留。
        write(
            &root.join("2004").join("project.json"),
            br#"{"title":"packed","type":"scene","file":"scene.json"}"#,
        );
        write(&root.join("2004").join("scene.pkg"), b"PKGV0024");

        // 5) 不认识的 type，但有真入口 ⇒ 保留（只是「不支持」不是「不是壁纸」）。
        write(
            &root.join("2005").join("project.json"),
            br#"{"title":"weird","type":"plantype","file":"a.json"}"#,
        );
        write(&root.join("2005").join("a.json"), b"{}");

        let scan = scan_roots(&[root], &ScanContext::default());

        assert_eq!(
            scan.items
                .iter()
                .map(|item| item.workshop_id.as_str())
                .collect::<Vec<_>>(),
            ["2004", "2005"],
            "空壳目录要被过滤，打包场景和未知 type 要留下"
        );
        assert_eq!(
            scan.items[0].kind, "scene",
            "scene.pkg 就是打包场景的入口"
        );
        assert_eq!(
            scan.items[1].kind, "other",
            "未知 type 归一成 other，但不能被过滤"
        );

        assert_eq!(scan.skipped.len(), 3);
        let mut reasons: Vec<&str> =
            scan.skipped.iter().map(|item| item.reason.as_str()).collect();
        reasons.sort_unstable();
        assert_eq!(
            reasons,
            [
                "project.json 没有入口文件",
                "project.json 没有入口文件",
                "入口文件不存在",
            ]
        );
        // 跳过的是哪三条也要对得上，不能是「碰巧数量一样」。
        let mut skipped_ids: Vec<String> = scan
            .skipped
            .iter()
            .filter_map(|item| {
                Path::new(&item.dir)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        skipped_ids.sort();
        assert_eq!(skipped_ids, ["2001", "2002", "2003"]);
    }

    #[test]
    fn an_entry_point_outside_the_directory_does_not_count_as_content() {
        // `file: "../../shared.json"`：文件确实存在，但不在这个条目目录里，
        // 所以既不算「有内容」，也不该被当成可读入口（safe_rel 先挡一道）。
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        write(&temp.path().join("shared.json"), b"{\"objects\":[]}");
        write(
            &root.join("2001").join("project.json"),
            br#"{"title":"escape","type":"scene","file":"../../shared.json"}"#,
        );

        let scan = scan_roots(&[root], &ScanContext::default());
        assert!(scan.items.is_empty(), "越界入口不能被当成有效壁纸");
        assert_eq!(scan.skipped.len(), 1);
        assert_eq!(scan.skipped[0].reason, "入口文件不存在");
    }

    #[test]
    fn scan_never_panics_on_a_missing_root() {
        let scan = scan_roots(
            &[PathBuf::from("D:/definitely/not/here/431960")],
            &ScanContext::default(),
        );
        assert!(scan.items.is_empty());
        assert_eq!(scan.skipped.len(), 1);
    }

    #[test]
    fn kind_counts_groups_the_items() {
        let scan = SteamScan {
            items: vec![
                SteamWallpaper {
                    workshop_id: "1".to_string(),
                    dir: String::new(),
                    title: "a".to_string(),
                    kind: "scene".to_string(),
                    preview: None,
                    bytes: 0,
                    installed_at: None,
                    item_state: "installed".to_string(),
                    imported_id: None,
                },
                SteamWallpaper {
                    workshop_id: "2".to_string(),
                    dir: String::new(),
                    title: "b".to_string(),
                    kind: "scene".to_string(),
                    preview: None,
                    bytes: 0,
                    installed_at: None,
                    item_state: "installed".to_string(),
                    imported_id: None,
                },
            ],
            ..SteamScan::default()
        };
        assert_eq!(scan.kind_counts().get("scene"), Some(&2));
    }

    // -- 导入校验 ---------------------------------------------------------

    #[test]
    fn import_only_accepts_ids_from_the_last_scan() {
        // 这条测试独占进程内的目录表，所以要串行 + 开头结尾各清一遍。
        let _guard = global_table_guard();
        forget_all();

        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        scene_entry(&root, "1001", "river");
        let scan = scan_roots(&[root], &ScanContext::default());
        remember(&scan);

        assert!(remembered_dir("1001").is_some());
        // 形状不对的、以及形状对但没扫到的，都必须拒绝。
        for fake in ["1002", "", "..", "../etc", "1001/../..", "abc"] {
            assert!(
                remembered_dir(fake).is_none(),
                "{fake:?} 不该出现在目录表里"
            );
        }

        let store = WallpaperStore::new(temp.path().join("wallpapers"));
        store.ensure_dirs().expect("ensure dirs");

        let error = import(&store, "1002").expect_err("必须拒绝没扫到的 id");
        assert!(matches!(error, SteamError::NotScanned));
        assert_eq!(
            error.user_message(),
            "这个壁纸不在最近一次扫描结果里，请重新扫描"
        );
        assert!(store.list().expect("list").is_empty(), "拒绝时不能落盘");

        // 扫到的那条能正常导入，并且落在工坊命名空间里。
        let outcome = import(&store, "1001").expect("import");
        assert!(!outcome.already_installed);
        assert!(outcome.meta.id.starts_with("ws-1001-"), "{}", outcome.meta.id);
        assert_eq!(outcome.meta.workshop_id, "1001");
        assert_eq!(outcome.meta.title, "river");

        // 二次导入命中内容哈希，不多占盘。
        let again = import(&store, "1001").expect("re-import");
        assert!(again.already_installed);
        assert_eq!(store.list().expect("list").len(), 1);

        forget_all();
        assert!(remembered_dir("1001").is_none());
    }

    #[test]
    fn import_reports_a_directory_that_vanished_after_the_scan() {
        let _guard = global_table_guard();
        forget_all();
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("431960");
        scene_entry(&root, "1001", "river");
        let scan = scan_roots(&[root.clone()], &ScanContext::default());
        remember(&scan);
        assert!(remembered_dir("1001").is_some());

        fs::remove_dir_all(root.join("1001")).expect("rm");

        let store = WallpaperStore::new(temp.path().join("wallpapers"));
        let error = import(&store, "1001").expect_err("必须报错");
        assert!(matches!(error, SteamError::SourceGone));
        assert!(!error.user_message().contains(":\\"));
        // 表里也不该留一个指向空气的条目：再导入要重新扫描。
        assert!(remembered_dir("1001").is_none());
        assert!(matches!(
            import(&store, "1001").expect_err("必须报错"),
            SteamError::NotScanned
        ));

        forget_all();
    }

    #[test]
    fn import_index_reads_workshop_ids_from_the_store() {
        let temp = tempfile::tempdir().expect("temp");
        let store = WallpaperStore::new(temp.path().join("wallpapers"));
        store.ensure_dirs().expect("ensure dirs");

        let source = temp.path().join("entry");
        write(
            &source.join("project.json"),
            br#"{"title":"river","type":"scene","file":"scene.json"}"#,
        );
        write(&source.join("scene.json"), b"{\"objects\":[]}");
        let outcome = store
            .import(
                &source,
                &ImportOptions {
                    workshop_id: Some("1001".to_string()),
                },
            )
            .expect("import");

        let index = import_index(&store);
        assert_eq!(index.get("1001"), Some(&outcome.meta.id));

        // 本地包（没有工坊 id）不该进索引。
        let local = temp.path().join("local");
        write(
            &local.join("project.json"),
            br#"{"title":"local","type":"scene","file":"scene.json"}"#,
        );
        write(&local.join("scene.json"), b"{\"objects\":[]}");
        store.import(&local, &ImportOptions::default()).expect("import local");
        assert_eq!(import_index(&store).len(), 1);
    }

    /// 诊断用：把**这台机器**上真实的扫描结果打出来（不进常规测试）。
    ///
    /// ```text
    /// cargo test -p theseus --release steam -- --ignored --nocapture
    /// ```
    ///
    /// 需要本机真的装了 Steam + Wallpaper Engine，所以默认忽略。
    /// 这条同时是对真实 `libraryfolders.vdf` / `appworkshop_431960.acf`
    /// 的一次端到端回归 —— 那两个文件的格式只会由 Valve 来改。
    #[test]
    #[ignore = "需要本机真的装了 Steam + Wallpaper Engine"]
    fn prints_the_real_machine_scan() {
        let _guard = global_table_guard();
        forget_all();

        let temp = tempfile::tempdir().expect("temp");
        let store = WallpaperStore::new(temp.path().join("wallpapers"));
        store.ensure_dirs().expect("ensure dirs");

        let scan = match scan(&store) {
            Ok(scan) => scan,
            Err(error) => {
                println!("这台机器上没有 Steam：{}", error.user_message());
                return;
            }
        };

        println!("steam root : {:?}", scan.steam_root);
        println!("roots      : {:?}", scan.roots);
        println!(
            "items      : {} / skipped: {}",
            scan.items.len(),
            scan.skipped.len()
        );
        let mut kinds: Vec<(String, usize)> =
            scan.kind_counts().into_iter().collect();
        kinds.sort();
        println!("kinds      : {kinds:?}");
        println!(
            "installed  : {}",
            scan.items
                .iter()
                .filter(|item| item.item_state == "installed")
                .count()
        );
        println!(
            "orphan     : {}",
            scan.items
                .iter()
                .filter(|item| item.item_state == "orphan")
                .count()
        );
        println!(
            "with prev  : {}",
            scan.items.iter().filter(|item| item.preview.is_some()).count()
        );
        for item in &scan.items {
            println!(
                "  {:<12} {:<11} {:<9} {:>10}  {}",
                item.workshop_id, item.kind, item.item_state, item.bytes, item.title
            );
        }
        for skip in &scan.skipped {
            println!("  SKIPPED {} :: {}", skip.dir, skip.reason);
        }

        forget_all();
    }
}
