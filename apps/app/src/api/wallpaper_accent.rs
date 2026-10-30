//! 壁纸主色采样（Rust 侧）
//!
//! 为什么不在前端做：壁纸资源走 `hayenai-we.localhost` 自定义协议，
//! 页面里 `fetch` 直接被拒（Failed to fetch），`<img>`/`<video>` 虽然能画出来，
//! 但 canvas 会被跨源污染，`getImageData` 必抛。所以取色只能在 Rust 侧读文件。
//!
//! 算法与后台 `lib/accent.ts` 对齐：缩到 32×32 做面积统计，取占比最高的一族颜色；
//! 整体亮度单独返回（决定深浅色档），与主色无关。
//!
//! 只读 `project.json` 里声明的 `preview`，没有就找 `preview.*`。读不到就返回 None ——
//! 取色失败不该影响任何东西，调用方保持用户原来的主题即可。

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Serialize)]
pub struct WallpaperAccent {
    /// 十六进制主色，如 `#3a6ea5`
    pub color: String,
    /// 整体平均亮度 0-1（<0.5 偏暗）
    pub luminance: f32,
}

const SAMPLE: u32 = 48;

#[tauri::command]
pub async fn wallpaper_accent<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<Option<WallpaperAccent>, String> {
    let Some(root) = crate::we_protocol::root_for(&app) else {
        return Ok(None);
    };
    let sampled = tauri::async_runtime::spawn_blocking(move || sample_package(&root, &id))
        .await
        .map_err(|_| "accent task interrupted".to_string())?;
    Ok(sampled)
}

fn sample_package(root: &Path, id: &str) -> Option<WallpaperAccent> {
    let dir = root.join("packages").join(id);
    let preview = preview_path(&dir)?;
    let image = image::open(&preview).ok()?;
    let small = image.resize_exact(SAMPLE, SAMPLE, image::imageops::FilterType::Triangle);
    let rgb = small.to_rgb8();

    // 按**色相**分 24 桶统计（2026-10-24 22:50，对齐后台 `lib/accent.ts`）。
    // 原来按 RGB 4bit 分格：同一色的不同明暗被拆成几百格，最终挑中的往往是某一块
    // 具体颜色（动漫壁纸里就是一片肤色），当强调色又脏又怪 —— 主人说的"取色不准"。
    const BUCKETS: usize = 24;
    /// 无彩色阈值（chroma）。低于它的像素色相纯属数值噪声：弃权，但面积照记（亮度要用）。
    const NEUTRAL_C: f32 = 0.055;
    /// 主色最小占比：连「有色像素」的一成都占不到，就算没有主色。
    const MIN_SHARE: f64 = 0.12;

    let mut vote = [0f64; BUCKETS];
    let mut hsum = [0f64; BUCKETS];
    let mut csum = [0f64; BUCKETS];
    let mut lsum = [0f64; BUCKETS];
    let mut colored = 0f64;
    // 收全部亮度值：判深浅要用**中位数**，不能用平均值。
    // 「夜空 + 一大块亮云」这类画面，平均值会被亮云拉高，于是暗壁纸被判成浅色档，
    // 深色字压在夜空上直接消失（主人 02:29 / 03:30 两次报的就是这个）。
    let mut lums: Vec<f32> = Vec::with_capacity(SAMPLE as usize * SAMPLE as usize);

    for pixel in rgb.pixels() {
        let [r, g, b] = pixel.0;

        // Rec.709 线性亮度（0–1），逐像素收集后取中位数
        lums.push((0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) as f32 / 255.0);

        let rf = r as f32 / 255.0;
        let gf = g as f32 / 255.0;
        let bf = b as f32 / 255.0;
        let chroma = rf.max(gf).max(bf) - rf.min(gf).min(bf);
        if chroma < NEUTRAL_C {
            continue;
        }

        let (h, _s, l) = rgb_to_hsl([r, g, b]);
        let bi = ((h / 360.0 * BUCKETS as f32) as usize).min(BUCKETS - 1);
        vote[bi] += 1.0;
        hsum[bi] += h as f64;
        csum[bi] += chroma as f64;
        lsum[bi] += l as f64;
        colored += 1.0;
    }

    let luminance = if lums.is_empty() {
        0.5
    } else {
        lums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        lums[lums.len() / 2]
    };

    // 3 桶环形平滑：暖色跨 0°/360° 会被劈进两桶、各自减半而输给噪声。
    let mut smooth = [0f64; BUCKETS];
    for b in 0..BUCKETS {
        smooth[b] = vote[b]
            + 0.5 * vote[(b + BUCKETS - 1) % BUCKETS]
            + 0.5 * vote[(b + 1) % BUCKETS];
    }
    let mut best: Option<usize> = None;
    for b in 0..BUCKETS {
        if smooth[b] > 0.0 && best.is_none_or(|cur| smooth[b] > smooth[cur]) {
            best = Some(b);
        }
    }

    // 没有成规模的主色：返回空色 —— 前端只跟着明暗档走，不硬塞一个杂交色。
    let Some(bi) = best.filter(|_| colored >= 12.0 && smooth[best.unwrap()] >= colored * MIN_SHARE)
    else {
        return Some(WallpaperAccent {
            color: String::new(),
            luminance,
        });
    };

    let h = (hsum[bi] / vote[bi]) as f32;
    let l = ((lsum[bi] / vote[bi]) as f32).clamp(0.40, 0.68);
    // 输出色从**平均彩度**还原，而不是平均 HSL 饱和度（后者在浅色区虚高，
    // 一片暖白会被渲染成荧光橙）。×1.3 是"要能当光用"的抬色，下限 0.34。
    let c = (csum[bi] / vote[bi]) as f32;
    let sat = ((c / (1.0 - (2.0 * l - 1.0).abs())) * 1.3).clamp(0.34, 1.0);

    let [r, g, b] = hsl_to_rgb(h, sat, l);
    Some(WallpaperAccent {
        color: format!("#{r:02x}{g:02x}{b:02x}"),
        luminance,
    })
}

/// RGB(0-255) → HSL，H 为 0-360，S/L 为 0-1。
fn rgb_to_hsl([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } * 60.0;
    (h, s, l)
}

/// HSL → RGB(0-255)。
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    [to_u8(r1), to_u8(g1), to_u8(b1)]
}

fn chroma(rgb: [u64; 3]) -> u64 {
    let max = rgb.iter().max().copied().unwrap_or(0);
    let min = rgb.iter().min().copied().unwrap_or(0);
    max - min
}

/// 包内预览图：优先 `project.json` 的 `preview` 字段，否则在目录里找 `preview.*`。
fn preview_path(dir: &Path) -> Option<PathBuf> {
    let project = dir.join("project.json");
    if let Ok(text) = std::fs::read_to_string(&project) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(preview) = value.get("preview").and_then(|v| v.as_str()) {
                let candidate = dir.join(preview);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if name.starts_with("preview.")
            && matches!(
                path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
                Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
            )
        {
            return Some(path);
        }
    }
    None
}
