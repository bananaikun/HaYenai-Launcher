<!--
  音乐（/music）
  ------------------------------------------------------------
  **照后台管理 `components/music/MusicPanel.tsx` 的「播放台 v5」逐块搬过来**：

    .mu-bar    顶部一行（Now Playing 眉题 + 「歌单 N 首 · 第 k 首」 + 重新加载）
    .mu-stage  舞台：左侧方块封面（播放时呼吸 + 主色光晕）／右侧歌词・队列
    .mu-pane   曲名 + 歌手 + 「歌词 / 队列」分段器
    .mu-lyr    歌词区（上下渐隐遮罩，当前行 24px 加粗发光，点击该行跳转）
    .mu-queue  队列（封面/曲名/歌手/序号，当前行高亮 + 跳动条）
    .mu-dock   底部控制坞：进度条 + 播放模式 / 上一首 / 播放 / 下一首 + 音量

  配色与玻璃一律用启动器令牌（`--c-a` / `--grad-a` / `--stroke-1` / `--ui-card-alpha`），
  跟侧栏、标题栏是同一套材质，不另建一套。
-->
<script setup lang="ts">
import { PauseIcon, PlayIcon, RefreshCwIcon } from '@modrinth/assets'
import { defineMessages, useVIntl } from '@modrinth/ui'
import { computed, nextTick, onMounted, ref, watch } from 'vue'

import { METING_SOURCES, hostOf } from '@/helpers/music/music-source'
import {
    currentSong,
    cycleMode,
    formatClock,
    loadPlaylist,
    music,
    next as musicNext,
    playIndex,
    prev as musicPrev,
    seek,
    setVolume,
    toggle,
    toggleMute,
} from '@/store/music'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    nowPlaying: { id: 'app.music.now-playing', defaultMessage: 'Now playing' },
    playlistMeta: { id: 'app.music.playlist-meta', defaultMessage: 'Playlist' },
    trackIndex: { id: 'app.music.track-index', defaultMessage: 'Track' },
    reload: { id: 'app.music.reload', defaultMessage: 'Reload' },
    lyricTab: { id: 'app.music.lyric-tab', defaultMessage: 'Lyrics' },
    queueTab: { id: 'app.music.queue-tab', defaultMessage: 'Queue' },
    empty: { id: 'app.music.empty', defaultMessage: 'No tracks loaded yet.' },
    loading: { id: 'app.music.loading', defaultMessage: 'Connecting to the music source…' },
    noLyrics: { id: 'app.music.no-lyrics', defaultMessage: 'Instrumental — no lyrics.' },
    play: { id: 'app.music.play', defaultMessage: 'Play' },
    pause: { id: 'app.music.pause', defaultMessage: 'Pause' },
    previous: { id: 'app.music.previous', defaultMessage: 'Previous' },
    nextTrack: { id: 'app.music.next', defaultMessage: 'Next' },
    volume: { id: 'app.music.volume', defaultMessage: 'Volume' },
    mute: { id: 'app.music.mute', defaultMessage: 'Mute' },
    unmute: { id: 'app.music.unmute', defaultMessage: 'Unmute' },
    modeLoop: { id: 'app.music.mode-loop', defaultMessage: 'Repeat all' },
    modeSingle: { id: 'app.music.mode-single', defaultMessage: 'Repeat one' },
    modeRandom: { id: 'app.music.mode-random', defaultMessage: 'Shuffle' },
})

const tab = ref<'lyric' | 'list'>('lyric')
const song = computed(() => currentSong())
const progress = computed(() =>
    music.duration > 0 ? Math.min(100, (music.currentTime / music.duration) * 100) : 0,
)

const modeIcon = computed(() =>
    music.mode === 'single' ? '↻¹' : music.mode === 'random' ? '⇄' : '↻',
)
const modeLabel = computed(() =>
    formatMessage(
        music.mode === 'single'
            ? messages.modeSingle
            : music.mode === 'random'
              ? messages.modeRandom
              : messages.modeLoop,
    ),
)
const volIcon = computed(() => (music.muted || music.volume === 0 ? '🔇' : '🔊'))

function onSeek(event: Event) {
    const pct = Number((event.target as HTMLInputElement).value)
    seek((pct / 100) * music.duration)
}

function onVolume(event: Event) {
    setVolume(Number((event.target as HTMLInputElement).value))
}

const activeIndex = computed(() => {
    const lines = music.lyrics
    for (let i = lines.length - 1; i >= 0; i--) {
        if (music.currentTime >= lines[i].at) return i
    }
    return -1
})

const lyricBox = ref<HTMLElement | null>(null)
const activeLine = ref<HTMLElement | null>(null)

/**
 * 把当前行滚到歌词框中线。
 * 与后台同法：用 offsetTop 算，不用 scrollIntoView —— 后者会把外层滚动容器一起带动，
 * 页面本身也跟着跳。切换 tab / 换歌 / 行变化都要重算。
 */
async function centerActiveLine() {
    await nextTick()
    const box = lyricBox.value
    const line = activeLine.value
    if (!box || !line) return
    box.scrollTo({ top: Math.max(0, line.offsetTop - (box.clientHeight - line.clientHeight) / 2) })
}

watch([activeIndex, tab], () => {
    void centerActiveLine()
})

onMounted(() => {
    if (!music.songs.length) void loadPlaylist()
})
</script>

<template>
    <div class="mu">
        <div class="mu-bar">
            <span class="mu-eyebrow">
                {{ formatMessage(messages.nowPlaying) }} · {{ hostOf(METING_SOURCES[0]) }}
            </span>
            <span class="mu-meta">
                <template v-if="music.songs.length">
                    {{ formatMessage(messages.playlistMeta) }} {{ music.songs.length }} ·
                    {{ formatMessage(messages.trackIndex) }} {{ music.index + 1 }}
                </template>
            </span>
            <span class="mu-grow" />
            <button
                type="button"
                class="mu-iconbtn"
                v-tooltip="formatMessage(messages.reload)"
                @click="loadPlaylist()"
            >
                <RefreshCwIcon class="size-3.5" />
            </button>
        </div>

        <div v-if="!music.songs.length" class="mu-empty">
            <span class="mu-meta">
                {{
                    music.loading
                        ? formatMessage(messages.loading)
                        : music.error || formatMessage(messages.empty)
                }}
            </span>
            <button type="button" class="mu-chip" @click="loadPlaylist()">
                {{ formatMessage(messages.reload) }}
            </button>
        </div>

        <template v-else>
            <div class="mu-stage">
                <div class="mu-art">
                    <span class="mu-halo" />
                    <div class="mu-disc" :class="{ on: music.playing }">
                        <img v-if="song?.pic" :src="song.pic" alt="" />
                        <span v-else class="mu-noart">♪</span>
                    </div>
                </div>

                <div class="mu-pane">
                    <div class="mu-head">
                        <div class="mu-stack">
                            <h2 class="mu-title">{{ song?.name ?? '—' }}</h2>
                            <span class="mu-artist">{{ song?.artist ?? '—' }}</span>
                        </div>
                        <div class="mu-seg">
                            <button
                                type="button"
                                :class="{ on: tab === 'lyric' }"
                                @click="tab = 'lyric'"
                            >
                                {{ formatMessage(messages.lyricTab) }}
                            </button>
                            <button
                                type="button"
                                :class="{ on: tab === 'list' }"
                                @click="tab = 'list'"
                            >
                                {{ formatMessage(messages.queueTab) }}
                            </button>
                        </div>
                    </div>

                    <div v-if="tab === 'lyric'" ref="lyricBox" class="mu-lyr">
                        <div class="mu-lyr-inner">
                            <p
                                v-for="(line, index) in music.settings.lyrics ? music.lyrics : []"
                                :key="`${line.at}-${index}`"
                                :ref="index === activeIndex ? (el) => (activeLine = el as HTMLElement) : undefined"
                                class="mu-ln"
                                :class="{ on: index === activeIndex, past: index < activeIndex }"
                                @click="seek(line.at)"
                            >
                                {{ line.text || '♪' }}
                            </p>
                            <p v-if="!music.settings.lyrics" class="mu-ln on">
                                {{ formatMessage(messages.noLyrics) }}
                            </p>
                            <p v-else-if="!music.lyrics.length" class="mu-ln on">
                                {{ formatMessage(messages.noLyrics) }}
                            </p>
                        </div>
                    </div>

                    <div v-else class="mu-queue">
                        <button
                            v-for="(item, index) in music.songs"
                            :key="`${item.id ?? index}`"
                            type="button"
                            class="mu-row"
                            :class="{ on: index === music.index }"
                            @click="playIndex(index)"
                        >
                            <span class="mu-thumb">
                                <img v-if="item.pic" :src="item.pic" alt="" loading="lazy" />
                            </span>
                            <span class="mu-row-text">
                                <span class="t">{{ item.name }}</span>
                                <span class="a">{{ item.artist }}</span>
                            </span>
                            <span v-if="index === music.index && music.playing" class="mu-eq">
                                <i /><i /><i />
                            </span>
                            <span v-else class="mu-num">
                                {{ String(index + 1).padStart(2, '0') }}
                            </span>
                        </button>
                    </div>
                </div>
            </div>

            <div class="mu-dock">
                <div class="mu-prog">
                    <span class="mu-meta num">{{ formatClock(music.currentTime) }}</span>
                    <div class="mu-track">
                        <div class="mu-rail">
                            <div class="mu-fill" :style="{ width: `${progress}%` }" />
                        </div>
                        <input
                            type="range"
                            min="0"
                            max="100"
                            step="0.1"
                            :value="progress"
                            @input="onSeek"
                        />
                    </div>
                    <span class="mu-meta num">{{ formatClock(music.duration) }}</span>
                </div>

                <div class="mu-ctrl">
                    <button type="button" class="mu-btn" v-tooltip="modeLabel" @click="cycleMode">
                        <span class="mu-glyph">{{ modeIcon }}</span>
                    </button>
                    <button
                        type="button"
                        class="mu-btn"
                        v-tooltip="formatMessage(messages.previous)"
                        @click="musicPrev"
                    >
                        <span class="mu-glyph">⏮</span>
                    </button>
                    <button
                        type="button"
                        class="mu-play"
                        v-tooltip="
                            music.playing
                                ? formatMessage(messages.pause)
                                : formatMessage(messages.play)
                        "
                        @click="toggle"
                    >
                        <PauseIcon v-if="music.playing" class="size-5" />
                        <PlayIcon v-else class="size-5" />
                    </button>
                    <button
                        type="button"
                        class="mu-btn"
                        v-tooltip="formatMessage(messages.nextTrack)"
                        @click="musicNext"
                    >
                        <span class="mu-glyph">⏭</span>
                    </button>

                    <div class="mu-vol">
                        <button
                            type="button"
                            class="mu-btn sm"
                            v-tooltip="
                                music.muted
                                    ? formatMessage(messages.unmute)
                                    : formatMessage(messages.mute)
                            "
                            @click="toggleMute"
                        >
                            <span class="mu-glyph sm">{{ volIcon }}</span>
                        </button>
                        <input
                            class="mu-slider"
                            type="range"
                            min="0"
                            max="1"
                            step="0.01"
                            :value="music.muted ? 0 : music.volume"
                            v-tooltip="formatMessage(messages.volume)"
                            @input="onVolume"
                        />
                        <span class="mu-meta num">
                            {{ Math.round((music.muted ? 0 : music.volume) * 100) }}
                        </span>
                    </div>
                </div>
            </div>
        </template>
    </div>
</template>

<style scoped lang="scss">
.mu {
    display: flex;
    flex-direction: column;
    gap: 18px;
    height: 100%;
    min-height: 0;
    padding: 20px 22px 22px;
    overflow-y: auto;
}

.mu-bar {
    display: flex;
    align-items: center;
    gap: 12px;
}

.mu-eyebrow {
    font-size: 10px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--text-3);
}

.mu-meta {
    font-size: 11.5px;
    color: var(--text-2);

    &.num {
        font-variant-numeric: tabular-nums;
        min-width: 34px;
    }
}

.mu-grow {
    flex: 1;
}

.mu-iconbtn {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--text-1);
    cursor: pointer;

    &:hover {
        background: var(--glass-4);
        color: var(--text-0);
    }
}

.mu-chip {
    height: 30px;
    padding: 0 14px;
    border-radius: 10px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--text-1);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
}

.mu-empty {
    display: grid;
    place-items: center;
    gap: 14px;
    padding: 60px 0;
}

/* ---- 舞台 ---- */
.mu-stage {
    display: grid;
    grid-template-columns: minmax(0, 0.78fr) minmax(0, 1.22fr);
    gap: 30px;
    align-items: center;
    padding: 24px;
    border-radius: 20px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-1) 100%),
        color-mix(in srgb, var(--surface-2-opaque) var(--ui-card-alpha, 55%), transparent);
    -webkit-backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(160%);
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(160%);
    box-shadow: var(--inset-1);
}

.mu-art {
    position: relative;
    display: grid;
    place-items: center;
    min-width: 0;
}

.mu-halo {
    position: absolute;
    inset: -6% -2%;
    border-radius: 50%;
    background: radial-gradient(circle at 50% 50%, var(--c-a), transparent 66%);
    filter: blur(48px);
    opacity: 0.45;
    pointer-events: none;
    animation: haloPulse 4.4s var(--ease) infinite;
}

@keyframes haloPulse {
    0%,
    100% {
        opacity: 0.4;
        transform: scale(1);
    }
    50% {
        opacity: 0.72;
        transform: scale(1.06);
    }
}

.mu-disc {
    position: relative;
    width: min(100%, 320px);
    aspect-ratio: 1;
    border-radius: 16px;
    overflow: hidden;
    background: var(--glass-2);
    box-shadow: var(--sh-2), inset 0 0 0 1px var(--stroke-2);

    img {
        display: block;
        width: 100%;
        height: 100%;
        object-fit: cover;
    }

    &.on {
        animation: mu-breathe 6.4s var(--ease) infinite;
    }
}

@keyframes mu-breathe {
    0%,
    100% {
        transform: scale(1);
    }
    50% {
        transform: scale(1.022);
    }
}

.mu-noart {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    font-size: 44px;
    color: var(--text-3);
}

/* ---- 正文 ---- */
.mu-pane {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
}

.mu-head {
    display: flex;
    align-items: flex-start;
    gap: 16px;
}

.mu-stack {
    display: grid;
    gap: 4px;
    min-width: 0;
    flex: 1;
}

.mu-title {
    margin: 0;
    font-size: 26px;
    font-weight: 600;
    line-height: 1.2;
    letter-spacing: -0.02em;
    color: var(--text-0);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mu-artist {
    font-size: 13px;
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mu-seg {
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: 10px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    flex: none;

    button {
        padding: 4px 10px;
        border: 0;
        border-radius: 7px;
        background: none;
        color: var(--text-2);
        font: inherit;
        font-size: 11.5px;
        cursor: pointer;

        &.on {
            background: var(--glass-4);
            color: var(--text-0);
        }
    }
}

/* 歌词：正文主体 */
.mu-lyr {
    position: relative;
    height: min(42vh, 360px);
    overflow-y: auto;
    scrollbar-width: none;
    mask-image: linear-gradient(180deg, transparent 0, #000 15%, #000 85%, transparent 100%);
    -webkit-mask-image: linear-gradient(
        180deg,
        transparent 0,
        #000 15%,
        #000 85%,
        transparent 100%
    );

    &::-webkit-scrollbar {
        display: none;
    }
}

.mu-lyr-inner {
    padding: 36% 0;
}

.mu-ln {
    margin: 0;
    padding: 10px 8px;
    text-align: center;
    font-size: 15px;
    line-height: 1.5;
    color: var(--text-1);
    border-radius: 8px;
    cursor: pointer;
    transition: color var(--t-base) var(--ease);

    &:hover {
        color: var(--text-0);
    }

    &.past {
        color: var(--text-2);
    }

    &.on {
        font-size: 24px;
        font-weight: 700;
        color: var(--text-0);
        text-shadow: 0 0 28px var(--c-a);
    }
}

/* 队列 */
.mu-queue {
    height: min(42vh, 360px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
}

.mu-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--text-1);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
        background 180ms var(--ease),
        transform 180ms var(--ease-out);

    &:hover {
        background: var(--glass-2);
        transform: translateX(3px);
    }

    &.on {
        background: color-mix(in srgb, var(--c-a) 18%, transparent);
        color: var(--text-0);
    }
}

.mu-thumb {
    width: 30px;
    height: 30px;
    flex: none;
    border-radius: 7px;
    overflow: hidden;
    background: var(--glass-2);

    img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
}

.mu-row-text {
    display: grid;
    gap: 1px;
    min-width: 0;
    flex: 1;

    .t,
    .a {
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .t {
        font-size: 12.5px;
    }

    .a {
        font-size: 10.5px;
        color: var(--text-3);
    }
}

.mu-num {
    font-size: 10px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
}

/* 正在播放的三根跳动条 */
.mu-eq {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 12px;

    i {
        width: 2px;
        background: var(--c-a);
        border-radius: 1px;
        animation: eq 900ms ease-in-out infinite;

        &:nth-child(1) {
            height: 60%;
        }
        &:nth-child(2) {
            height: 100%;
            animation-delay: 120ms;
        }
        &:nth-child(3) {
            height: 40%;
            animation-delay: 240ms;
        }
    }
}

@keyframes eq {
    0%,
    100% {
        transform: scaleY(0.5);
    }
    50% {
        transform: scaleY(1);
    }
}

/* ---- 控制坞 ---- */
.mu-dock {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 22px 18px;
    border-radius: 18px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-1) 100%),
        color-mix(in srgb, var(--surface-2-opaque) var(--ui-card-alpha, 55%), transparent);
    -webkit-backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(160%);
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(160%);
    box-shadow: var(--inset-1);
}

.mu-prog {
    display: flex;
    align-items: center;
    gap: 12px;
}

.mu-track {
    position: relative;
    flex: 1;
    height: 16px;
    display: flex;
    align-items: center;

    input[type='range'] {
        position: absolute;
        inset: 0;
        width: 100%;
        opacity: 0;
        cursor: pointer;
    }
}

.mu-rail {
    position: relative;
    width: 100%;
    height: 5px;
    border-radius: 999px;
    background: var(--stroke-1);
    overflow: hidden;
    box-shadow: var(--inset-1);
}

.mu-fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 999px;
    background: var(--grad-a);
    box-shadow: 0 0 12px -1px rgba(var(--c-a-rgb), 0.75);
}

.mu-ctrl {
    display: flex;
    align-items: center;
    gap: 8px;
}

.mu-btn {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--text-1);
    cursor: pointer;
    transition:
        color var(--t-fast) var(--ease),
        background var(--t-fast) var(--ease);

    &:hover {
        color: var(--text-0);
        background: var(--glass-2);
    }

    &.sm {
        width: 32px;
        height: 32px;
    }
}

.mu-glyph {
    font-size: 15px;
    line-height: 1;

    &.sm {
        font-size: 12px;
    }
}

.mu-play {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    margin: 0 6px;
    border: 0;
    border-radius: 50%;
    background: var(--grad-a);
    color: #fff;
    cursor: pointer;
    box-shadow: 0 12px 26px -14px rgba(var(--c-a-rgb), 0.9);
    transition: transform 180ms var(--ease-out);

    &:hover {
        transform: scale(1.05);
    }

    &:active {
        transform: scale(0.96);
    }
}

.mu-vol {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;

    .mu-slider {
        width: 96px;
        accent-color: var(--c-a);
    }
}

@media (max-width: 1000px) {
    .mu-stage {
        grid-template-columns: 1fr;
        gap: 20px;
        padding: 18px;
    }
}
</style>
