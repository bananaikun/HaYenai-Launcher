/**
 * 界面玻璃参数（设置 → 外观 → 界面玻璃）。
 *
 * 为什么单独一个 store 而不是塞进 ThemeStore：这三项只驱动 `<html>` 上的三个
 * CSS 变量，不参与 Rust 侧设置同步，也不需要主题/强调色那套派生逻辑。
 * 存 localStorage 即可，改一个数立刻写变量 —— 外观页的滑杆要的就是这个即时反馈。
 *
 * 与后台的关系：后台的玻璃值是固定的（它只有一张深色壁纸）；
 * 启动器的壁纸由用户换，所以「多透 / 多糊 / 压多暗」必须可调，
 * 这里给的是**下限可调**，不改变玻璃本身的三层结构（底垫 + 模糊 + 内高光）。
 */
import { reactive, watch } from 'vue'

export type UiGlassSettings = {
    /** 外壳（侧栏/标题栏）底垫浓度 %：越低越透，越高越像实心面板 */
    dim: number
    /** 卡片/面板实底比例 %：卡片读 --surface-2/3，这里决定它们压多实 */
    cardOpacity: number
    /** 玻璃模糊半径 px */
    blur: number
    /** 玻璃背后壁纸的饱和度 %：100 = 原图原样，默认 105（只比原图高一点点） */
    saturate: number
    /** 高级：白玻璃的浓度 %（--glass-2 的 alpha ×100），默认 7 */
    glassAlpha: number
    /** 高级：发丝描边的强度 %，默认 10 */
    strokeAlpha: number
    /** 高级：界面基础动效时长 ms，默认 360 */
    motionMs: number
}

export const UI_GLASS_LIMITS = {
    /* 上限 100 → 60（2026-10-25 11:45）。
       `--ui-dim` 是乘进 `color-mix(#0a0b10 N%, transparent)` 的：100% 就是**纯黑**，
       主人截图里那个"整块黑色设置内容区"就是把它拉满造成的（dim=25 时是正常玻璃）。
       60% 仍是很深的玻璃，但不会再糊成死黑。 */
    dim: { min: 0, max: 60, step: 2 },
    cardOpacity: { min: 0, max: 100, step: 5 },
    blur: { min: 0, max: 48, step: 2 },
    saturate: { min: 100, max: 160, step: 5 },
    glassAlpha: { min: 2, max: 20, step: 1 },
    strokeAlpha: { min: 0, max: 40, step: 2 },
    motionMs: { min: 120, max: 600, step: 20 },
} as const

const STORAGE_KEY = 'hayenai:ui-glass'

/**
 * 默认值 = 主人当前实际在用的档位（2026-10-31 从 localStorage 读回后固化）。
 * `cardOpacity: 0` 是刻意的：卡片不要实底，全靠玻璃 + 模糊 + 发丝描边，
 * 这样壁纸能透过卡片；`--surface-N` 已在令牌层接上这个值。
 */
const DEFAULTS: UiGlassSettings = {
    dim: 0,
    cardOpacity: 0,
    blur: 16,
    saturate: 160,
    glassAlpha: 7,
    strokeAlpha: 10,
    motionMs: 360,
}

function read(): UiGlassSettings {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return { ...DEFAULTS }
        const parsed = JSON.parse(raw) as Partial<UiGlassSettings>
        return {
            dim: clamp(parsed.dim ?? DEFAULTS.dim, 'dim'),
            cardOpacity: clamp(parsed.cardOpacity ?? DEFAULTS.cardOpacity, 'cardOpacity'),
            blur: clamp(parsed.blur ?? DEFAULTS.blur, 'blur'),
            saturate: clamp(parsed.saturate ?? DEFAULTS.saturate, 'saturate'),
            glassAlpha: clamp(parsed.glassAlpha ?? DEFAULTS.glassAlpha, 'glassAlpha'),
            strokeAlpha: clamp(parsed.strokeAlpha ?? DEFAULTS.strokeAlpha, 'strokeAlpha'),
            motionMs: clamp(parsed.motionMs ?? DEFAULTS.motionMs, 'motionMs'),
        }
    } catch {
        return { ...DEFAULTS }
    }
}

function clamp(value: number, key: keyof typeof UI_GLASS_LIMITS): number {
    const { min, max } = UI_GLASS_LIMITS[key]
    if (!Number.isFinite(value)) return DEFAULTS[key]
    return Math.min(Math.max(Math.round(value), min), max)
}

export const uiGlass = reactive<UiGlassSettings>(read())

/** 把三个值写成 `<html>` 上的 CSS 变量；CSS 侧只认变量，不认具体数值 */
export function applyUiGlass(): void {
    const root = document.documentElement
    root.style.setProperty('--ui-dim', `${uiGlass.dim}%`)
    root.style.setProperty('--ui-card-alpha', `${uiGlass.cardOpacity}%`)
    root.style.setProperty('--ui-glass-blur', `${uiGlass.blur}px`)
    root.style.setProperty('--ui-saturate', `${uiGlass.saturate}%`)
    // 白玻璃两层：上层（glass-2）就是滑杆值，下层（glass-1）取它的 0.64 倍 —— 与
    // 侧栏原来 0.07 / 0.045 的比例一致，拖动时保持同一条渐变。
    const g2 = uiGlass.glassAlpha / 100
    root.style.setProperty('--ui-glass-2-alpha', g2.toFixed(3))
    root.style.setProperty('--ui-glass-1-alpha', (g2 * 0.64).toFixed(3))
    root.style.setProperty('--ui-stroke-alpha', (uiGlass.strokeAlpha / 100).toFixed(3))
    root.style.setProperty('--ui-motion', `${uiGlass.motionMs}ms`)
}

export function setUiGlass(patch: Partial<UiGlassSettings>): void {
    if (patch.dim !== undefined) uiGlass.dim = clamp(patch.dim, 'dim')
    if (patch.cardOpacity !== undefined) uiGlass.cardOpacity = clamp(patch.cardOpacity, 'cardOpacity')
    if (patch.blur !== undefined) uiGlass.blur = clamp(patch.blur, 'blur')
    if (patch.saturate !== undefined) uiGlass.saturate = clamp(patch.saturate, 'saturate')
    if (patch.glassAlpha !== undefined) uiGlass.glassAlpha = clamp(patch.glassAlpha, 'glassAlpha')
    if (patch.strokeAlpha !== undefined) uiGlass.strokeAlpha = clamp(patch.strokeAlpha, 'strokeAlpha')
    if (patch.motionMs !== undefined) uiGlass.motionMs = clamp(patch.motionMs, 'motionMs')
}

export function resetUiGlass(): void {
    Object.assign(uiGlass, DEFAULTS)
}

watch(
    uiGlass,
    () => {
        applyUiGlass()
        try {
            localStorage.setItem(STORAGE_KEY, JSON.stringify(uiGlass))
        } catch {
            // 隐私模式下写不进去；本次会话内仍然生效，不必打断用户
        }
    },
    { deep: true },
)

applyUiGlass()
