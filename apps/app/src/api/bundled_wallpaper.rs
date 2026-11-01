//! 内置壁纸：安装包自带一张，首次启动落到用户目录，并在没有选择时默认选中。
//!
//! 为什么不是「让 `list()` 直接扫资源目录」：包的一切写入路径（删除、换画质、
//! 写属性、协议取流）都假定它躺在 `wallpapers/packages/<id>/` 下。多一条只读路径
//! 要在每个写入点都加分支，而复制一次 21MB 只用几百毫秒、只发生一次。
//!
//! 三条硬规则：
//!
//! 1. **幂等**：目标目录在就直接返回，不做任何 IO（每次 `wallpaper_list` 都会调它）。
//! 2. **不覆盖**：用户的包如果已经在（哪怕是同一张），一律不碰 —— 他可能已经删过。
//! 3. **默认选中只做一次**：靠 `.bundled-seeded` 哨兵。否则用户主动选「纯色背景」，
//!    下次启动又会被塞回这张壁纸。

use std::fs;
use std::path::{Path, PathBuf};

use tauri::{Manager, Runtime};
use theseus::wallpaper::store::{copy_tree, WallpaperStore};
use theseus::wallpaper::Result as WallpaperResult;

/// 内置壁纸的包 id。
///
/// 它是「工坊 id + 内容哈希」—— 内容不变，id 就不会变，所以可以写死。
/// 资源目录的**名字**必须与它一致：`list()` 用目录名当 id。
pub const BUNDLED_ID: &str = "ws-3358649912-37d2571554a1";

/// 资源目录（相对 `resource_dir()`），与 `tauri.conf.json` 的 `bundle.resources` 对应。
const BUNDLED_RESOURCE_DIR: &str = "resources/bundled-wallpapers/arona";

/// 本地元数据。`copy_tree` 有意跳过它，所以要让内置包也带上（省一次 `tex_audit`）。
const META_FILE_NAME: &str = ".hayenai-meta.json";

/// 「默认选中已做过」的哨兵。
const SEEDED_MARKER: &str = ".bundled-seeded";

/// 内置包的资源目录。dev 下 `resource_dir()` 指向 `target/debug`，那边也会被同步，
/// 拿不到就返回 `None`，`seed()` 直接跳过 —— 绝不能让「内置壁纸缺失」挡住列表。
pub fn source_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    let dir = app.path().resource_dir().ok()?.join(BUNDLED_RESOURCE_DIR);
    dir.is_dir().then_some(dir)
}

/// 把内置壁纸落到用户目录（幂等），并做一次性的「默认选中」。
///
/// **同步 IO**，调用方负责放到 `spawn_blocking` 里。任何失败都只记日志：
/// 内置壁纸是体验优化，它坏掉不该让壁纸列表打不开。
pub fn seed(store: &WallpaperStore, source: Option<&Path>) {
    let Some(source) = source else {
        return;
    };

    if let Err(error) = ensure_package(store, source) {
        tracing::warn!(?error, "内置壁纸落盘失败，本次跳过");
        return;
    }

    mark_and_select_once(store);
}

/// 目标目录不存在时才复制。先落 `incoming/` 再原子改名 ——
/// 直接往 `packages/<id>` 里拷，中途失败会被 `list()` 当成「包损坏」标红。
fn ensure_package(store: &WallpaperStore, source: &Path) -> WallpaperResult<()> {
    let dest = store.package_dir(BUNDLED_ID)?;
    if dest.is_dir() {
        return Ok(());
    }

    store.ensure_dirs()?;
    let staging = store.incoming_dir().join("bundled-arona");
    let _ = fs::remove_dir_all(&staging);

    copy_tree(source, &staging)?;

    // `copy_tree` 跳过 `.hayenai-meta.json`（import 场景要重新算 id），这里补上。
    let meta = source.join(META_FILE_NAME);
    if meta.is_file() {
        fs::copy(&meta, staging.join(META_FILE_NAME))?;
    }

    fs::rename(&staging, &dest)?;
    tracing::info!(id = BUNDLED_ID, "内置壁纸已落盘");
    Ok(())
}

/// 只做一次：当profile里没有选中任何壁纸时，把内置那张选上。
fn mark_and_select_once(store: &WallpaperStore) {
    let marker = store.root().join(SEEDED_MARKER);
    if marker.is_file() {
        return;
    }

    let already_chosen = store
        .read_config()
        .map(|config| config.selected.is_some())
        .unwrap_or(false);

    if !already_chosen {
        match store.select(Some(BUNDLED_ID)) {
            Ok(()) => tracing::info!(id = BUNDLED_ID, "默认选中内置壁纸"),
            Err(error) => tracing::warn!(?error, "默认选中内置壁纸失败"),
        }
    }

    // 无论这次有没有真的选中，哨兵都要写：用户主动清空壁纸后不该被反复改回。
    if let Err(error) = fs::write(&marker, b"1") {
        tracing::warn!(?error, "写内置壁纸哨兵失败");
    }
}
