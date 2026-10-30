//! `wallpapers/` 目录布局、内容哈希 id、导入 / 列出 / 删除与 `config.json`。
//!
//! ```text
//! $APPDATA/<identifier>/wallpapers/
//! ├─ packages/<id>/          # 松散目录形态（与后台完全一致）
//! ├─ incoming/               # 导入暂存，成功即清空
//! ├─ posters/<id>.jpg        # 海报（= 包内 preview 的副本）
//! └─ config.json             # 选中项 + 属性覆盖 + 档位（原子写）
//! ```
//!
//! 两条硬规则：
//!
//! 1. **id = 目录内容 SHA-1 的前 12 位**（同名 + 同字节）。内容一样 ⇒ id 一样 ⇒
//!    二次导入直接命中，不重复占盘；换个包 ⇒ 换 id ⇒ URL 换空间，
//!    前端那条 `immutable` 缓存天然失效。
//! 2. **写入一律「临时文件 + 原子 rename」**。`config.json` 写到一半被杀进程
//!    不能留下半个文件；包目录也是先在 `incoming/` 里装好，最后一步改名，
//!    装失败时 `packages/` 下不会出现半成品（半成品会被 `list()` 当成好包）。

use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use sha1_smol::Sha1;

use super::tex_audit;
use super::{Result, TexAudit, WallpaperConfig, WallpaperError, WallpaperMeta, WallpaperType};
use super::pkg;

/// 包目录名。
pub const PACKAGES_DIR_NAME: &str = "packages";
/// 导入暂存目录名。
pub const INCOMING_DIR_NAME: &str = "incoming";
/// 海报目录名。
pub const POSTERS_DIR_NAME: &str = "posters";
/// 配置文件。
pub const CONFIG_FILE_NAME: &str = "config.json";
/// 每个包内的元数据文件（与后台同名，便于两边目录对照）。
pub const META_FILE_NAME: &str = ".hayenai-meta.json";

/// id 里展示前缀 + 40 位哈希最多这么长。
const MAX_ID_LEN: usize = 96;
/// 标题转 slug 的长度上限。
const MAX_SLUG_LEN: usize = 40;
/// 内容哈希取前多少位（十六进制字符）。
const ID_HASH_LEN: usize = 12;
/// 拷贝缓冲。
const COPY_CHUNK: usize = 64 * 1024;
/// 目录递归深度上限。
const MAX_DEPTH: usize = 32;

/// 暂存目录名的自增部分，保证同一进程内不撞名。
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 目录统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirStat {
    /// 文件数。
    pub entries: usize,
    /// 总字节数。
    pub bytes: u64,
}

/// 导入选项。
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// 工坊 id。工坊包的 `project.json` 里**没有**这个字段，
    /// 只有拉取会话知道，所以允许调用方直接给。
    pub workshop_id: Option<String>,
}

/// 导入结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub meta: WallpaperMeta,
    /// 这个包本来就在（内容哈希撞上了），本次没有多占磁盘。
    pub already_installed: bool,
    /// 中文单句提示：ETC2 贴图、暂不支持的类型、入口文件缺失。
    pub warnings: Vec<String>,
}

/// `wallpapers/` 根目录的句柄。所有路径都由它派生，方便单测指到临时目录。
#[derive(Debug, Clone)]
pub struct WallpaperStore {
    root: PathBuf,
}

impl WallpaperStore {
    /// 以 `wallpapers/` 根目录构造。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `packages/`。
    pub fn packages_dir(&self) -> PathBuf {
        self.root.join(PACKAGES_DIR_NAME)
    }

    /// `incoming/`。
    pub fn incoming_dir(&self) -> PathBuf {
        self.root.join(INCOMING_DIR_NAME)
    }

    /// `posters/`。
    pub fn posters_dir(&self) -> PathBuf {
        self.root.join(POSTERS_DIR_NAME)
    }

    /// `config.json`。
    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE_NAME)
    }

    /// 建好四个必需目录。幂等。
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.packages_dir(),
            self.incoming_dir(),
            self.posters_dir(),
        ] {
            fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    /// `packages/<id>`。id 不合法时直接拒绝（它是用户可传的参数）。
    pub fn package_dir(&self, id: &str) -> Result<PathBuf> {
        if !is_valid_id(id) {
            return Err(WallpaperError::InvalidId);
        }
        Ok(self.packages_dir().join(id))
    }

    /// 列出所有已安装的壁纸。损坏的也列出来（界面标红），但不报错。
    pub fn list(&self) -> Result<Vec<WallpaperMeta>> {
        let mut items: Vec<WallpaperMeta> = Vec::new();
        let read_dir = match fs::read_dir(self.packages_dir()) {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(items);
            }
            Err(error) => return Err(WallpaperError::Io(error)),
        };

        for entry in read_dir {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if !is_valid_id(&id) {
                continue;
            }
            let dir = entry.path();
            // 清单读不出来不算致命：包可能只是坏了，界面要能标红并允许删除。
            let project = pkg::read_project_file(&dir).ok().flatten();
            let mut meta = match read_meta_file(&dir) {
                Some(meta) => meta,
                None => match &project {
                    Some(project) => self.meta_from_dir(&id, &dir, project)?,
                    None => continue,
                },
            };
            meta.id = id;
            if project.is_none() {
                meta.damaged = true;
            } else if meta.damaged {
                // 缓存（`.hayenai-meta.json`）里记着 damaged=true 的老包：如果目录里其实有
                // 容器入口，那是旧判定留下的误伤，就地改正 —— 否则用户得挨个点「修复」。
                match meta.kind {
                    WallpaperType::Scene if pkg::has_scene_container(&dir) => {
                        meta.damaged = false;
                        meta.file = String::new();
                    }
                    WallpaperType::Video => {
                        if let Some(name) =
                            pkg::any_with_ext(&dir, &[".mp4", ".webm", ".mov", ".m4v"])
                        {
                            meta.damaged = false;
                            meta.file = name;
                        }
                    }
                    _ => {}
                }
            }
            items.push(meta);
        }

        items.sort_by(|left, right| {
            left.title
                .to_lowercase()
                .cmp(&right.title.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(items)
    }

    /// 修复「容器形态」的旧包：目录里是 `scene.pkg`、但 `project.json` 指向的
    /// `scene.json` 并不存在（早期版本导入时没有就地展开）。
    ///
    /// 这类包会被 `damaged` 判死、界面按钮禁用 —— 表现就是「缩略图角落有红点、点不动」。
    /// 就地展开容器后，`scene.json` 与素材落在同一目录，下一次 `list()` 自然恢复正常。
    ///
    /// 返回 `true` 表示确实展开了（调用方应重新读一次 meta）。
    pub fn repair(&self, id: &str) -> Result<bool> {
        let dir = self.package_dir(id)?;
        if !dir.is_dir() {
            return Err(WallpaperError::UnknownPackage);
        }
        // 已经是松散形态：无事可做
        if dir.join("scene.json").is_file() {
            return Ok(false);
        }
        let expanded = pkg::expand_scene_packages(&dir)?;
        if expanded == 0 {
            return Ok(false);
        }
        // 缓存的 meta 里记着 damaged=true，必须清掉，否则下次 list() 还是判坏
        let _ = fs::remove_file(dir.join(META_FILE_NAME));
        Ok(true)
    }

    /// 读一个包。不在就报「找不到这个壁纸包」。
    pub fn get(&self, id: &str) -> Result<WallpaperMeta> {
        let dir = self.package_dir(id)?;
        if !dir.is_dir() {
            return Err(WallpaperError::UnknownPackage);
        }
        if let Some(mut meta) = read_meta_file(&dir) {
            meta.id = id.to_string();
            return Ok(meta);
        }
        let Some(project) = pkg::read_project_file(&dir)? else {
            return Err(WallpaperError::UnknownPackage);
        };
        let mut meta = self.meta_from_dir(id, &dir, &project)?;
        meta.id = id.to_string();
        Ok(meta)
    }

    /// 删除一个包：目录、海报、配置里的痕迹。幂等 —— 目录本来就不在也算成功，
    /// 因为「用户手动删掉包目录」这种情况下我们只负责把配置收拾干净。
    pub fn remove(&self, id: &str) -> Result<()> {
        let dir = self.package_dir(id)?;
        if dir.is_dir() {
            fs::remove_dir_all(&dir)?;
        }

        if let Ok(read_dir) = fs::read_dir(self.posters_dir()) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let matches_id =
                    path.file_stem().and_then(|stem| stem.to_str()) == Some(id);
                if matches_id {
                    let _ = fs::remove_file(path);
                }
            }
        }

        let mut config = self.read_config()?;
        let mut changed = false;
        if config.selected.as_deref() == Some(id) {
            config.selected = None;
            changed = true;
        }
        if config.properties.remove(id).is_some() {
            changed = true;
        }
        if changed {
            self.write_config(&config)?;
        }
        Ok(())
    }

    /// 选中（或取消选中）一个包。
    pub fn select(&self, id: Option<&str>) -> Result<()> {
        let mut config = self.read_config()?;
        match id {
            Some(id) => {
                let dir = self.package_dir(id)?;
                if !dir.is_dir() {
                    return Err(WallpaperError::UnknownPackage);
                }
                config.selected = Some(id.to_string());
            }
            None => config.selected = None,
        }
        self.write_config(&config)
    }

    /// 读 `config.json`。文件不存在 ⇒ 默认值；内容不是合法 JSON ⇒ 记一条 warn
    /// 后按默认值处理（配置坏掉不该让启动器打不开）。
    pub fn read_config(&self) -> Result<WallpaperConfig> {
        match fs::read_to_string(self.config_path()) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(config) => Ok(config),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "挂 wallpaper config.json 不是合法 JSON，按默认值处理"
                    );
                    Ok(WallpaperConfig::default())
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Ok(WallpaperConfig::default())
            }
            Err(error) => Err(WallpaperError::Io(error)),
        }
    }

    /// 原子写 `config.json`（临时文件 + rename），写之前把档位夹到安全区间。
    pub fn write_config(&self, config: &WallpaperConfig) -> Result<()> {
        self.ensure_dirs()?;
        write_json_atomic(&self.config_path(), &config.clone().sanitized())
    }

    /// 导入一个包。输入可以是 `.mpkg` / `.pkg` 文件，也可以是目录
    /// （工坊条目、WE 工程目录都算）。
    pub fn import(
        &self,
        src: &Path,
        options: &ImportOptions,
    ) -> Result<ImportOutcome> {
        self.ensure_dirs()?;
        let metadata = fs::metadata(src).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                WallpaperError::SourceMissing
            } else {
                WallpaperError::Io(error)
            }
        })?;
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(WallpaperError::SourceMissing);
        }

        let staging = self.incoming_dir().join(staging_name());
        let result = self.assemble(src, metadata.is_dir(), &staging, options);
        if result.is_err() {
            // 半成品必须清掉：它里面有 project.json，留着会被 list() 当成装好的包。
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    /// 装配流水线：拷贝/解包到暂存 → 展开 `scene.pkg` → 补 `project.json` →
    /// 算 id → 统计 + 体检 → 原子改名进 `packages/`。
    fn assemble(
        &self,
        src: &Path,
        is_dir: bool,
        staging: &Path,
        options: &ImportOptions,
    ) -> Result<ImportOutcome> {
        let mut is_scene_package = false;
        let mut project: Option<Value> = None;

        if is_dir {
            copy_tree(src, staging)?;
            // 工坊目录里的主场景常是打包的 scene.pkg，就地展开成松散目录。
            pkg::expand_scene_packages(staging)?;
            project = pkg::read_project_file(staging)?;
        } else {
            let index = pkg::index_from_file(src)?;
            is_scene_package = index.is_scene_package();
            project = pkg::read_project_from_index(src, &index)?;
            pkg::unpack_file_to_dir(src, staging)?;
        }

        let workshop_id = options
            .workshop_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| {
                project
                    .as_ref()
                    .and_then(|value| value.get("workshopid"))
                    .map(value_to_string)
                    .filter(|id| !id.is_empty())
            });

        // 场景子包不含 project.json：它是 project.json 之外的另一半。
        if project.is_none() && is_scene_package {
            project = pkg::synth_project(src, staging, workshop_id.as_deref());
        }
        let Some(project) = project else {
            return Err(WallpaperError::MissingProject);
        };

        // 合成的清单必须落盘：渲染器按 project.file 取入口，list() 也按它认包。
        if !staging.join("project.json").is_file() {
            write_json_atomic(&staging.join("project.json"), &project)?;
        }

        let id = self.build_id(&project, staging, workshop_id.as_deref())?;
        let dest = self.packages_dir().join(&id);

        if dest.is_dir() {
            // 内容哈希一样 ⇒ 就是同一个包 ⇒ 不重复占盘，直接把已有的报回去。
            let _ = fs::remove_dir_all(staging);
            let meta = match read_meta_file(&dest) {
                Some(meta) => meta,
                None => {
                    let existing =
                        pkg::read_project_file(&dest)?.unwrap_or(project);
                    self.meta_from_dir(&id, &dest, &existing)?
                }
            };
            return Ok(ImportOutcome {
                meta,
                already_installed: true,
                warnings: Vec::new(),
            });
        }

        let stat = dir_stat(staging)?;
        let tex = tex_audit::audit_dir(staging)?;
        let meta = meta_from_project(
            &id,
            staging,
            &project,
            stat,
            tex.clone(),
            now_millis(),
            workshop_id.as_deref(),
        );

        let mut warnings: Vec<String> = Vec::new();
        if let Some(warning) = tex.warning() {
            warnings.push(warning);
        }
        if !meta.kind.is_supported() {
            warnings.push(format!(
                "暂不支持{}类壁纸，导入后不会渲染",
                meta.kind.label()
            ));
        }
        if meta.damaged {
            warnings.push("包的入口文件缺失，可能装坏了".to_string());
        }

        write_json_atomic(&staging.join(META_FILE_NAME), &meta)?;

        fs::create_dir_all(self.packages_dir())?;
        // 同卷内目录改名是原子的：要么 packages/<id> 完整出现，要么压根没有。
        fs::rename(staging, &dest)?;

        // 海报只是给「过渡/回退」用的副本，失败不影响导入。
        if let Err(error) = self.copy_poster(&id, &dest, meta.preview.as_deref())
        {
            tracing::warn!(%error, "wallpaper 海报写入失败，忽略");
        }

        Ok(ImportOutcome {
            meta,
            already_installed: false,
            warnings,
        })
    }

    /// id = 前缀（`ws-<工坊 id>` / `local-<slug>`）+ 内容 SHA-1 前 12 位。
    fn build_id(
        &self,
        project: &Value,
        staging: &Path,
        workshop_id: Option<&str>,
    ) -> Result<String> {
        let prefix = match workshop_id.filter(|id| !id.is_empty()) {
            Some(workshop_id) => format!("ws-{}", slugify(workshop_id)),
            None => {
                let title = project
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                format!("local-{}", slugify(title))
            }
        };
        let hash = hash_dir(staging)?;
        let short = hash.get(..ID_HASH_LEN).unwrap_or(hash.as_str());
        let id = format!("{prefix}-{short}");
        if !is_valid_id(&id) {
            return Err(WallpaperError::InvalidId);
        }
        Ok(id)
    }

    /// 从目录内容重建元数据（`list()` 遇到没有清单文件的包时走这里）。
    ///
    /// 这里没有导入上下文，所以工坊 id 只能来自 `project.json` 自己
    /// （`workshop_id` 传 `None`）—— 缺清单文件的包本来就重建不出更多信息。
    fn meta_from_dir(
        &self,
        id: &str,
        dir: &Path,
        project: &Value,
    ) -> Result<WallpaperMeta> {
        let stat = dir_stat(dir)?;
        let tex = tex_audit::audit_dir(dir)?;
        Ok(meta_from_project(id, dir, project, stat, tex, now_millis(), None))
    }

    /// 把包内 `preview` 复制成 `posters/<id>.<ext>`。
    fn copy_poster(
        &self,
        id: &str,
        dir: &Path,
        preview: Option<&str>,
    ) -> Result<()> {
        let Some(preview) = preview else {
            return Ok(());
        };
        let source = dir.join(preview);
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("jpg")
            .to_ascii_lowercase();
        if !extension.chars().all(|character| character.is_ascii_alphanumeric())
        {
            return Ok(());
        }

        fs::create_dir_all(self.posters_dir())?;
        let mut source_file = File::open(&source)?;
        let mut temp = tempfile::NamedTempFile::new_in(self.posters_dir())
            .map_err(|_| WallpaperError::Config)?;
        std::io::copy(&mut source_file, &mut temp)?;
        temp.persist(self.posters_dir().join(format!("{id}.{extension}")))
            .map_err(|error| WallpaperError::Io(error.error))?;
        Ok(())
    }
}

/// `packages/<id>` 之类由用户传入的 id 白名单校验。
///
/// 生成出来的 id 只含 `[a-z0-9._-]`；这里不放行任何分隔符，也不放行以 `.`
/// 开头或含 `..` 的名字 —— id 会被拼进路径，也出现在取流 URL 里。
pub fn is_valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return false;
    }
    if id.starts_with('.') || id.contains("..") {
        return false;
    }
    id.chars()
        .all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.')
        })
}

/// 标题转 id 前缀：ASCII 字母数字转小写，其余折叠成单个 `-`。
///
/// 与服务端 `slugify`（`we.cjs:131`，保留中文）不同，这里**只保留 ASCII**：
/// id 会出现在 URL 与 Windows 目录名里，非 ASCII 要额外过百分号编码，
/// 不如统一成透明字符。中文标题会退化成 `local-wallpaper-<hash>` ——
/// id 本身是给机器看的，标题在 `WallpaperMeta.title` 里。
pub fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(character.to_ascii_lowercase());
            if out.len() >= MAX_SLUG_LEN {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "wallpaper".to_string()
    } else {
        let capped = &trimmed[..trimmed.len().min(MAX_SLUG_LEN)];
        let capped = capped.trim_matches('-');
        if capped.is_empty() {
            "wallpaper".to_string()
        } else {
            capped.to_string()
        }
    }
}

/// 目录内容 SHA-1：按路径排序后逐条喂「相对路径 + 文件字节」。
///
/// 排序用**字节序**（服务端 `hashDir` 用 `localeCompare`），所以同一个包在两边
/// 算出来的哈希不保证相等；本项目的 id 只要求自身稳定，不和服务端对齐 ——
/// 与服务端的对应关系靠 `project.json` 里的元信息，不靠 id。
pub fn hash_dir(dir: &Path) -> Result<String> {
    let mut sha = Sha1::new();
    let mut buffer = vec![0_u8; COPY_CHUNK];
    hash_dir_into(dir, "", &mut sha, &mut buffer, 0)?;
    Ok(sha.digest().to_string())
}

fn hash_dir_into(
    dir: &Path,
    prefix: &str,
    sha: &mut Sha1,
    buffer: &mut [u8],
    depth: usize,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let mut items: Vec<(String, bool)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // 清单文件是本工具自己写的，不能反过来影响 id（否则二次导入换 id）。
        if name == META_FILE_NAME {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            // 不跟随：包外内容不该算进哈希。
            continue;
        }
        if file_type.is_dir() {
            items.push((name, true));
        } else if file_type.is_file() {
            items.push((name, false));
        }
    }
    items.sort();

    for (name, is_dir) in items {
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if is_dir {
            hash_dir_into(
                &dir.join(&name),
                &rel,
                sha,
                buffer,
                depth + 1,
            )?;
            continue;
        }
        sha.update(rel.as_bytes());
        let mut file = File::open(dir.join(&name))?;
        loop {
            let read = file.read(buffer)?;
            if read == 0 {
                break;
            }
            sha.update(&buffer[..read]);
        }
    }
    Ok(())
}

/// 统计目录里的文件数与字节数（跳过清单文件与符号链接，与 [`hash_dir`] 一致）。
pub fn dir_stat(dir: &Path) -> Result<DirStat> {
    let mut stat = DirStat::default();
    accumulate_stat(dir, &mut stat, 0)?;
    Ok(stat)
}

fn accumulate_stat(
    dir: &Path,
    stat: &mut DirStat,
    depth: usize,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == META_FILE_NAME {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            accumulate_stat(&entry.path(), stat, depth + 1)?;
        } else if file_type.is_file() {
            stat.entries += 1;
            stat.bytes = stat
                .bytes
                .saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    Ok(())
}

/// 递归拷贝目录（临时目录也走这里）。符号链接与清单文件跳过。
pub fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    copy_tree_into(src, dest, 0)
}

fn copy_tree_into(src: &Path, dest: &Path, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == META_FILE_NAME {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let target = dest.join(&name);
        if file_type.is_dir() {
            fs::create_dir_all(&target)?;
            copy_tree_into(&entry.path(), &target, depth + 1)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 原子写 JSON：同目录临时文件 → fsync → rename。
fn write_json_atomic<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let parent = path.parent().ok_or(WallpaperError::Config)?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| WallpaperError::Config)?;
    serde_json::to_writer_pretty(&mut temp, value)
        .map_err(|_| WallpaperError::Config)?;
    temp.as_file_mut().write_all(b"\n")?;
    // rename 之前把内容刷到盘上：否则掉电后可能 rename 成功而内容是空的。
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| WallpaperError::Io(error.error))?;
    Ok(())
}

/// 读包内的清单文件；不存在或坏了都返回 `None`。
fn read_meta_file(dir: &Path) -> Option<WallpaperMeta> {
    let text = fs::read_to_string(dir.join(META_FILE_NAME)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 由 `project.json` 与目录统计拼出元数据。
/// 由 `project.json` 拼出元数据。
///
/// `workshop_id` 是导入时调用方给的工坊 id（`ImportOptions.workshop_id`），
/// 只在 `project.json` 自己没写 `workshopid` 时兜底用：实测 65 个 WE 工坊包里
/// 24 个没有这个字段，只信 `project.json` 会让那些包的 `meta.workshop_id`
/// 变成空串，「这个包来自哪个工坊条目」这条信息就永久丢了。
fn meta_from_project(
    id: &str,
    dir: &Path,
    project: &Value,
    stat: DirStat,
    tex: TexAudit,
    installed_at: i64,
    workshop_id: Option<&str>,
) -> WallpaperMeta {
    let kind = WallpaperType::from_project_type(
        project
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let title = project
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.to_string());
    let description = project
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tags: Vec<String> = project
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|tag| tag.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let raw_file = project
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut file_rel = pkg::safe_rel(&raw_file);
    let mut damaged = match &file_rel {
        Some(rel) => !dir.join(rel).is_file(),
        None => {
            !raw_file.is_empty()
                || matches!(kind, WallpaperType::Scene | WallpaperType::Video)
        }
    };

    // 声明的是编辑器工程（`file: "scene.json"`）而盘上只有容器 —— 这是**真实库里最常见**
    // 的形态，webwallgl 自己会回退容器，不该在这里判成坏包。
    // 判死的后果很直接：前端 `mode` 第一个条件 `meta.damaged` 就落到 poster，
    // 用户看到的是「一直只用缩略图」，而且永远进不了引擎（2026-10-25 实机 31/63 个包都是这种）。
    if damaged {
        match kind {
            WallpaperType::Scene if pkg::has_scene_container(dir) => {
                // 留空：容器由库按目录自己找（scene.pkg → scenes/scene.pkg → gifscene.pkg）
                file_rel = None;
                damaged = false;
            }
            WallpaperType::Video => {
                if let Some(name) = pkg::any_with_ext(dir, &[".mp4", ".webm", ".mov", ".m4v"]) {
                    file_rel = Some(name);
                    damaged = false;
                }
            }
            WallpaperType::Web => {
                if let Some(name) = pkg::any_with_ext(dir, &["index.html"]) {
                    file_rel = Some(name);
                    damaged = false;
                }
            }
            _ => {}
        }
    }

    let preview = project
        .get("preview")
        .and_then(Value::as_str)
        .map(str::trim)
        .and_then(pkg::safe_rel)
        .filter(|rel| dir.join(rel).is_file());

    WallpaperMeta {
        id: id.to_string(),
        kind,
        title,
        description,
        tags,
        workshop_id: project
            .get("workshopid")
            .map(value_to_string)
            .filter(|workshop_id| !workshop_id.is_empty())
            .or_else(|| {
                workshop_id
                    .map(str::trim)
                    .filter(|workshop_id| !workshop_id.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_default(),
        file: file_rel.unwrap_or(raw_file),
        preview,
        entries: stat.entries,
        bytes: stat.bytes,
        installed_at,
        has_properties: project
            .get("general")
            .and_then(|general| general.get("properties"))
            .is_some(),
        tex_audit: tex,
        damaged,
    }
}

/// `project.json` 里的 `workshopid` 可能是数字也可能是字符串。
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

/// 暂存目录名：进程内自增 + 纳秒时间戳，撞不上也不含用户可控内容。
fn staging_name() -> String {
    let counter = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("import-{nanos:x}-{counter:x}")
}

/// 当前时间（Unix 毫秒），与前端 `Date.now()` 对齐。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::pkg::{
        PKG_MAGIC_SCENE, test_package,
    };

    fn store() -> (tempfile::TempDir, WallpaperStore) {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = WallpaperStore::new(temp.path().join("wallpapers"));
        store.ensure_dirs().expect("ensure dirs");
        (temp, store)
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, bytes).expect("write");
    }

    /// 一个最小的场景类工程目录。
    fn scene_project(root: &Path, title: &str) {
        write(
            &root.join("project.json"),
            format!(
                r#"{{"title":"{title}","type":"scene","file":"scene.json","preview":"preview.jpg","tags":["风景"]}}"#
            )
            .as_bytes(),
        );
        write(&root.join("scene.json"), b"{\"objects\":[]}");
        write(&root.join("preview.jpg"), b"\xff\xd8\xffjpeg");
        write(&root.join("materials/a.json"), b"{}");
    }

    #[test]
    fn imports_a_directory_form_wallpaper() {
        let (temp, store) = store();
        let source = temp.path().join("workshop-entry");
        scene_project(&source, "river");

        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");

        assert!(!outcome.already_installed);
        assert!(outcome.meta.id.starts_with("local-river-"));
        assert_eq!(outcome.meta.kind, WallpaperType::Scene);
        assert_eq!(outcome.meta.title, "river");
        assert_eq!(outcome.meta.file, "scene.json");
        assert_eq!(outcome.meta.preview.as_deref(), Some("preview.jpg"));
        assert_eq!(outcome.meta.tags, vec!["风景".to_string()]);
        assert_eq!(outcome.meta.entries, 4);
        assert!(outcome.meta.bytes > 0);
        assert!(!outcome.meta.damaged);
        assert!(outcome.warnings.is_empty());

        let dir = store.package_dir(&outcome.meta.id).expect("package dir");
        assert!(dir.join("scene.json").is_file());
        assert!(dir.join(META_FILE_NAME).is_file());
        assert!(store
            .posters_dir()
            .join(format!("{}.jpg", outcome.meta.id))
            .is_file());
        // 暂存目录清空了。
        assert_eq!(
            fs::read_dir(store.incoming_dir()).expect("read").count(),
            0
        );
    }

    #[test]
    fn importing_the_same_package_twice_keeps_one_copy() {
        let (temp, store) = store();
        let source = temp.path().join("workshop-entry");
        scene_project(&source, "river");

        let first = store
            .import(&source, &ImportOptions::default())
            .expect("first import");
        let second = store
            .import(&source, &ImportOptions::default())
            .expect("second import");

        assert!(!first.already_installed);
        assert!(second.already_installed);
        assert_eq!(first.meta.id, second.meta.id);
        assert_eq!(
            fs::read_dir(store.packages_dir()).expect("read").count(),
            1,
            "二次导入不该多出一个包目录"
        );
        assert_eq!(store.list().expect("list").len(), 1);
    }

    #[test]
    fn imports_a_container_package() {
        let (temp, store) = store();
        let source = temp.path().join("my-video.mpkg");
        write(
            &source,
            &test_package(
                "PKGM0019",
                &[
                    (
                        "project.json",
                        br#"{"title":"beach","type":"video","file":"beach.mp4"}"#,
                    ),
                    ("beach.mp4", b"\x00\x00\x00\x18ftypmp42"),
                ],
            ),
        );

        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");

        assert_eq!(outcome.meta.kind, WallpaperType::Video);
        assert_eq!(outcome.meta.title, "beach");
        assert!(outcome.meta.id.starts_with("local-beach-"));
        assert!(!outcome.meta.damaged);
        let dir = store.package_dir(&outcome.meta.id).expect("dir");
        assert!(dir.join("beach.mp4").is_file());
        // 容器本身不该被写进包里。
        assert!(!dir.join("my-video.mpkg").exists());
    }

    #[test]
    fn imports_a_scene_only_package_by_synthesising_project_json() {
        let (temp, store) = store();
        let source = temp.path().join("987654.mpkg");
        write(
            &source,
            &test_package(
                PKG_MAGIC_SCENE,
                &[
                    ("scene.json", b"{\"objects\":[]}"),
                    ("preview.jpg", b"\xff\xd8\xff"),
                ],
            ),
        );

        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");

        assert_eq!(outcome.meta.kind, WallpaperType::Scene);
        // 合成的清单用文件名当标题。
        assert_eq!(outcome.meta.title, "987654");
        assert_eq!(outcome.meta.file, "scene.json");
        assert_eq!(outcome.meta.preview.as_deref(), Some("preview.jpg"));

        let dir = store.package_dir(&outcome.meta.id).expect("dir");
        let project: Value = serde_json::from_slice(
            &fs::read(dir.join("project.json")).expect("read"),
        )
        .expect("parse");
        assert_eq!(project["type"], "scene");
    }

    #[test]
    fn workshop_packages_land_in_the_workshop_namespace() {
        let (temp, store) = store();
        let source = temp.path().join("scene.pkg");
        write(
            &source,
            &test_package(
                PKG_MAGIC_SCENE,
                &[("scene.json", b"{\"objects\":[]}")],
            ),
        );

        let outcome = store
            .import(
                &source,
                &ImportOptions {
                    workshop_id: Some("1234567890".to_string()),
                },
            )
            .expect("import");

        assert!(
            outcome.meta.id.starts_with("ws-1234567890-"),
            "unexpected id {}",
            outcome.meta.id
        );
        assert_eq!(outcome.meta.workshop_id, "1234567890");
    }

    #[test]
    fn expands_a_scene_pkg_inside_a_workshop_directory() {
        let (temp, store) = store();
        let source = temp.path().join("workshop-entry");
        write(
            &source.join("project.json"),
            br#"{"title":"packed","type":"scene","file":"scene.json"}"#,
        );
        write(
            &source.join("scene.pkg"),
            &test_package(
                PKG_MAGIC_SCENE,
                &[("scene.json", b"{\"objects\":[1]}")],
            ),
        );

        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");

        let dir = store.package_dir(&outcome.meta.id).expect("dir");
        assert!(dir.join("scene.json").is_file(), "scene.pkg 应就地展开");
        assert!(!dir.join("scene.pkg").exists(), "容器不该留下");
        assert!(!outcome.meta.damaged);
    }

    #[test]
    fn a_directory_without_project_json_is_rejected() {
        let (temp, store) = store();
        let source = temp.path().join("random-folder");
        write(&source.join("hello.txt"), b"hi");

        let error = store
            .import(&source, &ImportOptions::default())
            .expect_err("must reject");

        assert!(matches!(error, WallpaperError::MissingProject));
        assert_eq!(error.to_string(), "壁纸包里没有 project.json，无法识别");
        // 失败不留半成品。
        assert_eq!(
            fs::read_dir(store.packages_dir()).expect("read").count(),
            0
        );
        assert_eq!(
            fs::read_dir(store.incoming_dir()).expect("read").count(),
            0
        );
    }

    #[test]
    fn a_missing_source_is_reported_plainly() {
        let (_temp, store) = store();
        let error = store
            .import(Path::new("D:/nope/wallpaper.mpkg"), &ImportOptions::default())
            .expect_err("must reject");

        assert!(matches!(error, WallpaperError::SourceMissing));
        assert!(!error.to_string().contains("D:"));
    }

    #[test]
    fn removing_a_package_deletes_the_directory_and_the_poster() {
        let (temp, store) = store();
        let source = temp.path().join("workshop-entry");
        scene_project(&source, "river");
        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");
        let id = outcome.meta.id.clone();
        store.select(Some(&id)).expect("select");
        assert_eq!(
            store.read_config().expect("config").selected.as_deref(),
            Some(id.as_str())
        );

        store.remove(&id).expect("remove");

        assert!(!store.package_dir(&id).expect("dir").exists());
        assert!(!store
            .posters_dir()
            .join(format!("{id}.jpg"))
            .exists());
        let config = store.read_config().expect("config");
        assert_eq!(config.selected, None);
        assert!(store.list().expect("list").is_empty());
        // 幂等：再删一次也不报错。
        store.remove(&id).expect("second remove is a no-op");
    }

    #[test]
    fn rejecting_a_bad_id_keeps_the_path_out_of_the_store() {
        let (_temp, store) = store();
        for id in ["../escape", "..", "", "a/b", "a\\b", "C:x", ".hidden"] {
            assert!(
                matches!(
                    store.package_dir(id).expect_err("must reject"),
                    WallpaperError::InvalidId
                ),
                "{id:?} should be rejected"
            );
        }
        assert!(store.package_dir("local-river-0123456789ab").is_ok());
    }

    #[test]
    fn selecting_an_unknown_package_is_rejected() {
        let (_temp, store) = store();
        let error = store
            .select(Some("local-missing-000000000000"))
            .expect_err("must reject");
        assert!(matches!(error, WallpaperError::UnknownPackage));
    }

    #[test]
    fn config_writes_leave_no_partial_file_behind() {
        let (temp, store) = store();
        let mut config = WallpaperConfig::default();
        config.selected = Some("local-river-0123456789ab".to_string());

        store.write_config(&config).expect("first write");
        store.write_config(&config).expect("second write");

        let mut names: Vec<String> = fs::read_dir(temp.path().join("wallpapers"))
            .expect("read")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();

        // 只有四个目录 + config.json，没有 .tmp 残留。
        assert_eq!(
            names,
            vec![
                "config.json".to_string(),
                "incoming".to_string(),
                "packages".to_string(),
                "posters".to_string(),
            ]
        );
        assert_eq!(
            store.read_config().expect("read back").selected,
            config.selected
        );
    }

    #[test]
    fn a_corrupt_config_falls_back_to_defaults() {
        let (_temp, store) = store();
        write(&store.config_path(), b"{not json");

        let config = store.read_config().expect("must not fail");

        assert_eq!(config, WallpaperConfig::default());
    }

    #[test]
    fn list_marks_a_package_damaged_when_project_json_disappears() {
        let (temp, store) = store();
        let source = temp.path().join("workshop-entry");
        scene_project(&source, "river");
        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].damaged);

        let dir = store.package_dir(&outcome.meta.id).expect("dir");
        fs::remove_file(dir.join("project.json")).expect("rm project.json");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert!(listed[0].damaged, "清单没了要标损坏");
    }

    #[test]
    fn list_rebuilds_missing_metadata_from_the_directory() {
        let (temp, store) = store();
        let source = temp.path().join("workshop-entry");
        scene_project(&source, "river");
        let outcome = store
            .import(&source, &ImportOptions::default())
            .expect("import");

        let dir = store.package_dir(&outcome.meta.id).expect("dir");
        fs::remove_file(dir.join(META_FILE_NAME)).expect("rm meta");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, outcome.meta.id);
        assert_eq!(listed[0].title, "river");
        assert!(listed[0].preview.is_some());
    }

    #[test]
    fn hash_dir_is_stable_and_content_addressed() {
        let temp = tempfile::tempdir().expect("temp dir");
        let left = temp.path().join("left");
        let right = temp.path().join("right");
        scene_project(&left, "river");
        scene_project(&right, "river");

        let hash = hash_dir(&left).expect("hash left");
        assert_eq!(hash.len(), 40);
        assert_eq!(hash, hash_dir(&right).expect("hash right"));

        write(&right.join("materials/a.json"), b"{\"changed\":true}");
        assert_ne!(hash, hash_dir(&right).expect("hash right again"));
    }

    #[test]
    fn hash_dir_ignores_the_metadata_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let dir = temp.path().join("wallpaper");
        scene_project(&dir, "river");
        let before = hash_dir(&dir).expect("hash");

        write(&dir.join(META_FILE_NAME), b"{\"id\":\"x\"}");

        assert_eq!(hash_dir(&dir).expect("hash after"), before);
    }

    #[test]
    fn slugify_keeps_ids_ascii_and_bounded() {
        assert_eq!(slugify("River Flow"), "river-flow");
        assert_eq!(slugify("  多 空格  "), "wallpaper");
        assert_eq!(slugify("abc--def"), "abc-def");
        assert_eq!(slugify(&"a".repeat(200)).len(), MAX_SLUG_LEN);
        assert!(slugify("../../../etc/passwd").chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-'
        }));
    }

    #[test]
    fn ids_are_validated_strictly() {
        assert!(is_valid_id("local-river-0123456789ab"));
        assert!(is_valid_id("ws-9876543210-0123456789ab"));
        for id in [
            "",
            ".",
            "..",
            "../evil",
            "a/b",
            "a\\b",
            "a:b",
            "a b",
            ".hidden",
            &"x".repeat(MAX_ID_LEN + 1),
        ] {
            assert!(!is_valid_id(id), "{id:?} should be rejected");
        }
    }
}
