<script setup lang="ts">
/**
 * 壁纸轮播选择器（2026-10-25 11:50）
 *
 * 主人要求：「轮播壁纸选择要下拉选项，弹出和后台管理选择壁纸一样的逻辑、形式」
 * ⇒ 不是原生 `<select>`，而是一个**弹出面板 + 缩略图网格多选**：
 * 点控制条里的「已选 N 张 ›」打开，点图即勾选/取消，右下角完成关闭。
 *
 * 复用点：缩略图走 `wallpaperAssetUrl(id, preview)`（和壁纸设置页同一套），
 * 状态直接读写 `rotationState.ids`（不另存副本，关掉面板即刻生效）。
 */
import { NewModal } from '@modrinth/ui'
import { computed, ref } from 'vue'

import {
    isRotationId,
    rotationState,
    setRotation,
    toggleRotationId,
} from '@/helpers/wallpaper-rotation'
import { wallpaperAssetUrl, wallpaperState } from '@/helpers/wallpaper'

const modal = ref<InstanceType<typeof NewModal>>()

function show(): void {
    modal.value?.show()
}

defineExpose({ show })

const items = computed(() => wallpaperState.items)

function thumbOf(id: string, preview: string | null): string | null {
    return preview ? wallpaperAssetUrl(id, preview) : null
}

function initialOf(title: string): string {
    return (title.trim()[0] ?? '?').toUpperCase()
}

function clearAll(): void {
    setRotation({ ids: [] })
}

const selectedCount = computed(() => rotationState.ids.length)

/**
 * 顺序模式下列表顺序就是轮播顺序，勾选时能看到序号。
 * 随机模式没有顺序概念，不显示序号。
 */
function orderOf(id: string): number {
    if (rotationState.mode !== 'sequential') return 0
    return rotationState.ids.indexOf(id) + 1
}
</script>

<template>
    <NewModal ref="modal" :header="'选择轮播壁纸'" max-width="720px">
        <p class="wp-picker-hint">
            点图加入或移出轮播。顺序模式下按勾选先后轮播（图上会标序号）；
            随机模式下无顺序。
        </p>

        <p v-if="!items.length" class="wp-picker-empty">还没有导入任何壁纸。</p>

        <div v-else class="wp-picker-grid">
            <button
                v-for="item in items"
                :key="item.id"
                type="button"
                class="wp-picker-tile"
                :class="{ 'is-picked': isRotationId(item.id) }"
                :aria-pressed="isRotationId(item.id)"
                @click="toggleRotationId(item.id)"
            >
                <img
                    v-if="thumbOf(item.id, item.preview)"
                    :src="thumbOf(item.id, item.preview) ?? undefined"
                    alt=""
                    loading="lazy"
                    decoding="async"
                    draggable="false"
                />
                <span v-else class="wp-picker-initial">{{ initialOf(item.title) }}</span>

                <span v-if="orderOf(item.id)" class="wp-picker-order">{{ orderOf(item.id) }}</span>
                <span class="wp-picker-name">{{ item.title }}</span>
            </button>
        </div>

        <template #actions>
            <div class="wp-picker-actions">
                <button type="button" class="wp-picker-btn" @click="clearAll">清空</button>
                <span class="wp-picker-count">已选 {{ selectedCount }} 张</span>
                <button type="button" class="wp-picker-btn is-primary" @click="modal?.hide()">
                    完成
                </button>
            </div>
        </template>
    </NewModal>
</template>

<style scoped lang="scss">
.wp-picker-hint {
    margin: 0 0 12px;
    font-size: 12px;
    color: var(--text-3);
}

.wp-picker-empty {
    margin: 24px 0;
    font-size: 13px;
    color: var(--text-3);
    text-align: center;
}

.wp-picker-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 10px;
    max-height: 52vh;
    overflow-y: auto;
    padding-right: 4px;
}

.wp-picker-tile {
    position: relative;
    aspect-ratio: 16 / 10;
    border-radius: 10px;
    overflow: hidden;
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    padding: 0;
    cursor: pointer;
    transition:
        border-color 140ms ease,
        transform 140ms ease;

    img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
    }

    &:hover {
        transform: translateY(-1px);
        border-color: var(--c-a);
    }

    /* 选中态用主题色描边 + 角标，和壁纸设置页的"使用中"区分开 */
    &.is-picked {
        border-color: var(--c-a);
        box-shadow: 0 0 0 1px var(--c-a) inset;
    }
}

.wp-picker-initial {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    font-weight: 700;
    color: var(--text-2);
}

.wp-picker-order {
    position: absolute;
    top: 6px;
    left: 6px;
    min-width: 18px;
    height: 18px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--c-a);
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    display: grid;
    place-items: center;
}

.wp-picker-name {
    position: absolute;
    inset: auto 0 0 0;
    padding: 12px 6px 4px;
    font-size: 10.5px;
    color: #fff;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    background: linear-gradient(180deg, transparent, rgba(0, 0, 0, 0.62));
}

.wp-picker-actions {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
}

.wp-picker-count {
    margin-left: auto;
    font-size: 12px;
    color: var(--text-2);
}

.wp-picker-btn {
    padding: 6px 12px;
    border-radius: 8px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--text-1);
    font-size: 12px;
    cursor: pointer;

    &.is-primary {
        background: color-mix(in srgb, var(--c-a) 22%, transparent);
        border-color: color-mix(in srgb, var(--c-a) 45%, transparent);
        color: var(--text-0);
    }
}
</style>
