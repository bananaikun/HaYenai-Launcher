/**
 * 壁纸轮播（2026-10-25，主人 06:44 需求）
 *
 * 契约：
 * - `mode: 'off'` 关闭；`'sequential'` 按勾选列表顺序循环；`'random'` 随机（排除当前那张）
 * - `intervalMinutes` 为切换间隔，UI 给 1/5/10/30/60 五档
 * - `ids` 是参与轮播的壁纸 id（壁纸库里多选勾出来的），顺序即轮播顺序
 *
 * 为什么存 localStorage 而不是 settings 表：轮播是一个纯前端的播放策略，
 * 加进 Rust 的 settings schema 需要动 serde 结构 + 迁移 + 类型定义，
 * 收益只是「能随配置导出」。先不承担这个成本；真要跟配置走时把 load/persist 换掉即可。
 *
 * 不变量：
 * - **只在可见时切**（`document.hidden` 跳过）——后台切壁纸会白白解码一帧还占内存
 * - 池子里不足 2 张就不启动 —— 一张自己切自己没有意义，还会让计时器空转
 * - 当前壁纸不在池子里时，顺序模式从第 1 张开始，而不是原地卡死
 */
import { reactive } from 'vue'

import { selectWallpaper, wallpaperState } from '@/helpers/wallpaper'

export type RotationMode = 'off' | 'sequential' | 'random'

export type WallpaperRotation = {
    mode: RotationMode
    intervalSeconds: number
    ids: string[]
}

const STORAGE_KEY = 'hayenai:wallpaper-rotation'

/** 间隔档位（秒）：滑块 5~90、步进 5。 */
export const ROTATION_INTERVAL_MIN = 5
export const ROTATION_INTERVAL_MAX = 90
export const ROTATION_INTERVAL_STEP = 5

const DEFAULTS: WallpaperRotation = {
    mode: 'off',
    intervalSeconds: 30,
    ids: [],
}

function load(): WallpaperRotation {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return { ...DEFAULTS, ids: [] }
        const parsed = JSON.parse(raw) as Partial<WallpaperRotation>
        const mode: RotationMode =
            parsed.mode === 'sequential' || parsed.mode === 'random' ? parsed.mode : 'off'
        /* 兼容旧存档：以前存的是"分钟"（1/5/10/30/60），现在统一按"秒"。
           读到旧字段就 ×60，再钳到 5~90。 */
        const legacyMinutes = Number((parsed as { intervalMinutes?: number }).intervalMinutes)
        const rawSeconds = Number(parsed.intervalSeconds ?? (Number.isFinite(legacyMinutes) ? legacyMinutes * 60 : NaN))
        const seconds = Number.isFinite(rawSeconds) && rawSeconds > 0 ? rawSeconds : DEFAULTS.intervalSeconds
        return {
            mode,
            intervalSeconds: Math.min(ROTATION_INTERVAL_MAX, Math.max(ROTATION_INTERVAL_MIN, Math.round(seconds))),
            ids: Array.isArray(parsed.ids)
                ? parsed.ids.filter((id): id is string => typeof id === 'string')
                : [],
        }
    } catch {
        return { ...DEFAULTS, ids: [] }
    }
}

export const rotationState = reactive<WallpaperRotation>(load())

let timer: number | null = null

function persist(): void {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(rotationState))
    } catch {
        // 隐私模式写不进去：本次会话仍然有效
    }
}

/** 轮播池：只保留当前还存在（没被删）的壁纸，且顺序按用户勾选顺序。 */
function pool(): string[] {
    const alive = new Set(wallpaperState.items.map((item) => item.id))
    return rotationState.ids.filter((id) => alive.has(id))
}

function pickNext(): string | null {
    const ids = pool()
    if (ids.length < 2) return null
    const current = wallpaperState.config.selected

    if (rotationState.mode === 'random') {
        const candidates = ids.filter((id) => id !== current)
        if (!candidates.length) return null
        return candidates[Math.floor(Math.random() * candidates.length)]
    }

    // sequential：当前不在池子里就从第 1 张开始，别原地卡住
    const index = current ? ids.indexOf(current) : -1
    return ids[(index + 1) % ids.length]
}

async function tick(): Promise<void> {
    if (rotationState.mode === 'off') return
    if (typeof document !== 'undefined' && document.hidden) return
    const next = pickNext()
    if (!next) return
    try {
        await selectWallpaper(next)
    } catch (error) {
        console.warn('[wallpaper-rotation] 切换失败', error)
    }
}

export function stopRotation(): void {
    if (timer !== null) {
        window.clearInterval(timer)
        timer = null
    }
}

export function startRotation(): void {
    stopRotation()
    if (rotationState.mode === 'off') return
    if (pool().length < 2) return
    const ms = Math.max(ROTATION_INTERVAL_MIN, rotationState.intervalSeconds) * 1_000
    timer = window.setInterval(() => {
        void tick()
    }, ms)
}

/** 改设置：持久化 + 按新参数重启计时器（改间隔要立刻按新周期走）。 */
export function setRotation(patch: Partial<WallpaperRotation>): void {
    if (patch.mode !== undefined) rotationState.mode = patch.mode
    if (patch.intervalSeconds !== undefined) rotationState.intervalSeconds = patch.intervalSeconds
    if (patch.ids !== undefined) rotationState.ids = [...patch.ids]
    persist()
    startRotation()
}

/** 壁纸库里勾选/取消一张。 */
export function toggleRotationId(id: string): void {
    const has = rotationState.ids.includes(id)
    const ids = has ? rotationState.ids.filter((item) => item !== id) : [...rotationState.ids, id]
    setRotation({ ids })
}

export function isRotationId(id: string): boolean {
    return rotationState.ids.includes(id)
}
