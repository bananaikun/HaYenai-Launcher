/**
 * 字体颜色 / 渐变字体（设置 → 外观 → 字体颜色）
 *
 * 主人 2026-11-15：字体只有黑/白太单调。这里给两类能力：
 * 1. **预设单色**：黑、白 + 七种常见色，作用在「标题 + 强调文字」上（正文保持单色，
 *    渐变正文可读性太差）。
 * 2. **上下渐变**：把壁纸缩成 64×64 后取**最鲜艳那一格**的颜色，与当前主题色拼成
 *    `linear-gradient(to bottom, 壁纸艳色, 主题色)`。
 *    取色在 Rust 侧做（`wallpaper_accent` 的 `vivid` 字段）：壁纸走自定义协议，
 *    前端 canvas 会被跨源污染，读不到像素。
 *
 * 落地方式：只写三个 CSS 变量 + 一个 data 属性，样式规则集中在
 * `assets/stylesheets/font-theme.css`，不逐元素改模板。
 */
import { invoke } from '@tauri-apps/api/core'

import { selectedWallpaper } from '@/helpers/wallpaper'

export type FontToneMode = 'default' | 'preset' | 'gradient'

export interface FontTheme {
    /** default = 跟随主题本来黑白；preset = 单一颜色；gradient = 上下渐变 */
    mode: FontToneMode
    /** preset 模式下用的颜色（`#rrggbb`） */
    color: string
    /** 渐变的上半色（壁纸最鲜艳格），为空时回退到主题色提亮 */
    vivid: string
}

const STORAGE_KEY = 'hayenai:font-theme'

/** 预设色：黑白 + 七色。都给足对比度的中间调，压在白/黑壁纸上都能读。 */
export const FONT_COLOR_PRESETS = [
    { value: '#ffffff', label: '白' },
    { value: '#12141a', label: '黑' },
    { value: '#ef4444', label: '红' },
    { value: '#f97316', label: '橙' },
    { value: '#eab308', label: '黄' },
    { value: '#22c55e', label: '绿' },
    { value: '#06b6d4', label: '青' },
    { value: '#3b82f6', label: '蓝' },
    { value: '#a855f7', label: '紫' },
] as const

const DEFAULTS: FontTheme = { mode: 'default', color: '#ffffff', vivid: '' }

function read(): FontTheme {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return { ...DEFAULTS }
        const parsed = JSON.parse(raw) as Partial<FontTheme>
        return {
            mode: parsed.mode ?? DEFAULTS.mode,
            color: parsed.color ?? DEFAULTS.color,
            vivid: parsed.vivid ?? DEFAULTS.vivid,
        }
    } catch {
        return { ...DEFAULTS }
    }
}

export const fontTheme: FontTheme = read()

/** 主题色（`--color-brand`）实际值，取不到就用品牌紫兜底。 */
export function brandColor(): string {
    try {
        const value = getComputedStyle(document.documentElement)
            .getPropertyValue('--color-brand')
            .trim()
        if (value) return value
    } catch {
        // SSR/测试环境
    }
    return '#a855f7'
}

/** 把当前设置写到根元素上（纯 CSS 变量 + data 属性，样式规则在 font-theme.css）。 */
export function applyFontTheme(): void {
    const root = document.documentElement
    if (fontTheme.mode === 'default') {
        delete root.dataset.fontTone
        root.style.removeProperty('--font-strong')
        root.style.removeProperty('--font-strong-a')
        root.style.removeProperty('--font-strong-b')
        return
    }

    root.dataset.fontTone = fontTheme.mode
    if (fontTheme.mode === 'preset') {
        root.style.setProperty('--font-strong', fontTheme.color)
        root.style.removeProperty('--font-strong-a')
        root.style.removeProperty('--font-strong-b')
        return
    }

    // 渐变：壁纸艳色 → 主题色。没取到艳色时用主题色提亮一档当上半色，
    // 至少保证上下有明暗变化，不会退化成一块纯色。
    const brand = brandColor()
    root.style.setProperty('--font-strong-a', fontTheme.vivid || brand)
    root.style.setProperty('--font-strong-b', brand)
}

export function setFontTheme(patch: Partial<FontTheme>): void {
    if (patch.mode !== undefined) fontTheme.mode = patch.mode
    if (patch.color !== undefined) fontTheme.color = patch.color
    if (patch.vivid !== undefined) fontTheme.vivid = patch.vivid
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(fontTheme))
    } catch {
        // 隐私模式不落盘，本次会话内仍生效
    }
    applyFontTheme()
}

/**
 * 重新取「壁纸最鲜艳格」。
 *
 * 只在切壁纸 / 打开设置时调用：Rust 要解码封面（动图按帧预算截断），
 * 一次几百毫秒到一两秒，不能挂在渲染路径上。
 */
export async function refreshVividColor(): Promise<string> {
    const id = selectedWallpaper()?.id
    if (!id) {
        setFontTheme({ vivid: '' })
        return ''
    }
    try {
        const sample = await invoke<{ vivid?: string } | null>('wallpaper_accent', { id })
        const vivid = sample?.vivid ?? ''
        setFontTheme({ vivid })
        return vivid
    } catch (error) {
        console.warn('[font-theme] 壁纸最鲜艳格采样失败，渐变回退到主题色', error)
        return fontTheme.vivid
    }
}
