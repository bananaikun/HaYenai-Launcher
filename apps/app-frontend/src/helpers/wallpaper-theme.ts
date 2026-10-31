/**
 * 随壁纸取色 / 随壁纸深浅
 *
 * 后台管理那套（`服务器端/src/lib/accent.ts`：按面积统计主色 + 一帧平均亮度）
 * 原样搬过来了。这里只做「采样结果 → 启动器主题」的接线：
 *
 * - 取色：把主色转成十六进制，交给 `themeStore.setAccentColor()`。
 *   走它而不是自己写 `--c-a`：强调色在启动器里是 `accent-custom` + 明暗两档变体的组合，
 *   自己写根变量只会改到外壳，按钮/滑块仍然是旧的。
 * - 深浅：用采样帧的平均亮度决定挂 `light-mode` 还是 `dark-mode`。
 *
 * 三个开关都在 设置 → 外观 → 界面玻璃 里（localStorage，切完立即生效）。
 *
 * ⚠ 采样在 **Rust 侧**完成（`wallpaper_accent` 命令）：
 * 壁纸资源走 `hayenai-we.localhost` 自定义协议，页面里 `fetch` 被拒、canvas 被跨源污染，
 * 前端根本读不到像素。Rust 直接读包里的 `preview.*` 解码，顺带把亮度一起算好。
 */
import { invoke } from '@tauri-apps/api/core'
import { update as updateSettings } from '@/helpers/settings'
import { selectedWallpaper } from '@/helpers/wallpaper'

import { useTheming } from '@/store/state'

export type WallpaperThemeSettings = {
    /** 强调色跟随壁纸主色 */
    accent: boolean
    /** 深浅档跟随壁纸亮度 */
    scheme: boolean
}

const STORAGE_KEY = 'hayenai:wallpaper-theme'
const DEFAULTS: WallpaperThemeSettings = { accent: true, scheme: true }

function read(): WallpaperThemeSettings {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return { ...DEFAULTS }
        const parsed = JSON.parse(raw) as Partial<WallpaperThemeSettings>
        return {
            accent: parsed.accent ?? DEFAULTS.accent,
            scheme: parsed.scheme ?? DEFAULTS.scheme,
        }
    } catch {
        return { ...DEFAULTS }
    }
}

export const wallpaperTheme: WallpaperThemeSettings = read()

export function setWallpaperTheme(patch: Partial<WallpaperThemeSettings>): void {
    if (patch.accent !== undefined) wallpaperTheme.accent = patch.accent
    if (patch.scheme !== undefined) wallpaperTheme.scheme = patch.scheme
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(wallpaperTheme))
    } catch {
        // 隐私模式下不落盘，本次会话内仍生效
    }

    /* ⚠ 必须**立即重新应用**（2026-10-25 11:50）。
       原来这里只改状态 + 落盘，谁都不会去调 `applyWallpaperTheme` ——
       于是设置里点「主题色跟随壁纸 / 深浅色跟随壁纸」两个开关毫无反应，
       要等下一次切壁纸才生效。这条正是主人反复报的「开关闭选项不会立即生效」。 */
    void applyWallpaperTheme(selectedWallpaper()?.id ?? null)
}


type AccentResult = { color: string; luminance: number } | null

/**
 * 按当前开关把壁纸主题应用一次。
 * `id` 为空（没有壁纸）时直接返回，不动用户自己选的主题。
 */
export async function applyWallpaperTheme(id: string | null | undefined): Promise<void> {
    if (!id) {
        delete document.documentElement.dataset.wallpaperTone
        return
    }
    /* ⚠ 这里**不能**再按 accent/scheme 两个开关提前返回（2026-10-25 08:45）。
       实机症状：壁纸是亮的，字却是白的、压在亮壁纸上发淡 —— 因为主人关掉了
       「随壁纸取色 / 随壁纸切深浅」，函数在那行直接 return，`data-wallpaper-tone`
       从来没被写过，字体色落回默认白族。
       字色是**可读性底线**，必须与那两个开关解耦：取一次亮度 → 写 tone，总是执行；
       下面 accent 的部分才受开关控制。 */
    let sample: AccentResult = null
    try {
        sample = await invoke<AccentResult>('wallpaper_accent', { id })
    } catch {
        // 命令缺失或读图失败：保持用户自己的主题，不报错
        return
    }
    if (!sample) return

    const themeStore = useTheming()

    /* 先判三档，后面所有东西都跟着它走：
         dark    —— 壁纸暗，字体必须是白的
         light   —— 壁纸亮，字体必须是深的
         neutral —— 中间调，**不切主题**，靠光晕保证可读
       `luminance` 是 Rust 侧的**中位亮度**（0–1，线性 Rec.709）。
       用中位数而不是平均：平均会被「夜空里的亮云」拉高，把暗壁纸判成浅色。 */
    const tone: WallpaperTone = !Number.isFinite(sample.luminance)
        ? 'neutral'
        : sample.luminance < LUMINANCE_DARK_MAX
          ? 'dark'
          : sample.luminance > LUMINANCE_LIGHT_MIN
            ? 'light'
            : 'neutral'

    // 字体族完全跟着壁纸走（与主题档、与取色开关都解耦）
    document.documentElement.dataset.wallpaperTone = tone

    // 取色/切档两个开关都关时，字色已经设置完，到此为止
    if (!wallpaperTheme.accent && !wallpaperTheme.scheme) return

    const scheme: 'dark' | 'light' =
        tone === 'dark'
            ? 'dark'
            : tone === 'light'
              ? 'light'
              : document.documentElement.classList.contains('light-mode')
                ? 'light'
                : 'dark'

    /* 2026-10-25 07:00 主人定：**只切文字色，不切主题档**（不碰背景/模糊）。
       以前这里调 `setThemeState(tone)` 会把整站换成浅/深主题 ——
       那等于连玻璃一起换色，主人明确禁止。文字色由 CSS 的
       `[data-wallpaper-tone]` 决定，`scheme` 只用于强调色钳位。 */

    // 必须带 `custom:` 前缀：setAccentColor 用 parseCustomAccentColor 校验，
    // 裸 `#rrggbb` 会被判为非法值然后静默退回预设色（上一版就是这个问题）。
    if (wallpaperTheme.accent && /^#[0-9a-fA-F]{6}$/.test(sample.color)) {
        const hex = clampAccentForScheme(sample.color, scheme)
        themeStore.setAccentColor(`custom:${hex}`)

        /* 2026-10-30 19:50 主人：「主题色总是没有跟随壁纸，而是跟随系统」。
           根因：`setAccentColor` **只改内存里的 `selectedAccentColor` + CSS 变量，不写 settings**。
           以前是"设置 → 外观 → 强调色"那块 UI 负责把值写回 `settings.accent_color`，
           而我 10-25 按主人要求把强调色整块删掉了 —— 于是再没人持久化。
           ⇒ 每次启动 `applyThemeFromSettings` 都会用 settings 里的 `'system'`
           把刚取到的壁纸色**覆盖**掉。
           这里补上持久化：取到色就写回 settings，下次启动直接就是壁纸色。 */
        void updateSettings({ accent_color: `custom:${hex}` }).catch((error) => {
            console.warn('[wallpaper-theme] 写回强调色失败', error)
        })
    }
}

/* ------------------------------------------------------------------ *
 * 强调色的「可读性钳位」
 * ------------------------------------------------------------------ *
 * Rust 侧只按图片自身把 HSL 收进 0.40–0.68 / 0.34–1.0，并不知道界面此刻是
 * 深色档还是浅色档。于是会出现「深色档 + 亮度 0.40 的暗色」这种事：强调色
 * 压在深色玻璃上与背景只差一点点，用它上色的文字（链接、选中项、图标）几乎
 * 看不见 —— 主人 00:01 报的「完全看不到字」就是这一类。
 *
 * 这里按最终落地的档位再收一次：深色档保证亮，浅色档保证暗。
 */
/**
 * 线性平均亮度低于它就算「深色壁纸」。
 *
 * 0.18 太靠暗了：MC 那种「大片灰黑 + 一块亮天空」的壁纸，线性平均会被亮块拉到 0.25~0.32，
 * 于是被判成浅色档，深色字压在暗画面上直接消失（主人 02:29 报的就是这张）。
 * sRGB 的 50% 中灰 = 线性 0.214，取 0.20 相当于「画面整体明显偏暗才算深色」。
 */
const LUMINANCE_DARK_MAX = 0.2
/**
 * 高于它算「亮壁纸」（用深色字）。
 *
 * ⚠ 2026-11-01 从 0.42 降到 0.30。原因是中性带（0.20~0.42）**太宽**，
 * 而中性档在 CSS 里根本没有任何规则 —— 落回白字，且 `--text-halo` 被设成 none，
 * 等于「白字直接压在亮壁纸上」，侧栏图标与文字全看不清
 * （主人报的那张壁纸实测中位亮度 **0.396**，就差 0.024 被判成中性）。
 *
 * 降到 0.30 之后：>0.30 一律深色字。中位亮度 0.30 已经是「整体明显偏亮」，
 * 深色字压上去才有对比。仍然用**中位数**而不是平均值，所以「暗画面带一块亮天空」
 * 不会被误判（亮块占比小，中位数还在暗侧）。
 */
const LUMINANCE_LIGHT_MIN = 0.3

export type WallpaperTone = 'dark' | 'neutral' | 'light'

const ACCENT_L_MIN_DARK = 0.58
const ACCENT_L_MAX_LIGHT = 0.5

function hexToHsl(hex: string): { h: number; s: number; l: number } | null {
    const match = /^#([0-9a-f]{6})$/i.exec(hex)
    if (!match) return null
    const int = parseInt(match[1], 16)
    const r = ((int >> 16) & 255) / 255
    const g = ((int >> 8) & 255) / 255
    const b = (int & 255) / 255
    const max = Math.max(r, g, b)
    const min = Math.min(r, g, b)
    const l = (max + min) / 2
    const delta = max - min
    if (delta < 1e-6) return { h: 0, s: 0, l }
    const s = l > 0.5 ? delta / (2 - max - min) : delta / (max + min)
    let h: number
    if (max === r) h = ((g - b) / delta) % 6
    else if (max === g) h = (b - r) / delta + 2
    else h = (r - g) / delta + 4
    h *= 60
    if (h < 0) h += 360
    return { h, s, l }
}

function hslToHex(h: number, s: number, l: number): string {
    const c = (1 - Math.abs(2 * l - 1)) * s
    const hh = ((h % 360) + 360) % 360 / 60
    const x = c * (1 - Math.abs((hh % 2) - 1))
    const m = l - c / 2
    const [r1, g1, b1] =
        hh < 1 ? [c, x, 0] : hh < 2 ? [x, c, 0] : hh < 3 ? [0, c, x]
        : hh < 4 ? [0, x, c] : hh < 5 ? [x, 0, c] : [c, 0, x]
    const to = (v: number) =>
        Math.round(Math.min(1, Math.max(0, v + m)) * 255)
            .toString(16)
            .padStart(2, '0')
    return `#${to(r1)}${to(g1)}${to(b1)}`
}

/** 按深浅档把强调色收进「一定读得清」的区间；色相不动。 */
export function clampAccentForScheme(color: string, scheme: 'dark' | 'light'): string {
    const hsl = hexToHsl(color)
    if (!hsl) return color
    // 饱和度也顺手托底：太灰的强调色在玻璃上等于是背景色
    const s = Math.max(0.42, hsl.s)
    const l =
        scheme === 'dark'
            ? Math.max(ACCENT_L_MIN_DARK, hsl.l)
            : Math.min(ACCENT_L_MAX_LIGHT, hsl.l)
    if (s === hsl.s && l === hsl.l) return color
    return hslToHex(hsl.h, s, l)
}
