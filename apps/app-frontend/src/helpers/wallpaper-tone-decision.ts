/**
 * 壁纸明暗判定（纯函数，无任何外部依赖）
 *
 * 抽成独立模块的原因：这段是「文字在浅色/深色壁纸上能不能读清」的唯一判据，
 * 必须能脱离 Tauri 单独跑测试 —— 否则每次验证都要装一次包。
 *
 * 判据是 **WCAG 极小极大对比度**：
 *   白字最坏 = 压在区域里最亮处（p98）
 *   深字最坏 = 压在区域里最暗处（p02）
 *   两者取最坏值更好的那个。且各区域取最差的那个区域。
 *
 * 为什么不沿用「全图中位亮度 + 阈值」：那套判的是**整张图**，
 * 既不知道文字压在哪一块，也不知道预览图与窗口的宽高比不同（cover 裁切会把
 * 相当一部分像素裁掉）。实测 71 张壁纸里会判错 19 张（27%），
 * 其中 18 张是该用深字却给了白字，最坏对比度低到 1.15:1。
 */

/** Rust 侧回传的低频亮度网格（0–255，对应 WCAG 相对亮度 ×255）。 */
export type LumGrid = {
    gridW: number
    gridH: number
    /** 跨帧逐格最大 —— 白字的最坏情况 */
    lumMax: number[]
    /** 跨帧逐格最小 —— 深字的最坏情况 */
    lumMin: number[]
}

/** 归一化矩形（0–1，相对窗口）。 */
export type Region = { name: string; x0: number; x1: number; y0: number; y1: number }

/** 玻璃两层：深色底垫 alpha，白玻璃叠加 alpha。 */
export type GlassLayers = { dim: number; white: number }

export type ToneDecision = {
    tone: 'dark' | 'light'
    /** 达到目标对比度的**面积占比**（0–1），跨区域取最差的区域 */
    whiteCoverage: number
    inkCoverage: number
    /** 该区域内的中位对比度，用于覆盖率为 0 对 0 时破平 */
    whiteMedian: number
    inkMedian: number
    /** 各自最差的区域（诊断用） */
    whiteAt: string
    inkAt: string
}

export type ToneContext = {
    regions: Region[]
    glass: GlassLayers
    /** 窗口宽高比 */
    aspect: number
}

/** 墨色亮度与玻璃底色亮度（预先算好，避免每次判定重算）。 */
const INK_RGB = [20, 22, 28] as const

/** `#0a0b10` 的相对亮度 —— 玻璃底垫就是这个色。 */
export const GLASS_LUM = wcagLum(10, 11, 16)
export const INK_LUM = wcagLum(INK_RGB[0], INK_RGB[1], INK_RGB[2])

/**
 * 「读得清」的门槛。WCAG 对小字号要求 4.5:1，但壁纸是满屏图案，
 * 按 4.5 打分会把几乎所有壁纸判成不合格、判据失去区分度；3:1 是 WCAG 的大字号门槛，
 * 这里取它做**面积统计的分界线**，再用中位对比度做次级区分。
 */
const TARGET_CONTRAST = 3

/** 覆盖率差不到 5 个百分点就不换（迟滞），避免阈值附近来回闪。 */
export const MIN_SWITCH_COVERAGE = 0.05
/** 覆盖率打平时，用中位对比度的倍数做迟滞。 */
export const MIN_SWITCH_GAIN = 1.12

function srgbToLinear(c: number): number {
    const s = c / 255
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
}

/** WCAG 相对亮度：逐通道线性化再加权（纯蓝这类饱和色，先加权再线性化会差一个数量级）。 */
export function wcagLum(r: number, g: number, b: number): number {
    return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b)
}

export function contrast(a: number, b: number): number {
    const hi = Math.max(a, b)
    const lo = Math.min(a, b)
    return (hi + 0.05) / (lo + 0.05)
}

function median(sorted: number[]): number {
    if (!sorted.length) return 0
    return sorted[sorted.length >> 1]
}

/**
 * cover 裁切：网格按原图比例，窗口是另一个比例，取中间那块。
 * 返回裁切后的尺寸与偏移（单位：格）。
 */
export function coverCrop(gridW: number, gridH: number, aspect: number) {
    const gridAspect = gridW / gridH
    if (gridAspect > aspect) {
        const width = gridH * aspect
        return { width, height: gridH, ox: (gridW - width) / 2, oy: 0 }
    }
    const height = gridW / aspect
    return { width: gridW, height, ox: 0, oy: (gridH - height) / 2 }
}

/** 玻璃合成（在线性亮度里做）：先深色底垫，再白玻璃，顺序同 CSS。 */
function compositor(glass: GlassLayers): (lum: number) => number {
    const dim = Math.min(1, Math.max(0, glass.dim))
    const white = Math.min(1, Math.max(0, glass.white))
    return (lum) => (1 - white) * ((1 - dim) * lum + dim * GLASS_LUM) + white
}

/** 一个候选在若干局部背景上的表现。 */
type Score = { coverage: number; median: number }

function score(sorted: number[], foreground: number): Score {
    let hit = 0
    for (const bg of sorted) {
        if (contrast(foreground, bg) >= TARGET_CONTRAST) hit += 1
    }
    return {
        coverage: sorted.length ? hit / sorted.length : 0,
        median: contrast(foreground, median(sorted)),
    }
}

/**
 * 判定：分别给白字/深字算「读得清的面积占比」，取覆盖更多的那一侧。
 *
 * 为什么不按单个最亮/最暗像素：自然图像里几乎每一块都同时有极暗与极亮点，
 * 两个候选的最坏值都被离群像素支配 —— 实测 71 张里 60 张会判成深字，
 * 而最优侧中位对比度只有 2.38:1，判据彻底失去区分度。
 *
 * 为什么按**面积加权**而不是「取最差的区域」：区域面积差得太多。
 * 实测 ws-1375031246 只有顶栏那条亮天空不适合白字（23%），而侧栏 80%、主区 74%
 * 都适合 —— 按最差区域会为了 8% 的面积把整屏翻成深字。
 * 面积加权量的正是「整个界面有多少文字面积读得清」，这才是使用者的实际观感。
 *
 * ⚠ 已知取舍：侧栏只占约 5% 面积，权重小。所以「大片亮 + 左边缘一条暗」这种极端
 * 构图，侧栏仍可能偏淡 —— 那是 `--ui-dim = 0`（完全不压暗）的固有代价，
 * 想彻底解决只能加底衬或光晕，而主人明确不要。
 */
export function decideTone(grid: LumGrid, ctx: ToneContext): ToneDecision | null {
    const { gridW: gw, gridH: gh, lumMax, lumMin } = grid
    if (!gw || !gh || lumMax.length !== gw * gh || lumMin.length !== gw * gh) return null

    const composite = compositor(ctx.glass)
    const crop = coverCrop(gw, gh, ctx.aspect || 1)

    let whiteHit = 0
    let inkHit = 0
    let total = 0
    let whiteAt = ''
    let inkAt = ''
    // 初值 2 是为了第一轮一定写进白/深各自的「最差区域」（覆盖率最高也就 1）
    let whiteWorstCov = 2
    let inkWorstCov = 2
    let whiteMedianSum = 0
    let inkMedianSum = 0
    let regionCount = 0

    for (const region of ctx.regions) {
        const bright: number[] = []
        const dark: number[] = []
        for (let y = 0; y < gh; y += 1) {
            const ny = (y + 0.5 - crop.oy) / crop.height
            if (ny < region.y0 || ny > region.y1) continue
            for (let x = 0; x < gw; x += 1) {
                const nx = (x + 0.5 - crop.ox) / crop.width
                if (nx < region.x0 || nx > region.x1) continue
                const i = y * gw + x
                bright.push(composite(lumMax[i] / 255))
                dark.push(composite(lumMin[i] / 255))
            }
        }
        if (!bright.length) continue

        bright.sort((a, b) => a - b)
        dark.sort((a, b) => a - b)

        const w = score(bright, 1)
        const k = score(dark, INK_LUM)
        whiteHit += w.coverage * bright.length
        inkHit += k.coverage * dark.length
        total += bright.length
        whiteMedianSum += w.median
        inkMedianSum += k.median
        regionCount += 1
        if (w.coverage < whiteWorstCov) {
            whiteWorstCov = w.coverage
            whiteAt = region.name
        }
        if (k.coverage < inkWorstCov) {
            inkWorstCov = k.coverage
            inkAt = region.name
        }
    }

    let whiteWorst: Score
    let inkWorst: Score
    if (!total) {
        // 区域全部落空（DOM 还没挂）：退回整张图，别拿不到结论
        const bright = lumMax.map((v) => composite(v / 255)).sort((a, b) => a - b)
        const dark = lumMin.map((v) => composite(v / 255)).sort((a, b) => a - b)
        whiteWorst = score(bright, 1)
        inkWorst = score(dark, INK_LUM)
        whiteAt = inkAt = '全图'
    } else {
        whiteWorst = { coverage: whiteHit / total, median: whiteMedianSum / regionCount }
        inkWorst = { coverage: inkHit / total, median: inkMedianSum / regionCount }
    }

    /* 覆盖率优先；覆盖率打平（常见于两边都不合格）时看中位对比度。 */
    const byCoverage = whiteWorst.coverage - inkWorst.coverage
    const tone: 'dark' | 'light' =
        Math.abs(byCoverage) > 1e-6
            ? byCoverage > 0
                ? 'dark'
                : 'light'
            : whiteWorst.median >= inkWorst.median
              ? 'dark'
              : 'light'

    return {
        tone,
        whiteCoverage: whiteWorst.coverage,
        inkCoverage: inkWorst.coverage,
        whiteMedian: whiteWorst.median,
        inkMedian: inkWorst.median,
        whiteAt,
        inkAt,
    }
}

/**
 * 迟滞：`incumbent` 非空且与结论不同时，只有优势足够才真的切。
 * 覆盖率差 5 个百分点以上，或覆盖率打平但中位对比度好 12% 以上。
 */
export function applyHysteresis(
    decision: ToneDecision,
    incumbent: 'dark' | 'light' | null,
): 'dark' | 'light' {
    if (!incumbent || incumbent === decision.tone) return decision.tone

    const nowCoverage = incumbent === 'dark' ? decision.whiteCoverage : decision.inkCoverage
    const nextCoverage = decision.tone === 'dark' ? decision.whiteCoverage : decision.inkCoverage
    if (Math.abs(nextCoverage - nowCoverage) >= MIN_SWITCH_COVERAGE) return decision.tone

    const nowMedian = incumbent === 'dark' ? decision.whiteMedian : decision.inkMedian
    const nextMedian = decision.tone === 'dark' ? decision.whiteMedian : decision.inkMedian
    if (nextMedian / Math.max(1e-6, nowMedian) < MIN_SWITCH_GAIN) return incumbent
    return decision.tone
}
