<script setup lang="ts">
/**
 * 壁纸可调项面板（2026-10-25 12:35）
 *
 * 主人要求：「壁纸里面还有一些配置可以调整的」，点名的包是 Widescreen Mount Fuji Visualiser。
 * 那个包的 `general.properties` 有 28 项：slider 15 / bool 9 / color 2 / group 2，
 * 还带 `condition: "mediaplayertoggle.value == true"` 这种条件显隐。
 *
 * 数据链路（已存在，不重复造）：
 *   Rust `wallpaper_properties` → 本面板渲染控件
 *   改值 → 写 `wallpaperState.config.properties[id][key]` → `setWallpaperConfig`
 *   → `WallpaperLayer` 把它传给 `mount({ properties })`
 *   → `SceneWallpaper` watch `props.properties` 热更（不用重挂）
 *
 * 所以这里**只负责把 schema 画成控件**，不额外发任何消息。
 */
import { invoke } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'

import {
    setWallpaperConfig,
    type WallpaperPropertyValue,
    wallpaperState,
} from '@/helpers/wallpaper'

interface PropertyDef {
    type?: string
    text?: string
    value?: unknown
    min?: number
    max?: number
    step?: number
    precision?: number
    /** fraction=true 表示 0~1 的比例（位置类），UI 上按百分比显示。 */
    fraction?: boolean
    order?: number
    condition?: string
    options?: unknown
}

const props = defineProps<{ wallpaperId: string }>()

const schema = ref<Record<string, PropertyDef> | null>(null)
const loading = ref(false)
const failed = ref<string | null>(null)

/** 当前值：用户改过的优先，否则用 schema 里的出厂值。 */
const values = computed<Record<string, WallpaperPropertyValue>>(
    () => wallpaperState.config.properties[props.wallpaperId] ?? {},
)

function valueOf(key: string, def: PropertyDef): WallpaperPropertyValue {
    const stored = values.value[key]
    if (stored !== undefined) return stored
    const raw = def.value
    if (typeof raw === 'boolean' || typeof raw === 'number' || typeof raw === 'string') return raw
    return def.type === 'bool' ? false : 0
}

/** 按 order 排好序的条目（group 当小标题用）。 */
const entries = computed(() => {
    if (!schema.value) return []
    return Object.entries(schema.value)
        .filter(([, def]) => {
            const type = def?.type
            return type === 'slider' || type === 'bool' || type === 'color' || type === 'group'
        })
        .sort((a, b) => (a[1].order ?? 0) - (b[1].order ?? 0))
})

/**
 * `condition` 只支持作者实际在用的极简形态：`key.value == 值`。
 * 解不出来时**默认显示**（宁可多显示一项，也不要让控件凭空消失）。
 */
function visible(def: PropertyDef): boolean {
    const cond = def.condition?.trim()
    if (!cond) return true
    const match = /^([\w.]+)\.value\s*==\s*(.+)$/.exec(cond)
    if (!match) return true
    const key = match[1]
    const expectedRaw = match[2].trim()
    const actual = values.value[key] ?? schema.value?.[key]?.value
    if (expectedRaw === 'true') return actual === true
    if (expectedRaw === 'false') return actual === false
    if (/^-?\d+(\.\d+)?$/.test(expectedRaw)) return Number(actual) === Number(expectedRaw)
    return actual === expectedRaw.replace(/^["']|["']$/g, '')
}

let saveTimer: number | null = null

/** 滑块拖动会连发 input，攒一下再落盘。 */
function patch(key: string, value: WallpaperPropertyValue): void {
    const id = props.wallpaperId
    const next = { ...(wallpaperState.config.properties[id] ?? {}), [key]: value }
    wallpaperState.config = {
        ...wallpaperState.config,
        properties: { ...wallpaperState.config.properties, [id]: next },
    }

    if (saveTimer !== null) window.clearTimeout(saveTimer)
    saveTimer = window.setTimeout(() => {
        saveTimer = null
        void setWallpaperConfig(wallpaperState.config).catch((error) => {
            console.warn('[wallpaper-properties] 保存失败', error)
        })
    }, 220)
}

/** `"1 1 1"` ↔ `[1, 1, 1]`（WE 的 color 是空格分隔的 0~1 RGB）。 */
function colorParts(value: string): [number, number, number] {
    const parts = value.split(/\s+/).map(Number)
    if (parts.length < 3 || parts.some((part) => !Number.isFinite(part))) return [1, 1, 1]
    return [parts[0], parts[1], parts[2]]
}

function colorCss(value: string): string {
    const [r, g, b] = colorParts(value)
    const to255 = (v: number) => Math.round(Math.max(0, Math.min(1, v)) * 255)
    return `rgb(${to255(r)}, ${to255(g)}, ${to255(b)})`
}

function setColor(key: string, channel: number, next: number): void {
    const current = String(values.value[key] ?? schema.value?.[key]?.value ?? '1 1 1')
    const parts = colorParts(current)
    parts[channel] = next
    patch(key, parts.map((part) => Number(part.toFixed(3))).join(' '))
}

function sliderText(def: PropertyDef, value: WallpaperPropertyValue): string {
    const numeric = Number(value)
    if (!Number.isFinite(numeric)) return '—'
    if (def.fraction) return `${Math.round(numeric * 100)}%`
    const precision = def.precision ?? 0
    return numeric.toFixed(precision)
}

/** 中文语境下把作者写的英文标签保留原样（壁纸作者的定义就是英文，翻译反而认不出）。 */
async function load(): Promise<void> {
    schema.value = null
    failed.value = null
    loading.value = true
    try {
        const raw = await invoke<Record<string, PropertyDef> | null>(
            'plugin:wallpaper|wallpaper_properties',
            { id: props.wallpaperId },
        )
        schema.value = raw && typeof raw === 'object' ? raw : null
    } catch (error) {
        failed.value = error instanceof Error ? error.message : String(error)
        console.warn('[wallpaper-properties] 读取失败', error)
    } finally {
        loading.value = false
    }
}

watch(() => props.wallpaperId, () => void load(), { immediate: true })
</script>

<template>
    <div v-if="loading" class="wp-props-empty">读取可调项…</div>
    <div v-else-if="failed" class="wp-props-empty is-error">{{ failed }}</div>
    <div v-else-if="!entries.length" class="wp-props-empty">这张壁纸没有可调项。</div>

    <div v-else class="wp-props">
        <template v-for="[key, def] in entries" :key="key">
            <!-- 分组标题 -->
            <div v-if="def.type === 'group'" class="wp-props-group">{{ def.text ?? key }}</div>

            <template v-else-if="visible(def)">
                <!-- 开关 -->
                <label v-if="def.type === 'bool'" class="wp-props-row is-switch">
                    <span class="wp-props-label">{{ def.text ?? key }}</span>
                    <input
                        type="checkbox"
                        :checked="valueOf(key, def) === true"
                        @change="patch(key, ($event.target as HTMLInputElement).checked)"
                    />
                </label>

                <!-- 单值滑块 -->
                <label v-else-if="def.type === 'slider'" class="wp-props-row">
                    <span class="wp-props-label">{{ def.text ?? key }}</span>
                    <input
                        type="range"
                        :min="def.min ?? 0"
                        :max="def.max ?? 1"
                        :step="def.step ?? 0.01"
                        :value="Number(valueOf(key, def))"
                        @input="patch(key, Number(($event.target as HTMLInputElement).value))"
                    />
                    <b class="wp-props-value">{{ sliderText(def, valueOf(key, def)) }}</b>
                </label>

                <!-- 颜色：RGB 三通道 + 色块预览 -->
                <div v-else-if="def.type === 'color'" class="wp-props-row is-color">
                    <span class="wp-props-label">{{ def.text ?? key }}</span>
                    <span
                        class="wp-props-swatch"
                        :style="{ background: colorCss(String(valueOf(key, def))) }"
                    />
                    <input
                        v-for="channel in 3"
                        :key="channel"
                        type="range"
                        min="0"
                        max="1"
                        step="0.01"
                        :value="colorParts(String(valueOf(key, def)))[channel - 1]"
                        @input="
                            setColor(
                                key,
                                channel - 1,
                                Number(($event.target as HTMLInputElement).value),
                            )
                        "
                    />
                </div>
            </template>
        </template>
    </div>
</template>

<style scoped lang="scss">
.wp-props {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 46vh;
    overflow-y: auto;
    padding-right: 4px;
}

.wp-props-empty {
    font-size: 12px;
    color: var(--text-3);
    padding: 10px 2px;

    &.is-error {
        color: #ff9d9d;
    }
}

.wp-props-group {
    margin-top: 10px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--stroke-1);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-3);
}

.wp-props-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 130px 52px;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-1);

    &.is-switch {
        grid-template-columns: minmax(0, 1fr) auto;
    }

    &.is-color {
        grid-template-columns: minmax(0, 1fr) 22px repeat(3, minmax(0, 52px));
        row-gap: 2px;
    }

    input[type='range'] {
        width: 100%;
        accent-color: var(--c-a);
        cursor: pointer;
    }

    input[type='checkbox'] {
        accent-color: var(--c-a);
        cursor: pointer;
    }
}

.wp-props-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.wp-props-value {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
    font-weight: 600;
}

.wp-props-swatch {
    width: 20px;
    height: 20px;
    border-radius: 5px;
    border: 1px solid var(--stroke-1);
}
</style>
