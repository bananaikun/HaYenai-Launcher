<script setup lang="ts">
/**
 * 启动背景用的壁纸选择器（2026-11-14 重写）。
 *
 * 为什么单独一个组件、而且**不接收任何外部状态**：
 * 之前把它塞在 SplashSettingsSection 里，数据要经过「父组件的 ref/computed →
 * 两个实例（外观 / 首页与导航）」好几跳，结果数据永远填不进真正显示的那个实例，
 * 网格一直是 0 格。这里自己 onMounted 自己拉，父组件只负责放一个标签。
 *
 * 三种状态都明确显示出来（加载中 / 出错 / 空），不再"点了没反应"。
 */
import { invoke } from '@tauri-apps/api/core'
import { onMounted, ref } from 'vue'

import { wallpaperAssetUrl, wallpaperState } from '@/helpers/wallpaper'

const emit = defineEmits<{ picked: [] }>()

interface WallpaperRow {
    id: string
    title?: string
    type?: string
    preview?: string | null
}

const items = ref<WallpaperRow[]>([])
const loading = ref(true)
const error = ref('')
const busyId = ref<string | null>(null)
const currentId = ref<string | null>(null)

function thumb(item: WallpaperRow): string | null {
    return item.preview ? wallpaperAssetUrl(item.id, item.preview) : null
}

onMounted(async () => {
    try {
        const list = await invoke<WallpaperRow[]>('plugin:wallpaper|wallpaper_list')
        items.value = Array.isArray(list) ? list : []
        // ⚠ 同步写回全局单例：父组件（SplashSettingsSection）的 `wallpapers` 是
        // 从这个单例派生的，只填自己的 items 会让父组件永远看不到壁纸 ——
        // 表现就是「选了壁纸，右侧预览一直是黑的」。
        if (items.value.length) {
            wallpaperState.items = items.value as never
        }
    } catch (cause) {
        error.value = `读取壁纸库失败：${String(cause)}`
    } finally {
        loading.value = false
    }

    // 当前启动背景如果用的是场景型，顺手高亮它（失败不影响选择）。
    try {
        const config = await invoke<{ media?: { kind?: string; file?: string } | null }>(
            'splash_get_config',
        )
        currentId.value = config.media?.kind === 'scene' ? (config.media.file ?? null) : null
    } catch {
        currentId.value = null
    }
})

async function pick(item: WallpaperRow): Promise<void> {
    busyId.value = item.id
    error.value = ''
    try {
        const command =
            item.type === 'scene'
                ? 'splash_set_media_from_scene'
                : 'splash_set_media_from_wallpaper'
        await invoke(command, { id: item.id })
        currentId.value = item.type === 'scene' ? item.id : null
        emit('picked')
    } catch (cause) {
        error.value = `设为启动背景失败：${String(cause)}`
    } finally {
        busyId.value = null
    }
}

defineExpose({ reload: onMounted })
</script>

<template>
    <div class="swp">
        <p v-if="error" class="swp-note is-error">{{ error }}</p>
        <p v-else-if="loading" class="swp-note">正在读取壁纸库…</p>
        <p v-else-if="!items.length" class="swp-note">壁纸库里还没有壁纸。</p>

        <div v-else class="swp-grid">
            <button
                v-for="item in items"
                :key="item.id"
                type="button"
                class="swp-cell"
                :class="{ 'is-current': item.id === currentId }"
                :disabled="busyId !== null"
                :title="item.title || item.id"
                @click="pick(item)"
            >
                <img v-if="thumb(item)" :src="thumb(item) ?? ''" alt="" loading="lazy" />
                <span v-else class="swp-blank" />
                <span class="swp-title">{{ item.title || item.id }}</span>
                <span v-if="busyId === item.id" class="swp-busy">…</span>
            </button>
        </div>
    </div>
</template>

<style scoped>
.swp {
    width: 100%;
}

.swp-note {
    margin: 0.2rem 0;
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}

.swp-note.is-error {
    color: var(--color-red, #e05a5a);
}

.swp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(86px, 1fr));
    gap: 0.35rem;
    max-height: 240px;
    overflow-y: auto;
    padding: 0.1rem;
}

.swp-cell {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0;
    border-radius: 9px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    color: var(--color-text-secondary);
    cursor: pointer;
    overflow: hidden;
    transition:
        border-color var(--t-fast) var(--ease),
        background var(--t-fast) var(--ease);
}

.swp-cell:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--color-brand) 55%, transparent);
    background: var(--glass-2);
}

.swp-cell.is-current {
    border-color: var(--color-brand);
    background: color-mix(in srgb, var(--color-brand) 12%, transparent);
    color: var(--text-0);
}

.swp-cell:disabled {
    opacity: 0.6;
    cursor: default;
}

.swp-cell img,
.swp-blank {
    display: block;
    width: 100%;
    aspect-ratio: 16 / 10;
    object-fit: cover;
    background: var(--glass-2);
}

.swp-title {
    font-size: 0.6rem;
    line-height: 1.2;
    padding: 0 0.25rem 0.25rem;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
}

.swp-busy {
    position: absolute;
    top: 4px;
    right: 6px;
    font-size: 0.7rem;
    color: var(--text-0);
}
</style>
