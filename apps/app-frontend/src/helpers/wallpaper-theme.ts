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

import {
    applyHysteresis,
    decideTone,
    wcagLum,
    type GlassLayers,
    type LumGrid,
    type Region,
} from '@/helpers/wallpaper-tone-decision'
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


/** Rust 侧回传的采样：低频亮度网格 + 主色。判据全在 `wallpaper-tone-decision`。 */
type AccentSample = LumGrid & {
    color: string
    luminance: number
}

/**
 * 文字真正压住的区域 —— 从 DOM 真实几何读，不猜。
 *
 * 侧栏是唯一「大片文字直接压在壁纸上、下面没有卡片」的地方；顶栏放标题与图标；
 * 主区剩下的大块（卡片自身有玻璃，但行内文字同样吃壁纸底色）。
 * 侧栏收起（`.side.min`）或还没挂载时退回经验值。
 */
function textRegions(): Region[] {
    const W = window.innerWidth || 1
    const H = window.innerHeight || 1
    const rect = (document.querySelector('.side') as HTMLElement | null)?.getBoundingClientRect()
    const hasSide = !!rect && rect.width > 8 && rect.height > 8

    const sideX1 = hasSide ? rect.right / W : 0.06
    const sideY0 = hasSide ? rect.top / H : 0.05
    const sideY1 = hasSide ? rect.bottom / H : 0.95
    // 顶栏就是侧栏顶部那条空档（标题栏与侧栏同高起步），夹到 4%–20%
    const topY1 = Math.min(0.2, Math.max(0.04, sideY0))

    return [
        { name: '左侧栏', x0: 0, x1: sideX1, y0: sideY0, y1: sideY1 },
        { name: '顶栏', x0: sideX1, x1: 1, y0: 0, y1: topY1 },
        { name: '主区', x0: sideX1, x1: 1, y0: topY1, y1: 1 },
    ]
}

/**
 * 玻璃两层：深色底垫（`--ui-dim`）+ 白玻璃叠加（`--ui-glass-2-alpha`）。
 * 主人定的是 `--ui-dim = 0`（完全不压暗），所以这里必须读实际值 ——
 * 硬编码 25% 会把「文字压在原图上」错算成「压在压暗 25% 的背景上」，
 * 结论会整体偏向白字。
 */
function glassLayers(): GlassLayers {
    const cs = getComputedStyle(document.documentElement)
    const dimRaw = parseFloat(cs.getPropertyValue('--ui-dim'))
    const whiteRaw = parseFloat(cs.getPropertyValue('--ui-glass-2-alpha'))
    return {
        dim: Number.isFinite(dimRaw) ? dimRaw / 100 : 0,
        white: Number.isFinite(whiteRaw) ? whiteRaw : 0.07,
    }
}

let cachedSample: AccentSample | null = null
let cachedId: string | null = null
/**
 * 屏幕上**真正渲染的那一帧**的低频网格。有它就优先于 preview。
 *
 * 为什么必须这么做：Wallpaper Engine 的 `preview.*` 是**工坊封面**，跟场景实际画面
 * 对不上的情况很常见（实测 ws-2986218263 的封面是亮粉角色立绘，场景却是深绿水面）。
 * 按封面判定，深色壁纸会被判成「该用深字」，侧栏直接糊掉。
 */
let liveGrid: LumGrid | null = null
let resizeTimer: number | null = null
let domRetryTimer: number | null = null

/**
 * 第一次判定可能发生在挂载前：`App.vue` 的 watch 是 `immediate`，setup 阶段 `.side`
 * 还没进 DOM，只能退回经验区域。等侧栏出现后补算一次，判定才用上真实几何。
 */
function retryWhenMounted(tries = 0): void {
    if (document.querySelector('.side') || tries > 40) {
        domRetryTimer = null
        applyToneFromSample(true)
        return
    }
    domRetryTimer = window.setTimeout(() => retryWhenMounted(tries + 1), 100)
}

function currentTone(): 'dark' | 'light' | null {
    const tone = document.documentElement.dataset.wallpaperTone
    return tone === 'dark' || tone === 'light' ? tone : null
}

/** 按缓存样本重新判定并写 `data-wallpaper-tone`；`force` 表示不受迟滞约束（换壁纸）。 */
function applyToneFromSample(force = false): void {
    const grid: LumGrid | null = liveGrid ?? cachedSample
    if (!grid) return
    const decision = decideTone(grid, {
        regions: textRegions(),
        glass: glassLayers(),
        aspect: (window.innerWidth || 1) / (window.innerHeight || 1),
    })
    if (import.meta.env.DEV) {
        console.info(
            `[wallpaper-theme] decide source=${liveGrid ? 'surface' : 'preview'} grid=${grid.gridW}x${grid.gridH} ` +
                `tone=${decision?.tone ?? 'null'} white=${decision ? decision.whiteCoverage.toFixed(2) : '-'} ` +
                `ink=${decision ? decision.inkCoverage.toFixed(2) : '-'}`,
        )
    }
    if (!decision) return

    const tone = applyHysteresis(decision, force ? null : currentTone())
    document.documentElement.dataset.wallpaperTone = tone

    if (force && !document.querySelector('.side') && domRetryTimer === null) retryWhenMounted()
}

/** 网格长边格数，与 Rust 侧 `wallpaper_accent` 的 GRID 保持一致。 */
const GRID_EDGE = 64

function surfaceSize(el: HTMLCanvasElement | HTMLVideoElement): [number, number] {
    if (el instanceof HTMLVideoElement) return [el.videoWidth, el.videoHeight]
    return [el.width, el.height]
}

function isBlankGrid(grid: LumGrid): boolean {
    return grid.lumMax.every((v) => v === 0)
}

/**
 * 把一个「正在显示的元素」缩成低频亮度网格。
 * 失败返回 null —— 调用方退回 preview 那条路，不阻断任何东西。
 */
function gridFromSurface(el: HTMLCanvasElement | HTMLVideoElement): LumGrid | null {
    const [w, h] = surfaceSize(el)
    if (!w || !h) return null

    const gw = w >= h ? GRID_EDGE : Math.max(1, Math.round((w * GRID_EDGE) / h))
    const gh = w >= h ? Math.max(1, Math.round((h * GRID_EDGE) / w)) : GRID_EDGE

    const canvas = document.createElement('canvas')
    canvas.width = gw
    canvas.height = gh
    const ctx = canvas.getContext('2d', { willReadFrequently: true })
    if (!ctx) return null

    try {
        ctx.drawImage(el, 0, 0, gw, gh)
        const { data } = ctx.getImageData(0, 0, gw, gh)
        const lum: number[] = []
        for (let i = 0; i < data.length; i += 4) {
            lum.push(Math.round(wcagLum(data[i], data[i + 1], data[i + 2]) * 255))
        }
        return { gridW: gw, gridH: gh, lumMax: lum, lumMin: lum }
    } catch {
        // 跨源污染（视频走 hayenai-we 协议且响应没带 ACAO）—— 读不到就不读
        return null
    }
}

/**
 * 用**屏幕上真正渲染的那一帧**重判明暗。场景壁纸的 WebGL canvas 是页内同源创建的，
 * 可以直接读；视频则要求 `<video crossorigin="anonymous">` 且取流响应带 ACAO，
 * 否则 canvas 被污染、`getImageData` 抛错，这里就会返回 `null`。
 *
 * WebGL 默认没开 `preserveDrawingBuffer`，首帧回调里立刻读可能拿到空缓冲 ——
 * 读到全 0 就退一帧重试，最多 4 次。**4 次都空就当读不到**：
 * 把「读不到」当成「全黑壁纸」会让判定永远落成白字，比退回封面更糟。
 */
export function applyToneFromSurface(
    el: HTMLCanvasElement | HTMLVideoElement,
    attempt = 0,
    force = true,
): void {
    const grid = gridFromSurface(el)
    if (!grid) return
    if (isBlankGrid(grid)) {
        if (attempt < 4) requestAnimationFrame(() => applyToneFromSurface(el, attempt + 1, force))
        return
    }
    liveGrid = grid
    // 首帧那一次不受迟滞约束（换壁纸要立刻定字色）；之后的重采允许迟滞防抖
    applyToneFromSample(force)
}

/* 窗口比例一变，cover 裁切就变了 —— 可见区域跟着变，判定必须重算。
   只重算缓存里的网格，不再解一次图（Rust 那边解 4K 图要 30–80ms，拖窗口跟不上）。 */
if (typeof window !== 'undefined') {
    window.addEventListener('resize', () => {
        if (resizeTimer !== null) window.clearTimeout(resizeTimer)
        resizeTimer = window.setTimeout(() => {
            resizeTimer = null
            applyToneFromSample()
        }, 300)
    })
}

/**
 * 按当前开关把壁纸主题应用一次。
 * `id` 为空（没有壁纸）时直接返回，不动用户自己选的主题。
 */
export async function applyWallpaperTheme(id: string | null | undefined): Promise<void> {
    if (import.meta.env.DEV) {
        // 明暗判定整条链路只在「切壁纸」时跑一次，出了问题界面上看不出来 ——
        // 留一条 DEV 日志，实机排查时不必再改代码。
        console.info(`[wallpaper-theme] apply id=${id ?? 'null'} tone=${currentTone() ?? 'null'}`)
    }
    if (!id) {
        delete document.documentElement.dataset.wallpaperTone
        cachedSample = null
        cachedId = null
        return
    }
    /* ⚠ 这里**不能**再按 accent/scheme 两个开关提前返回（2026-10-25 08:45）。
       实机症状：壁纸是亮的，字却是白的、压在亮壁纸上发淡 —— 因为主人关掉了
       「随壁纸取色 / 随壁纸切深浅」，函数在那行直接 return，`data-wallpaper-tone`
       从来没被写过，字体色落回默认白族。
       字色是**可读性底线**，必须与那两个开关解耦：取一次采样 → 写 tone，总是执行；
       下面 accent 的部分才受开关控制。 */
    if (!cachedSample || cachedId !== id) {
        // 换了壁纸：上一张的实渲染帧不能再用来判这一张
        liveGrid = null
        try {
            cachedSample = await invoke<AccentSample | null>('wallpaper_accent', { id })
        } catch (error) {
            /* 命令缺失或读图失败：保持用户自己的主题，不报错到 UI。
               但**必须留一行日志**：这条 catch 静默吞掉过一次真实故障
               （网页类壁纸的 tone 一直为空、字色停在白族），界面看不出任何异常。 */
            console.warn('[wallpaper-theme] 明暗采样失败，本次保留原字色', error)
            return
        }
        cachedId = id
    }
    if (!cachedSample) {
        if (import.meta.env.DEV) {
            console.warn(`[wallpaper-theme] 采样为空（${id}），保留原字色`)
        }
        return
    }

    // 字色完全跟着壁纸走（与主题档、与取色开关都解耦）；换壁纸时不受迟滞约束
    applyToneFromSample(true)

    // 取色/切档两个开关都关时，字色已经设置完，到此为止
    if (!wallpaperTheme.accent && !wallpaperTheme.scheme) return

    const themeStore = useTheming()
    const scheme: 'dark' | 'light' = currentTone() ?? 'dark'

    /* 2026-10-25 07:00 主人定：**只切文字色，不切主题档**（不碰背景/模糊）。
       以前这里调 `setThemeState(tone)` 会把整站换成浅/深主题 ——
       那等于连玻璃一起换色，主人明确禁止。文字色由 CSS 的
       `[data-wallpaper-tone]` 决定，`scheme` 只用于强调色钳位。 */

    // 必须带 `custom:` 前缀：setAccentColor 用 parseCustomAccentColor 校验，
    // 裸 `#rrggbb` 会被判为非法值然后静默退回预设色（上一版就是这个问题）。
    if (wallpaperTheme.accent && /^#[0-9a-fA-F]{6}$/.test(cachedSample.color)) {
        const hex = clampAccentForScheme(cachedSample.color, scheme)
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
 * 现在只有两档。`neutral` 是历史遗留：CSS 里从来没给中性档写过规则，
 * 落回白字 + `--text-halo: none`，等于「白字直接压在壁纸上」——
 * 12/71 张壁纸落进这一档，这正是「中性色识别不准」最刺眼的那部分。
 * 改成极小极大对比度之后，每一张都会明确落到白字或深字其中之一。
 */
export type WallpaperTone = 'dark' | 'light'

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
