<script setup lang="ts">
/**
 * 首页「正在播放」小组件：封面 + 歌名 + 当前歌词行，点一下切播放/暂停。
 *
 * 数据直接读全局音乐 store（和磁悬条那一页同源），不额外拉接口 ——
 * 两边不一致会让人以为「歌词卡住了」。
 */
import { defineMessages, useVIntl } from '@modrinth/ui'
import { computed } from 'vue'

import {
    currentSong,
    music,
    next as musicNext,
    prev as musicPrev,
    toggle as musicToggle,
} from '@/store/music'

defineProps<{ dashboardSize?: string | null }>()

const { formatMessage } = useVIntl()

const messages = defineMessages({
    empty: { id: 'app.home.lyrics.empty', defaultMessage: 'Nothing playing' },
    hint: { id: 'app.home.lyrics.hint', defaultMessage: 'Start the music player' },
})

const song = computed(() => currentSong())
const lyric = computed(() => {
    // 歌词关掉时不要显示上一句残留 —— 直接退回提示。
    if (!music.settings.lyrics) return ''
    return music.activeLyric || ''
})
</script>

<template>
    <div class="home-lyrics">
        <template v-if="song">
            <span class="hl-cover">
                <img v-if="song.pic" :src="song.pic" alt="" loading="lazy" decoding="async" />
                <i v-else class="hl-cover-fallback" />
            </span>
            <div class="hl-body">
                <span class="hl-title">{{ song.name }}</span>
                <span class="hl-line">{{ lyric || song.artist }}</span>
                <div class="hl-actions">
                    <button
                        type="button"
                        :aria-label="formatMessage(messages.hint)"
                        @click="musicPrev"
                    >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M18 6v12l-9-6z" />
                            <path d="M6 6v12" />
                        </svg>
                    </button>
                    <button
                        type="button"
                        :aria-label="formatMessage(messages.hint)"
                        @click="musicToggle"
                    >
                        <svg v-if="music.playing" viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M9 6v12M15 6v12" />
                        </svg>
                        <svg v-else viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M8 5l11 7-11 7z" />
                        </svg>
                    </button>
                    <button
                        type="button"
                        :aria-label="formatMessage(messages.hint)"
                        @click="musicNext"
                    >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M6 6v12l9-6z" />
                            <path d="M18 6v12" />
                        </svg>
                    </button>
                </div>
            </div>
        </template>
        <div v-else class="hl-empty">
            <strong>{{ formatMessage(messages.empty) }}</strong>
            <span>{{ formatMessage(messages.hint) }}</span>
        </div>
    </div>
</template>

<style scoped>
.home-lyrics {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    height: 100%;
    min-height: 0;
    padding: 0.55rem 0.7rem;
    overflow: hidden;
}

.hl-cover {
    width: 54px;
    height: 54px;
    border-radius: 10px;
    overflow: hidden;
    flex-shrink: 0;
    border: 1px solid var(--stroke-1);
}

.hl-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.hl-cover-fallback {
    display: block;
    width: 100%;
    height: 100%;
    background: var(--glass-2);
}

.hl-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
}

.hl-title {
    font-size: 0.8rem;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.hl-line {
    font-size: 0.72rem;
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.hl-actions {
    display: flex;
    gap: 0.15rem;
    margin-top: 0.2rem;
}

.hl-actions button {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.hl-actions button:hover {
    background: var(--glass-2);
    border-color: var(--stroke-1);
    color: var(--text-0);
}

.hl-actions svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
}

.hl-empty {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    margin: auto;
    text-align: center;
}

.hl-empty strong {
    font-size: 0.78rem;
}

.hl-empty span {
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}
</style>
