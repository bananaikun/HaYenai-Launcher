//! `PKGM0019`（容器）与 `PKGV0024`（场景子包）的解析与解包。
//!
//! 两种包的二进制布局**同构**（服务端 `server/we.cjs:99 parsePkg` 已验证）：
//!
//! ```text
//! u32 magic_len
//! [magic_len] magic            // "PKGM0019" 或 "PKGV0024"
//! u32 count
//! count × { u32 name_len, [name_len] name, u32 data_off, u32 data_size }
//! ---- data_base ----
//! count 段裸数据（**不压缩**），按 data_off / data_size 定位
//! ```
//!
//! 这里把「解析」和「写盘」分两层：
//!
//! 1. [`index_from_bytes`] / [`index_from_file`] 只产出索引，不碰内容；
//! 2. [`unpack_file_to_dir`] 按索引 `seek` 到每一段，64 KiB 一块拷出去 ——
//!    全程内存里只有索引加一块缓冲，几十 MB 的包也不会整文件读进内存。
//!
//! 条目名在**写盘之前**逐条过 [`safe_rel`]（zip-slip 防护），校验失败时磁盘上
//! 一个字节都没写过。

use std::fs::{self, File};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde_json::Value;

use super::{Result, WallpaperError};

/// 容器包（完整壁纸：`project.json` + 场景/视频/网页资源）。
pub const PKG_MAGIC_CONTAINER: &str = "PKGM0019";
/// 场景子包（只有场景那一半，`project.json` 之外的部分）。
pub const PKG_MAGIC_SCENE: &str = "PKGV0024";

/// magic 的前缀。**版本号那 4 位数字不能当判据**：工坊里真实存在的场景
/// 子包有 `PKGV0001` / `0003` / `0005` / `0006` / `0021` / `0022` / `0024` 等，
/// 条目表结构（nameLen + name + offset + size）与数据区（不压缩）完全一致，
/// 只有这 4 位不同。早先只认 `PKGV0024`，于是老包一律被判「不是壁纸包」，
/// `repair()` 恒失败、界面红点永远点不动。
pub const PKG_MAGIC_SCENE_PREFIX: &str = "PKGV";
/// 容器包同理（`PKGM0019` 之外还有更早的容器版本）。
pub const PKG_MAGIC_CONTAINER_PREFIX: &str = "PKGM";

/// `PKGV0024` / `PKGM0019` 这类固定 8 字节 magic：4 字节前缀 + 4 位数字。
fn magic_matches(magic: &str, prefix: &str) -> bool {
    magic.len() == 8
        && magic.starts_with(prefix)
        && magic.as_bytes()[4..].iter().all(u8::is_ascii_digit)
}

/// 是不是场景子包（任意版本）。
pub fn is_scene_magic(magic: &str) -> bool {
    magic_matches(magic, PKG_MAGIC_SCENE_PREFIX)
}

/// 是不是容器包（任意版本）。
pub fn is_container_magic(magic: &str) -> bool {
    magic_matches(magic, PKG_MAGIC_CONTAINER_PREFIX)
}

/// 单个包解包后的体积上限。
pub const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 相对路径总长度上限。
pub const MAX_REL_LEN: usize = 1024;
/// 单段路径长度上限（Windows 的单个文件名上限是 255）。
pub const MAX_SEGMENT_LEN: usize = 255;

/// magic 字段本身长度上限（正常只有 8）。
const MAX_MAGIC_LEN: u32 = 64;
/// 拷贝缓冲大小。
const COPY_CHUNK: usize = 64 * 1024;
/// 从容器里读 `project.json` 时的上限，防止被一个巨大的假 entry 拖死。
const MAX_PROJECT_BYTES: u64 = 1024 * 1024;

/// 解析上限。做成可配置是为了让单测能用几十字节的 fixture 验证「总量超限」这条规则。
#[derive(Debug, Clone, Copy)]
pub struct PkgLimits {
    /// 解包总量上限。
    pub max_unpacked_bytes: u64,
    /// 单条条目名长度上限。
    pub max_name_len: u32,
    /// 条目数上限。
    pub max_entries: u32,
    /// 索引区一次性读取的上限。
    pub max_header_bytes: u64,
}

impl Default for PkgLimits {
    fn default() -> Self {
        Self {
            max_unpacked_bytes: MAX_UNPACKED_BYTES,
            max_name_len: 4096,
            max_entries: 65_536,
            max_header_bytes: 8 * 1024 * 1024,
        }
    }
}

/// 索引里的一条：`rel` 已过 [`safe_rel`]，`offset` 相对数据区起点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryIndex {
    pub rel: String,
    pub offset: u64,
    pub size: u64,
}

/// 解析出来的包索引（不含内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkgIndex {
    pub magic: String,
    pub entries: Vec<EntryIndex>,
    /// 数据区在文件里的起始偏移。
    pub data_base: u64,
}

impl PkgIndex {
    /// 是不是「只有场景那一半」的子包（任意版本，见 [`is_scene_magic`]）。
    pub fn is_scene_package(&self) -> bool {
        is_scene_magic(&self.magic)
    }

    /// 所有条目字节数之和。
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|entry| entry.size).sum()
    }

    /// 按相对路径找一条。
    pub fn entry(&self, rel: &str) -> Option<&EntryIndex> {
        self.entries.iter().find(|entry| entry.rel == rel)
    }

    /// 找 `project.json`：可能在根，也可能在一层子目录里。
    fn project_entry(&self) -> Option<&EntryIndex> {
        self.entries
            .iter()
            .find(|entry| entry.rel == "project.json")
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|entry| entry.rel.ends_with("/project.json"))
            })
    }
}

/// 解包统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnpackSummary {
    pub entries: usize,
    pub bytes: u64,
}

/// 包内路径安全校验。合法则返回规范化后的相对路径（`\` → `/`，去掉 `.` 段）。
///
/// 拒绝：空名、绝对路径、盘符（`C:`）、任何位置上的 `:`（Windows 备用数据流）、
/// NUL 与控制字符、`..` 段、空段（含 `a//b`、结尾 `/`）、超长、Windows 设备名。
///
/// 与服务端 `safeRel`（`server/we.cjs:91`）同款，额外收紧了两条：全路径统一的
/// `:` 检查，以及设备名/尾随点号空格（Windows 会把 `a.` 落到 `a`，让两条条目
/// 互相覆盖）。
/// 目录里有没有可用的**容器入口**（WE 的三种命名）。
///
/// 真实库里大量条目的 `project.json` 写的是 `"file": "scene.json"`（编辑器工程形态），
/// 而实际只导入了 `scene.pkg` 容器。webwallgl 的 `httpSource` 对此**自带回退**
/// （README：「松散入口取不到时自动回退容器」），所以我们不该把这种包判成 damaged ——
/// 判死的后果是前端 `mode` 直接落到 poster，看起来就是「一直只显示缩略图」。
pub fn has_scene_container(dir: &Path) -> bool {
    ["scene.pkg", "scenes/scene.pkg", "gifscene.pkg"]
        .iter()
        .any(|rel| dir.join(rel).is_file())
}

/// 目录里任意一个指定后缀的文件（只扫一层，够用且便宜）。
pub fn any_with_ext(dir: &Path, exts: &[&str]) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().ok()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        if exts.iter().any(|ext| lower.ends_with(ext)) {
            return Some(name);
        }
    }
    None
}

pub fn safe_rel(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    // 反斜杠一律当分隔符：条目名是别人写的，两种分隔符都得防。
    let normalized = name.replace('\\', "/");
    if normalized.is_empty() || normalized.len() > MAX_REL_LEN {
        return None;
    }
    if normalized.starts_with('/') {
        return None;
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
    {
        return None;
    }
    for character in normalized.chars() {
        // 控制字符包含 NUL；`:` 既能伪装盘符也能开备用数据流。
        if character == ':' || (character as u32) < 0x20 {
            return None;
        }
    }

    let mut segments: Vec<&str> = Vec::new();
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == ".." {
            return None;
        }
        if segment == "." {
            // `./a` 与 `a` 是同一个文件，规整掉而不是拒绝。
            continue;
        }
        if segment.len() > MAX_SEGMENT_LEN {
            return None;
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return None;
        }
        if is_windows_device_name(segment) {
            return None;
        }
        segments.push(segment);
    }

    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

/// `CON`、`NUL`、`COM1` … 在 Windows 上是设备而不是文件。
fn is_windows_device_name(segment: &str) -> bool {
    let stem = segment
        .split_once('.')
        .map_or(segment, |(stem, _extension)| stem);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.len() == 4
        && upper.as_bytes()[3].is_ascii_digit())
}

/// 顺序读取头部字节的小游标：读不够就返回 `None`，由调用方判断是「文件太短」
/// 还是「索引区放不进一次读取」。
struct HeaderCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> HeaderCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn u32(&mut self) -> Option<u32> {
        let chunk = self.take(4)?;
        Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
}

/// 头部解析结果：要么解析完了，要么**这段字节**还不够。
enum HeaderOutcome {
    Parsed(PkgIndex),
    Short,
}

fn parse_header(
    bytes: &[u8],
    limits: &PkgLimits,
) -> Result<HeaderOutcome> {
    if bytes.len() < 12 {
        return Ok(HeaderOutcome::Short);
    }
    let mut cursor = HeaderCursor::new(bytes);

    // 长度已经过 `bytes.len() >= 12` 保证，取不到只可能是逻辑出错。
    let Some(magic_len) = cursor.u32() else {
        return Ok(HeaderOutcome::Short);
    };
    if magic_len > MAX_MAGIC_LEN {
        return Err(WallpaperError::NotAPackage);
    }
    let Some(magic_bytes) = cursor.take(magic_len as usize) else {
        return Ok(HeaderOutcome::Short);
    };
    let magic = String::from_utf8_lossy(magic_bytes).into_owned();
    if !is_container_magic(&magic) && !is_scene_magic(&magic) {
        return Err(WallpaperError::NotAPackage);
    }

    let Some(count) = cursor.u32() else {
        return Ok(HeaderOutcome::Short);
    };
    if count > limits.max_entries {
        return Err(WallpaperError::PackageTableTooLarge);
    }

    let mut entries = Vec::with_capacity((count as usize).min(1024));
    let mut total: u64 = 0;
    for _ in 0..count {
        let Some(name_len) = cursor.u32() else {
            return Ok(HeaderOutcome::Short);
        };
        if name_len > limits.max_name_len {
            return Err(WallpaperError::EntryNameTooLong);
        }
        let Some(name_bytes) = cursor.take(name_len as usize) else {
            return Ok(HeaderOutcome::Short);
        };
        let Ok(name) = std::str::from_utf8(name_bytes) else {
            return Err(WallpaperError::EntryNameInvalid);
        };
        let Some(offset) = cursor.u32() else {
            return Ok(HeaderOutcome::Short);
        };
        let Some(size) = cursor.u32() else {
            return Ok(HeaderOutcome::Short);
        };

        // 写盘之前就把不安全的名字挡住。
        let Some(rel) = safe_rel(name) else {
            return Err(WallpaperError::UnsafePath);
        };

        total = total.saturating_add(u64::from(size));
        if total > limits.max_unpacked_bytes {
            return Err(WallpaperError::PackageTooLarge);
        }

        entries.push(EntryIndex {
            rel,
            offset: u64::from(offset),
            size: u64::from(size),
        });
    }

    Ok(HeaderOutcome::Parsed(PkgIndex {
        magic,
        entries,
        data_base: cursor.pos as u64,
    }))
}

/// 校验索引声明的数据区没有伸出文件末尾。
fn finish_index(index: PkgIndex, file_len: u64) -> Result<PkgIndex> {
    let needed = index
        .entries
        .iter()
        .map(|entry| entry.offset.saturating_add(entry.size))
        .max()
        .unwrap_or(0);
    if index.data_base.saturating_add(needed) > file_len {
        return Err(WallpaperError::PackageTruncated);
    }
    Ok(index)
}

/// 从一段完整字节里解析（单测与内存场景用）。
pub fn index_from_bytes(bytes: &[u8]) -> Result<PkgIndex> {
    index_from_bytes_with_limits(bytes, &PkgLimits::default())
}

/// 同 [`index_from_bytes`]，但可指定上限。
pub fn index_from_bytes_with_limits(
    bytes: &[u8],
    limits: &PkgLimits,
) -> Result<PkgIndex> {
    if (bytes.len() as u64) < 12 {
        return Err(WallpaperError::PackageTooShort);
    }
    match parse_header(bytes, limits)? {
        HeaderOutcome::Short => Err(WallpaperError::PackageTruncated),
        HeaderOutcome::Parsed(index) => {
            finish_index(index, bytes.len() as u64)
        }
    }
}

/// 从文件里解析，只读索引区，不读内容。
pub fn index_from_file(path: &Path) -> Result<PkgIndex> {
    index_from_file_with_limits(path, &PkgLimits::default())
}

/// 同 [`index_from_file`]，但可指定上限。
pub fn index_from_file_with_limits(
    path: &Path,
    limits: &PkgLimits,
) -> Result<PkgIndex> {
    let mut file = open_source(path)?;
    let file_len = file.metadata()?.len();
    if file_len < 12 {
        return Err(WallpaperError::PackageTooShort);
    }

    let prefix_len = file_len.min(limits.max_header_bytes) as usize;
    let mut prefix = vec![0_u8; prefix_len];
    file.read_exact(&mut prefix)?;

    match parse_header(&prefix, limits)? {
        // 索引区放不进一次读取：要么文件本身就到头了（下载不完整），
        // 要么索引区大得不像话（损坏或伪造）。
        HeaderOutcome::Short => {
            if (prefix_len as u64) < file_len {
                Err(WallpaperError::PackageTableTooLarge)
            } else {
                Err(WallpaperError::PackageTruncated)
            }
        }
        HeaderOutcome::Parsed(index) => finish_index(index, file_len),
    }
}

/// 打开源文件，把「不存在」翻译成面向用户的错误。
fn open_source(path: &Path) -> Result<File> {
    File::open(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            WallpaperError::SourceMissing
        } else {
            WallpaperError::Io(error)
        }
    })
}

/// 按索引把条目写到 `dest`（`seek` + 定长读取，内存只占一块缓冲）。
pub fn unpack_file_to_dir(src: &Path, dest: &Path) -> Result<UnpackSummary> {
    let limits = PkgLimits::default();
    let index = index_from_file_with_limits(src, &limits)?;
    unpack_index_to_dir(src, &index, dest, &limits)
}

/// 同 [`unpack_file_to_dir`]，但用现成的索引（避免重复解析）。
pub fn unpack_index_to_dir(
    src: &Path,
    index: &PkgIndex,
    dest: &Path,
    limits: &PkgLimits,
) -> Result<UnpackSummary> {
    fs::create_dir_all(dest)?;
    let mut file = open_source(src)?;
    let mut buffer = vec![0_u8; COPY_CHUNK];
    let mut summary = UnpackSummary::default();

    for entry in &index.entries {
        // 索引里已经校验过一次；写盘前再挡一遍，避免将来有人绕过索引直接用条目。
        let Some(rel) = safe_rel(&entry.rel) else {
            return Err(WallpaperError::UnsafePath);
        };
        let target = dest.join(&rel);
        if !target.starts_with(dest) {
            return Err(WallpaperError::UnsafePath);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        file.seek(SeekFrom::Start(
            index.data_base.saturating_add(entry.offset),
        ))?;
        let mut out = File::create(&target)?;
        let mut remaining = entry.size;
        while remaining > 0 {
            let chunk = remaining.min(COPY_CHUNK as u64) as usize;
            file.read_exact(&mut buffer[..chunk])?;
            out.write_all(&buffer[..chunk])?;
            remaining -= chunk as u64;
            summary.bytes = summary.bytes.saturating_add(chunk as u64);
        }
        out.flush()?;
        summary.entries += 1;
    }

    if summary.bytes > limits.max_unpacked_bytes {
        return Err(WallpaperError::PackageTooLarge);
    }
    Ok(summary)
}

/// 把目录里**顶层**的 `.pkg`（工坊目录里主场景常是打包的 `scene.pkg`）就地展开。
///
/// 展开后容器删掉，目录形态与 `.mpkg` 解包后一致（不留容器）：否则前端按
/// `project.file` 取 `scene.json` 只会拿到 404。认不出的 `.pkg` 原样留着 ——
/// 包里可能夹带别的 pkg，那不是我们的东西。
///
/// 返回展开了几个容器。容器删不掉不算错（多留一个文件不影响功能）。
pub fn expand_scene_packages(dir: &Path) -> Result<usize> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Ok(0);
    };

    let mut names: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let is_pkg = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pkg"));
        if is_pkg {
            names.push(path.to_string_lossy().into_owned());
        }
    }
    // 顺序固定，排查问题时日志可复现。
    names.sort();

    let limits = PkgLimits::default();
    let mut expanded = 0;
    for name in names {
        let path = Path::new(&name);
        let Ok(index) = index_from_file_with_limits(path, &limits) else {
            continue;
        };
        if unpack_index_to_dir(path, &index, dir, &limits).is_err() {
            continue;
        }
        let _ = fs::remove_file(path);
        expanded += 1;
    }
    Ok(expanded)
}

/// 读目录里的 `project.json`；不存在返回 `Ok(None)`，坏 JSON 返回错误。
pub fn read_project_file(dir: &Path) -> Result<Option<Value>> {
    match fs::read(dir.join("project.json")) {
        Ok(bytes) => parse_project_bytes(&bytes).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(WallpaperError::Io(error)),
    }
}

/// 从**尚未解包**的容器里读 `project.json`（目录形态读不到时才用得到）。
pub fn read_project_from_index(
    src: &Path,
    index: &PkgIndex,
) -> Result<Option<Value>> {
    let Some(entry) = index.project_entry() else {
        return Ok(None);
    };
    if entry.size > MAX_PROJECT_BYTES {
        return Err(WallpaperError::ProjectTooLarge);
    }

    let mut file = open_source(src)?;
    file.seek(SeekFrom::Start(
        index.data_base.saturating_add(entry.offset),
    ))?;
    let mut bytes = vec![0_u8; entry.size as usize];
    file.read_exact(&mut bytes)?;
    parse_project_bytes(&bytes).map(Some)
}

/// 解析 `project.json` 字节（容忍 UTF-8 BOM）。
fn parse_project_bytes(bytes: &[u8]) -> Result<Value> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    Ok(serde_json::from_str(text)?)
}

/// 场景子包（`PKGV0024`）不含 `project.json` —— 它是 `project.json` 之外的另一半。
/// 单独导入时按最小信息补一份，否则资源全在场也装不进去。
///
/// 没有 `scene.json` 就不补：那不是能用的包。
pub fn synth_project(
    src: &Path,
    dir: &Path,
    workshop_id: Option<&str>,
) -> Option<Value> {
    if !dir.join("scene.json").is_file() {
        return None;
    }
    let title = src
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "wallpaper".to_string());
    let preview = ["preview.gif", "preview.jpg", "preview.jpeg", "preview.png"]
        .into_iter()
        .find(|name| dir.join(name).is_file())
        .unwrap_or("");

    let mut project = serde_json::json!({
        "title": title,
        "type": "scene",
        "file": "scene.json",
        "preview": preview,
    });
    if let Some(workshop_id) = workshop_id
        && !workshop_id.is_empty()
        && let Some(object) = project.as_object_mut()
    {
        object.insert(
            "workshopid".to_string(),
            Value::String(workshop_id.to_string()),
        );
    }
    Some(project)
}

/// 拼一个容器字节流：`magic` + 若干 `(name, data)`。仅测试用。
#[cfg(test)]
pub(crate) fn test_package(magic: &str, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let magic_bytes = magic.as_bytes();
    out.extend_from_slice(&(magic_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(magic_bytes);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());

    // 数据区起点 = 头部总长度，先算出来才能写 offset。
    let header_len: usize = 4
        + magic_bytes.len()
        + 4
        + entries
            .iter()
            .map(|(name, _)| 4 + name.len() + 8)
            .sum::<usize>();
    let mut data = Vec::new();
    for (name, bytes) in entries {
        let name_bytes = name.as_bytes();
        out.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(bytes);
    }
    assert_eq!(out.len(), header_len, "头部长度算错了");
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_both_magics() {
        let container = test_package(
            PKG_MAGIC_CONTAINER,
            &[("project.json", b"{}"), ("scene.json", b"{}")],
        );
        let index = index_from_bytes(&container).expect("container parses");
        assert_eq!(index.magic, PKG_MAGIC_CONTAINER);
        assert!(!index.is_scene_package());
        assert_eq!(index.entries.len(), 2);
        assert_eq!(index.entries[0].rel, "project.json");
        assert_eq!(index.entries[1].rel, "scene.json");
        assert_eq!(index.total_bytes(), 4);

        let scene = test_package(PKG_MAGIC_SCENE, &[("scene.json", b"{}")]);
        let index = index_from_bytes(&scene).expect("scene package parses");
        assert_eq!(index.magic, PKG_MAGIC_SCENE);
        assert!(index.is_scene_package());
    }

    #[test]
    fn rejects_unknown_magic() {
        // 只认 `PKGM` / `PKGV` 前缀，别的前缀一律不是壁纸包。
        let bytes = test_package("PKGX0001", &[("project.json", b"{}")]);
        let error = index_from_bytes(&bytes).expect_err("must reject");
        assert!(matches!(error, WallpaperError::NotAPackage));
        assert_eq!(error.to_string(), "不是 Wallpaper Engine 壁纸包");
    }

    /// 工坊里真实存在的老版本场景子包（`PKGV0001` … `PKGV0022`）必须能解析。
    /// 早先只认 `PKGV0024`，导致这些包的 `repair()` 恒失败、界面红点永远点不动。
    #[test]
    fn accepts_older_scene_versions() {
        for magic in ["PKGV0001", "PKGV0003", "PKGV0005", "PKGV0021", "PKGV0022"] {
            let bytes = test_package(magic, &[("scene.json", b"{}")]);
            let index = index_from_bytes(&bytes).unwrap_or_else(|error| {
                panic!("{magic} must parse: {error}")
            });
            assert!(index.is_scene_package(), "{magic} is a scene package");
            assert_eq!(index.magic, magic);
        }
        // 容器版本号同理放开，但仍不能把场景包认成容器。
        let container = test_package("PKGM0017", &[("project.json", b"{}")]);
        let index = index_from_bytes(&container).expect("container parses");
        assert!(!index.is_scene_package());
        // 前缀对但数字位不是数字：不是合法 magic。
        assert!(index_from_bytes(&test_package("PKGV00xy", &[("a", b"")])).is_err());
    }

    #[test]
    fn rejects_short_file() {
        let error =
            index_from_bytes(b"PKGM").expect_err("must reject short file");
        assert!(matches!(error, WallpaperError::PackageTooShort));

        // 只有 magic、没有 count。
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(PKG_MAGIC_CONTAINER.as_bytes());
        let error =
            index_from_bytes(&bytes).expect_err("must reject no-count");
        assert!(matches!(error, WallpaperError::PackageTruncated));
    }

    #[test]
    fn rejects_parent_traversal_entries() {
        let bytes = test_package(
            PKG_MAGIC_CONTAINER,
            &[("project.json", b"{}"), ("../../evil.txt", b"x")],
        );
        let error = index_from_bytes(&bytes).expect_err("must reject ..");
        assert!(matches!(error, WallpaperError::UnsafePath));
        assert_eq!(error.to_string(), "壁纸包里含有不安全的路径");

        let bytes = test_package(
            PKG_MAGIC_CONTAINER,
            &[("..\\..\\evil.txt", b"x")],
        );
        assert!(matches!(
            index_from_bytes(&bytes).expect_err("backslashes too"),
            WallpaperError::UnsafePath
        ));
    }

    #[test]
    fn rejects_absolute_paths_and_drive_letters() {
        for name in [
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:\\Windows\\x.txt",
            "c:/windows/x.txt",
        ] {
            let bytes = test_package(PKG_MAGIC_CONTAINER, &[(name, b"x")]);
            assert!(
                matches!(
                    index_from_bytes(&bytes).expect_err("must reject"),
                    WallpaperError::UnsafePath
                ),
                "{name} should have been rejected"
            );
        }
    }

    #[test]
    fn rejects_oversized_unpacked_total() {
        // 两条各 8 字节 = 16 字节，上限压到 10 就能在几十字节的 fixture 上
        // 验证「总量超限」这条规则，不必真的造 2 GB。
        let bytes = test_package(
            PKG_MAGIC_CONTAINER,
            &[("a.bin", b"12345678"), ("b.bin", b"12345678")],
        );
        let limits = PkgLimits {
            max_unpacked_bytes: 10,
            ..PkgLimits::default()
        };
        let error = index_from_bytes_with_limits(&bytes, &limits)
            .expect_err("must reject oversized total");
        assert!(matches!(error, WallpaperError::PackageTooLarge));
        assert_eq!(error.to_string(), "壁纸包解出来超过 2 GB，已中止");

        // 默认上限就是 2 GiB。
        assert_eq!(PkgLimits::default().max_unpacked_bytes, MAX_UNPACKED_BYTES);
        assert_eq!(MAX_UNPACKED_BYTES, 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn rejects_truncated_data_region() {
        let mut bytes =
            test_package(PKG_MAGIC_CONTAINER, &[("project.json", b"{}")]);
        bytes.truncate(bytes.len() - 1);
        assert!(matches!(
            index_from_bytes(&bytes).expect_err("must reject"),
            WallpaperError::PackageTruncated
        ));
    }

    #[test]
    fn rejects_null_bytes_and_overlong_names() {
        let mut bytes = Vec::new();
        let magic_bytes = PKG_MAGIC_CONTAINER.as_bytes();
        bytes.extend_from_slice(&(magic_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(magic_bytes);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let name = b"bad\0name";
        bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        assert!(matches!(
            index_from_bytes(&bytes).expect_err("NUL is not a path"),
            WallpaperError::UnsafePath
        ));

        let limits = PkgLimits {
            max_name_len: 4,
            ..PkgLimits::default()
        };
        let bytes = test_package(PKG_MAGIC_CONTAINER, &[("abcde", b"")]);
        assert!(matches!(
            index_from_bytes_with_limits(&bytes, &limits)
                .expect_err("name too long"),
            WallpaperError::EntryNameTooLong
        ));
    }

    #[test]
    fn safe_rel_normalises_separators() {
        assert_eq!(safe_rel("materials/a.json").as_deref(), Some("materials/a.json"));
        assert_eq!(
            safe_rel("materials\\a.json").as_deref(),
            Some("materials/a.json")
        );
        assert_eq!(safe_rel("./scene.json").as_deref(), Some("scene.json"));
        assert_eq!(
            safe_rel("shaders/glsl/v2/common.vert").as_deref(),
            Some("shaders/glsl/v2/common.vert")
        );
        // 中文名是合法文件名（工坊包很常见）。
        assert_eq!(
            safe_rel("素材/背景.png").as_deref(),
            Some("素材/背景.png")
        );
    }

    #[test]
    fn safe_rel_rejects_unsafe_shapes() {
        for name in [
            "",
            "/",
            "//a",
            "a//b",
            "a/",
            "..",
            "../a",
            "a/../../b",
            "C:a",
            "a:b",
            "a/b\0c",
            "a/ ",
            "a/b.",
            "CON",
            "nul.txt",
            "COM1",
        ] {
            assert!(safe_rel(name).is_none(), "{name:?} should be rejected");
        }

        let long_segment = "a".repeat(MAX_SEGMENT_LEN + 1);
        assert!(safe_rel(&long_segment).is_none());

        let long_path = std::iter::repeat_n("abcdefghij", 120)
            .collect::<Vec<_>>()
            .join("/");
        assert!(long_path.len() > MAX_REL_LEN);
        assert!(safe_rel(&long_path).is_none());
    }

    #[test]
    fn finds_a_project_json_in_a_subdirectory() {
        let bytes = test_package(
            PKG_MAGIC_CONTAINER,
            &[("wallpaper/project.json", b"{\"type\":\"scene\"}")],
        );
        let index = index_from_bytes(&bytes).expect("parses");
        assert!(index.project_entry().is_some());
        assert!(index.entry("nope.json").is_none());
    }

    #[test]
    fn unpacks_a_package_into_a_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("wallpaper.mpkg");
        let dest = temp.path().join("out");
        let bytes = test_package(
            PKG_MAGIC_CONTAINER,
            &[
                ("project.json", b"{\"type\":\"video\"}"),
                ("materials/x.json", b"{\"a\":1}"),
            ],
        );
        std::fs::write(&source, &bytes).expect("write fixture");

        let summary = unpack_file_to_dir(&source, &dest).expect("unpack");

        assert_eq!(summary.entries, 2);
        assert_eq!(summary.bytes, 23);
        assert_eq!(
            std::fs::read(dest.join("project.json")).expect("read"),
            b"{\"type\":\"video\"}"
        );
        assert!(dest.join("materials/x.json").is_file());
    }

    #[test]
    fn unpacks_a_megabyte_payload_without_truncation() {
        // 只有头部改动、内容留空的 1 MiB 包：解包必须按块拷，不能整读。
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("wallpaper.mpkg");
        let dest = temp.path().join("out");
        let payload = vec![7_u8; 1024 * 1024];
        let bytes = test_package(PKG_MAGIC_CONTAINER, &[("blob.bin", &payload)]);
        std::fs::write(&source, &bytes).expect("write fixture");

        let summary = unpack_file_to_dir(&source, &dest).expect("unpack");

        assert_eq!(summary.bytes, payload.len() as u64);
        assert_eq!(
            std::fs::metadata(dest.join("blob.bin")).expect("stat").len(),
            payload.len() as u64
        );
    }

    #[test]
    fn expands_a_scene_package_in_place_and_drops_the_container() {
        let temp = tempfile::tempdir().expect("temp dir");
        let scene = test_package(
            PKG_MAGIC_SCENE,
            &[
                ("scene.json", b"{\"objects\":[]}"),
                ("shaders/common.vert", b"void main(){}"),
            ],
        );
        std::fs::write(temp.path().join("scene.pkg"), &scene)
            .expect("write scene.pkg");
        // 认不出的 pkg 必须原样留着。
        let not_a_pkg = b"\x00\x00\x00\x00nope";
        std::fs::write(temp.path().join("other.pkg"), not_a_pkg)
            .expect("write other.pkg");

        let expanded = expand_scene_packages(temp.path()).expect("expand");

        assert_eq!(expanded, 1);
        assert!(temp.path().join("scene.json").is_file());
        assert!(temp.path().join("shaders/common.vert").is_file());
        assert!(!temp.path().join("scene.pkg").exists());
        assert_eq!(
            std::fs::read(temp.path().join("other.pkg")).expect("read"),
            not_a_pkg
        );
    }

    #[test]
    fn reads_project_json_out_of_an_unpacked_container() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("wallpaper.mpkg");
        std::fs::write(
            &source,
            test_package(
                PKG_MAGIC_CONTAINER,
                &[("project.json", b"\xef\xbb\xbf{\"title\":\"\xe6\xb2\xb3\"}")],
            ),
        )
        .expect("write fixture");

        let index = index_from_file(&source).expect("index");
        let project =
            read_project_from_index(&source, &index).expect("read project");

        assert_eq!(
            project.and_then(|value| value["title"].as_str().map(str::to_string)),
            Some("河".to_string())
        );
    }

    #[test]
    fn synthesises_a_minimal_project_for_a_scene_package() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp.path().join("scene.json"), b"{}").expect("scene");
        std::fs::write(temp.path().join("preview.jpg"), b"jpg").expect("preview");

        let project = synth_project(
            Path::new("D:/workshop/12345/scene.pkg"),
            temp.path(),
            Some("12345"),
        )
        .expect("synth");

        assert_eq!(project["title"], "scene");
        assert_eq!(project["type"], "scene");
        assert_eq!(project["file"], "scene.json");
        assert_eq!(project["preview"], "preview.jpg");
        assert_eq!(project["workshopid"], "12345");

        // 没有 scene.json 就不补。
        std::fs::remove_file(temp.path().join("scene.json")).expect("rm");
        assert!(
            synth_project(Path::new("x.pkg"), temp.path(), None).is_none()
        );
    }

    #[test]
    fn rejects_bad_project_json_in_a_container() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("wallpaper.mpkg");
        std::fs::write(
            &source,
            test_package(PKG_MAGIC_CONTAINER, &[("project.json", b"{oops")]),
        )
        .expect("write fixture");

        let index = index_from_file(&source).expect("index");
        let error = read_project_from_index(&source, &index)
            .expect_err("must reject bad json");
        assert!(matches!(error, WallpaperError::BadProject(_)));
        assert_eq!(error.to_string(), "壁纸包的 project.json 不是合法 JSON");
    }

    #[test]
    fn missing_source_is_reported_without_a_path() {
        let error = index_from_file(Path::new(
            "D:/definitely/not/here/wallpaper.mpkg",
        ))
        .expect_err("must fail");
        assert!(matches!(error, WallpaperError::SourceMissing));
        assert!(!error.to_string().contains("D:"));
    }

    #[test]
    fn header_cursor_stops_at_the_end() {
        let data = [1_u8, 0, 0, 0, 2];
        let mut cursor = HeaderCursor::new(&data);
        assert_eq!(cursor.u32(), Some(1));
        assert_eq!(cursor.take(4), None);
        assert_eq!(cursor.take(1), Some(&data[4..]));
        assert_eq!(cursor.take(0), Some(&data[5..]));
    }
}
