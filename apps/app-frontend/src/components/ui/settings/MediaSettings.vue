<!--
  设置 → 音乐与歌词
  ------------------------------------------------------------
  从「外观」里独立出来的分类：音乐是功能，不是外观。

  这里放的四项，正好对应磁悬条 / 音乐页共用的那份状态：
    · 歌单 ID       —— 换歌单不用改代码
    · 音源           —— Meting 的四个镜像，按顺序降级
    · 歌词           —— 开关（关掉后歌词区显示「纯音乐」）
    · 自动播放       —— 打开应用后是否自动开播（默认关，尊重「别自己出声」）
-->
<script setup lang="ts">
import { Button, defineMessages, Toggle, useVIntl } from '@modrinth/ui'
import { computed, ref } from 'vue'

import { METING_SOURCES, hostOf } from '@/helpers/music/music-source'
import { loadPlaylist, music, musicSpec, setMusicSpec, updateSettings } from '@/store/music'

import SettingsSection from './SettingsSection.vue'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: { id: 'app.settings.media.title', defaultMessage: 'Music & lyrics' },
    description: {
        id: 'app.settings.media.description',
        defaultMessage:
            'Shared by the bottom bar and the Music page. Everything talks to the Meting source directly — nothing goes through your server.',
    },
    playlist: { id: 'app.settings.media.playlist', defaultMessage: 'Playlist ID' },
    playlistHint: {
        id: 'app.settings.media.playlist-hint',
        defaultMessage: 'A NetEase playlist id (digits), or comma-separated song ids.',
    },
    sources: { id: 'app.settings.media.sources', defaultMessage: 'Sources' },
    sourcesHint: {
        id: 'app.settings.media.sources-hint',
        defaultMessage: 'Tried in order — the first one that answers wins.',
    },
    lyrics: { id: 'app.settings.media.lyrics', defaultMessage: 'Show lyrics' },
    lyricsHint: {
        id: 'app.settings.media.lyrics-hint',
        defaultMessage: 'When off, the lyric area shows “instrumental” instead of the words.',
    },
    autoplay: { id: 'app.settings.media.autoplay', defaultMessage: 'Autoplay on launch' },
    autoplayHint: {
        id: 'app.settings.media.autoplay-hint',
        defaultMessage: 'Start playing as soon as the launcher opens. Off by default.',
    },
    loaded: { id: 'app.settings.media.loaded', defaultMessage: 'Loaded tracks' },
    reload: { id: 'app.settings.media.reload', defaultMessage: 'Reload playlist' },
})

const playlistInput = ref(musicSpec())

function applyPlaylist() {
    const value = playlistInput.value.trim()
    if (!value) {
        playlistInput.value = musicSpec()
        return
    }
    setMusicSpec(value)
    void loadPlaylist(value)
}

const lyricEnabled = computed(() => music.settings.lyrics)
const autoplayEnabled = computed(() => music.settings.autoplay)
</script>

<template>
    <SettingsSection>
        <template #header>
            <h2
                id="settings-target-media"
                tabindex="-1"
                class="m-0 text-lg font-semibold text-[var(--color-text-primary)]"
            >
                {{ formatMessage(messages.title) }}
            </h2>
            <p class="m-0 mt-1 text-sm leading-relaxed text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.description) }}
            </p>
        </template>

        <div class="grid gap-5 lg:grid-cols-2">
            <div class="flex flex-col gap-2">
                <h3 class="m-0 font-semibold text-[var(--color-text-primary)]">
                    {{ formatMessage(messages.playlist) }}
                </h3>
                <input
                    v-model="playlistInput"
                    type="text"
                    class="h-9 rounded-lg px-3 text-sm"
                    @keyup.enter="applyPlaylist"
                    @blur="applyPlaylist"
                />
                <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.playlistHint) }}
                </p>
            </div>

            <div class="flex flex-col gap-2">
                <h3 class="m-0 font-semibold text-[var(--color-text-primary)]">
                    {{ formatMessage(messages.sources) }}
                </h3>
                <div class="flex flex-wrap gap-2">
                    <span
                        v-for="source in METING_SOURCES"
                        :key="source"
                        class="rounded-full border border-[var(--stroke-1)] bg-[var(--glass-2)] px-3 py-1 text-xs text-[var(--color-text-secondary)]"
                    >
                        {{ hostOf(source) }}
                    </span>
                </div>
                <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.sourcesHint) }}
                </p>
            </div>
        </div>

        <div class="mt-5 grid gap-3">
            <div class="flex items-center justify-between gap-4">
                <div>
                    <p class="m-0 font-medium text-[var(--color-text-primary)]">
                        {{ formatMessage(messages.lyrics) }}
                    </p>
                    <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.lyricsHint) }}
                    </p>
                </div>
                <Toggle
                    id="music-lyrics"
                    :model-value="lyricEnabled"
                    @update:model-value="(v) => updateSettings({ lyrics: !!v })"
                />
            </div>
            <div class="flex items-center justify-between gap-4">
                <div>
                    <p class="m-0 font-medium text-[var(--color-text-primary)]">
                        {{ formatMessage(messages.autoplay) }}
                    </p>
                    <p class="m-0 text-sm text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.autoplayHint) }}
                    </p>
                </div>
                <Toggle
                    id="music-autoplay"
                    :model-value="autoplayEnabled"
                    @update:model-value="(v) => updateSettings({ autoplay: !!v })"
                />
            </div>
        </div>

        <div class="mt-5 flex items-center justify-between">
            <span class="text-sm text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.loaded) }}: {{ music.songs.length }}
            </span>
            <Button @click="applyPlaylist">{{ formatMessage(messages.reload) }}</Button>
        </div>
    </SettingsSection>
</template>
