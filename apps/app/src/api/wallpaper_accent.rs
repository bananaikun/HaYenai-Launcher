//! 壁纸主色采样 + 明暗判定数据（Rust 侧）
//!
//! 为什么不在前端做：壁纸资源走 `hayenai-we.localhost` 自定义协议，
//! 页面里 `fetch` 直接被拒（Failed to fetch），`<img>`/`<video>` 虽然能画出来，
//! 但 canvas 会被跨源污染，`getImageData` 必抛。所以取色只能读文件。
//!
//! 职责边界（2026-11-01 重构）：
//! - **这里只负责「把像素变成可判定的数据」**：解一帧或几帧，缩成一块低频亮度网格，
//!   跨帧逐格取最大/最小，连同主色一起回传。
//! - **判定在前端做**（`helpers/wallpaper-theme.ts`）：因为判据要用到窗口宽高比、
//!   文字真实所在的 DOM 区域、以及用户当前的玻璃浓度 —— 这些只有浏览器知道，
//!   而且窗口一改尺寸就要重算，放前端才能免解码重算。
//!
//! 为什么不是「回传一个亮度标量」：标量无法表达「文字压在哪一块」，也无法表达
//! 「这块里最亮/最暗是多少」。实测 71 张壁纸，用全图中位亮度会判错 19 张（27%）——
//! 其中 18 张是该用深字却给了白字，最坏对比度低到 1.15:1。
//!
//! ⚠ 网格必须**按原图比例**缩放，不能压成正方形：判定要按窗口做 cover 裁切，
//! 压扁就丢了比例信息（旧实现正是压成 48×48，判的是窗口里根本看不到的区域）。

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use image::AnimationDecoder;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperAccent {
    /// 十六进制主色，如 `#3a6ea5`；无成规模主色时为空串
    pub color: String,
    /// 全图中位亮度（0–1 线性）。仅作日志/兜底，判定不再用它
    pub luminance: f32,
    pub grid_w: u32,
    pub grid_h: u32,
    /// 逐格 WCAG 相对亮度 ×255，**跨帧取最大**（白字的最坏情况看这里）
    pub lum_max: Vec<u8>,
    /// 逐格 WCAG 相对亮度 ×255，**跨帧取最小**（深字的最坏情况看这里）
    pub lum_min: Vec<u8>,
}

/// 网格长边格数。64 保证「左侧栏一条竖带」在 16:9 下仍有约 5×34 个采样点，
/// 同时把回传体积压在 ~18KB JSON（只在切壁纸时传一次）。
const GRID: u32 = 64;
/// 动图最多解这么多帧。预览图通常只有几百见方，120 帧的解码成本可以忽略；
/// 设上限是防极端长动图（几千帧）把切壁纸拖慢。
const MAX_FRAMES: usize = 120;

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

    let mut sampler: Option<Sampler> = None;

    if is_gif(&preview) {
        if let Ok(file) = File::open(&preview) {
            if let Ok(decoder) = image::codecs::gif::GifDecoder::new(BufReader::new(file)) {
                for frame in decoder.into_frames().take(MAX_FRAMES) {
                    let Ok(frame) = frame else { break };
                    let buf = frame.buffer();
                    sampler.get_or_insert_with(|| Sampler::new(buf.width(), buf.height()))
                        .add(buf);
                }
            }
        }
    }

    // 非动图，或动图解码一帧都没成：退回单帧
    if sampler.is_none() {
        let image = image::open(&preview).ok()?.to_rgb8();
        sampler = Some(Sampler::new(image.width(), image.height()));
        sampler.as_mut()?.add(&image);
    }

    sampler?.finish()
}

fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// sRGB 分量 → 线性（WCAG 2.x 的定义）
fn srgb_to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 相对亮度：**逐通道线性化再加权**，不是先加权再线性化。
/// 差别在饱和色上很大（纯蓝按后者算会低一个数量级），所以必须按通道来。
fn wcag_lum(r: u8, g: u8, b: u8) -> f32 {
    0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
}

/// 按色相分 24 桶统计主色（与后台 `lib/accent.ts` 对齐）。
const BUCKETS: usize = 24;
/// 无彩色阈值（chroma）。低于它的像素色相纯属数值噪声：弃权，但面积照记。
const NEUTRAL_C: f32 = 0.055;
/// 主色最小占比：连「有色像素」的一成都占不到，就算没有主色。
const MIN_SHARE: f64 = 0.12;

struct Sampler {
    gw: u32,
    gh: u32,
    lum_max: Vec<u8>,
    lum_min: Vec<u8>,
    /// 还没有任何一帧写入
    empty: bool,
    vote: [f64; BUCKETS],
    hsum: [f64; BUCKETS],
    csum: [f64; BUCKETS],
    lsum: [f64; BUCKETS],
    colored: f64,
    /// 首帧的逐格亮度，用于 `luminance`（中位）
    first_lums: Vec<f32>,
}

impl Sampler {
    fn new(w: u32, h: u32) -> Self {
        let (gw, gh) = if w >= h {
            (GRID, ((h as u64 * GRID as u64 / w.max(1) as u64) as u32).max(1))
        } else {
            (((w as u64 * GRID as u64 / h.max(1) as u64) as u32).max(1), GRID)
        };
        let n = (gw * gh) as usize;
        Self {
            gw,
            gh,
            lum_max: vec![0; n],
            lum_min: vec![u8::MAX; n],
            empty: true,
            vote: [0.0; BUCKETS],
            hsum: [0.0; BUCKETS],
            csum: [0.0; BUCKETS],
            lsum: [0.0; BUCKETS],
            colored: 0.0,
            first_lums: Vec::new(),
        }
    }

    /// 收一帧。泛型是因为 GIF 帧是 RGBA、静态图是 RGB —— 内部统一按 `to_rgb()` 取色。
    fn add<P>(&mut self, img: &image::ImageBuffer<P, Vec<u8>>)
    where
        P: image::Pixel<Subpixel = u8> + 'static,
    {
        let small = image::imageops::resize(
            img,
            self.gw,
            self.gh,
            image::imageops::FilterType::Triangle,
        );

        let first_frame = self.empty;
        for (i, pixel) in small.pixels().enumerate() {
            let [r, g, b] = pixel.to_rgb().0;
            let lum = wcag_lum(r, g, b);
            let q = (lum * 255.0).round().clamp(0.0, 255.0) as u8;

            if first_frame {
                self.lum_max[i] = q;
                self.lum_min[i] = q;
                self.first_lums.push(lum);
            } else {
                if q > self.lum_max[i] {
                    self.lum_max[i] = q;
                }
                if q < self.lum_min[i] {
                    self.lum_min[i] = q;
                }
            }

            // 主色投票：无彩色像素只记面积（亮度已单独处理），不参与色相
            let rf = r as f32 / 255.0;
            let gf = g as f32 / 255.0;
            let bf = b as f32 / 255.0;
            let chroma = rf.max(gf).max(bf) - rf.min(gf).min(bf);
            if chroma < NEUTRAL_C {
                continue;
            }
            let (h, _s, l) = rgb_to_hsl([r, g, b]);
            let bi = ((h / 360.0 * BUCKETS as f32) as usize).min(BUCKETS - 1);
            self.vote[bi] += 1.0;
            self.hsum[bi] += h as f64;
            self.csum[bi] += chroma as f64;
            self.lsum[bi] += l as f64;
            self.colored += 1.0;
        }
        self.empty = false;
    }

    fn finish(self) -> Option<WallpaperAccent> {
        if self.empty {
            return None;
        }

        let median = {
            let mut lums = self.first_lums;
            if lums.is_empty() {
                0.5
            } else {
                lums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                lums[lums.len() / 2]
            }
        };

        // 3 桶环形平滑：暖色跨 0°/360° 会被劈进两桶、各自减半而输给噪声。
        let mut smooth = [0f64; BUCKETS];
        for b in 0..BUCKETS {
            smooth[b] = self.vote[b]
                + 0.5 * self.vote[(b + BUCKETS - 1) % BUCKETS]
                + 0.5 * self.vote[(b + 1) % BUCKETS];
        }
        let mut best: Option<usize> = None;
        for b in 0..BUCKETS {
            if smooth[b] > 0.0 && best.is_none_or(|cur| smooth[b] > smooth[cur]) {
                best = Some(b);
            }
        }

        let color = match best.filter(|_| {
            self.colored >= 12.0 && smooth[best.unwrap()] >= self.colored * MIN_SHARE
        }) {
            None => String::new(),
            Some(bi) => {
                let h = (self.hsum[bi] / self.vote[bi]) as f32;
                let l = ((self.lsum[bi] / self.vote[bi]) as f32).clamp(0.40, 0.68);
                // 输出色从**平均彩度**还原，而不是平均 HSL 饱和度（后者在浅色区虚高，
                // 一片暖白会被渲染成荧光橙）。×1.3 是"要能当光用"的抬色，下限 0.34。
                let c = (self.csum[bi] / self.vote[bi]) as f32;
                let sat = ((c / (1.0 - (2.0 * l - 1.0).abs())) * 1.3).clamp(0.34, 1.0);
                let [r, g, b] = hsl_to_rgb(h, sat, l);
                format!("#{r:02x}{g:02x}{b:02x}")
            }
        };

        Some(WallpaperAccent {
            color,
            luminance: median,
            grid_w: self.gw,
            grid_h: self.gh,
            lum_max: self.lum_max,
            lum_min: self.lum_min,
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcag_black_and_white_are_the_extremes() {
        assert!((wcag_lum(0, 0, 0) - 0.0).abs() < 1e-6);
        assert!((wcag_lum(255, 255, 255) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn wcag_luminance_linearises_per_channel() {
        // 纯蓝：先加权再线性化会得到 ~0.005，正确值是 0.0722（相差一个数量级）
        let blue = wcag_lum(0, 0, 255);
        assert!((blue - 0.0722).abs() < 5e-3, "blue = {blue}");
    }

    #[test]
    fn grid_keeps_source_aspect() {
        let wide = Sampler::new(1920, 1080);
        assert_eq!((wide.gw, wide.gh), (64, 36));
        let tall = Sampler::new(1080, 1920);
        assert_eq!((tall.gw, tall.gh), (36, 64));
        let square = Sampler::new(1067, 1067);
        assert_eq!((square.gw, square.gh), (64, 64));
    }

    #[test]
    fn payload_uses_the_field_names_the_frontend_reads() {
        // 前端 `AccentSample` 读的是 gridW / gridH / lumMax / lumMin —— 少一个就是静默 undefined，
        // 判定会退回「全图」，界面上看不出报错但结论会变差。
        let value = serde_json::to_value(WallpaperAccent {
            color: "#112233".into(),
            luminance: 0.5,
            grid_w: 2,
            grid_h: 1,
            lum_max: vec![1, 2],
            lum_min: vec![0, 3],
        })
        .expect("serializes");

        for key in ["color", "luminance", "gridW", "gridH", "lumMax", "lumMin"] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
        assert_eq!(value["gridW"], 2);
        assert_eq!(value["lumMax"][1], 2);
    }

    #[test]
    fn multi_frame_accumulates_worst_case_per_cell() {
        let mut sampler = Sampler::new(64, 64);
        sampler.add(&image::RgbImage::from_pixel(64, 64, image::Rgb([0, 0, 0])));
        sampler.add(&image::RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255])));

        let out = sampler.finish().expect("two frames sampled");
        // 白帧贡献 max=255，黑帧贡献 min=0
        assert!(out.lum_max.iter().all(|&v| v == 255));
        assert!(out.lum_min.iter().all(|&v| v == 0));
        // luminance 取首帧（黑）
        assert!(out.luminance < 0.01);
    }
}
