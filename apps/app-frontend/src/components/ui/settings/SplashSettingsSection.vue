<script setup lang="ts">
/**
 * 「启动动画」设置分区。
 *
 * 自包含：配置存 `splash/config.json`（不走设置表），素材复制到 `<settings>/splash/`。
 * 启动画面在数据库初始化之前就要出图，所以这一块不能依赖 State 里的任何东西。
 */
import { ImageIcon, TrashIcon, VideoIcon } from '@modrinth/assets'
import { Button, defineMessages, injectNotificationManager, StyledInput, Toggle, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, onMounted, ref } from 'vue'

const { formatMessage } = useVIntl()
const { addNotification } = injectNotificationManager()

const messages = defineMessages({
    title: { id: 'app.splash-settings.title', defaultMessage: 'Startup animation' },
    description: {
        id: 'app.splash-settings.description',
        defaultMessage:
            'Replace the cold-start screen with your own wallpaper, image or video. Leave it empty to keep the built-in particle background.',
    },
    media: { id: 'app.splash-settings.media', defaultMessage: 'Custom background' },
    mediaEmpty: {
        id: 'app.splash-settings.media-empty',
        defaultMessage: 'Using the built-in animated background.',
    },
    pick: { id: 'app.splash-settings.pick', defaultMessage: 'Choose file' },
    replace: { id: 'app.splash-settings.replace', defaultMessage: 'Replace' },
    clear: { id: 'app.splash-settings.clear', defaultMessage: 'Remove' },
    hint: {
        id: 'app.splash-settings.hint',
        defaultMessage:
            'Images (png / jpg / webp / gif) or video (mp4 / webm / mov), up to 200 MB. The file is copied into the launcher data folder, so moving the original will not break it.',
    },
    elements: { id: 'app.splash-settings.elements', defaultMessage: 'Elements' },
    elementsHint: {
        id: 'app.splash-settings.elements-hint',
        defaultMessage: 'Turn off anything you do not want on the startup screen.',
    },
    showIcon: { id: 'app.splash-settings.show-icon', defaultMessage: 'App icon' },
    showTagline: { id: 'app.splash-settings.show-tagline', defaultMessage: 'Tagline' },
    showProgress: {
        id: 'app.splash-settings.show-progress',
        defaultMessage: 'Progress bar',
    },
    tagline: { id: 'app.splash-settings.tagline', defaultMessage: 'Custom tagline' },
    taglineHint: {
        id: 'app.splash-settings.tagline-hint',
        defaultMessage: 'Leave empty to keep the built-in text.',
    },
    saved: { id: 'app.splash-settings.saved', defaultMessage: 'Startup animation updated' },
})

interface SplashConfig {
    media: { kind: 'image' | 'video'; file: string } | null
    showIcon: boolean
    showTagline: boolean
    showProgress: boolean
    tagline: string
}

const config = ref<SplashConfig>({
    media: null,
    showIcon: true,
    showTagline: true,
    showProgress: true,
    tagline: '',
})
const loading = ref(true)
const busy = ref(false)

const mediaLabel = computed(() => {
    const media = config.value.media
    if (!media) return formatMessage(messages.mediaEmpty)
    return `${media.file} · ${media.kind === 'video' ? 'video' : 'image'}`
})

onMounted(async () => {
    try {
        config.value = await invoke<SplashConfig>('splash_get_config')
    } catch (error) {
        report(error)
    } finally {
        loading.value = false
    }
})

function report(error: unknown): void {
    const text =
        error instanceof Error
            ? error.message
            : error && typeof error === 'object' && 'message' in error
              ? String((error as { message: unknown }).message)
              : String(error)
    addNotification({ title: formatMessage(messages.title), text, type: 'error' })
}

async function persist(next: Partial<SplashConfig>): Promise<void> {
    const merged = { ...config.value, ...next }
    config.value = merged
    busy.value = true
    try {
        config.value = await invoke<SplashConfig>('splash_set_config', { config: merged })
    } catch (error) {
        report(error)
    } finally {
        busy.value = false
    }
}

async function pickMedia(): Promise<void> {
    const selected = await open({
        multiple: false,
        filters: [
            { name: 'Media', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'avif', 'mp4', 'webm', 'mov', 'm4v'] },
        ],
    })
    if (typeof selected !== 'string') return
    busy.value = true
    try {
        config.value = await invoke<SplashConfig>('splash_set_media', { path: selected })
        addNotification({
            title: formatMessage(messages.title),
            text: formatMessage(messages.saved),
            type: 'success',
        })
    } catch (error) {
        report(error)
    } finally {
        busy.value = false
    }
}

async function clearMedia(): Promise<void> {
    busy.value = true
    try {
        config.value = await invoke<SplashConfig>('splash_clear_media')
    } catch (error) {
        report(error)
    } finally {
        busy.value = false
    }
}
</script>

<template>
    <div class="flex flex-col gap-5">
        <div class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center justify-between gap-3">
                <div class="min-w-0">
                    <p class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                        {{ formatMessage(messages.media) }}
                    </p>
                    <p class="m-0 mt-0.5 truncate text-xs text-[var(--color-text-tertiary)]">
                        {{ loading ? '…' : mediaLabel }}
                    </p>
                </div>
                <div class="flex shrink-0 items-center gap-2">
                    <Button type="base" :disabled="busy" @click="pickMedia">
                        <component :is="config.media?.kind === 'video' ? VideoIcon : ImageIcon" />
                        {{
                            formatMessage(
                                config.media ? messages.replace : messages.pick,
                            )
                        }}
                    </Button>
                    <Button v-if="config.media" type="quiet" color="red" :disabled="busy" @click="clearMedia">
                        <TrashIcon />{{ formatMessage(messages.clear) }}
                    </Button>
                </div>
            </div>
            <p class="m-0 text-xs leading-relaxed text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.hint) }}
            </p>
        </div>

        <div class="flex flex-col gap-3 border-t border-solid border-[var(--surface-5)] pt-4">
            <div>
                <p class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                    {{ formatMessage(messages.elements) }}
                </p>
                <p class="m-0 mt-0.5 text-xs text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.elementsHint) }}
                </p>
            </div>

            <label class="flex items-center justify-between gap-4">
                <span class="text-sm text-[var(--color-text-primary)]">{{
                    formatMessage(messages.showIcon)
                }}</span>
                <Toggle
                    id="splash-show-icon"
                    :model-value="config.showIcon"
                    :disabled="busy"
                    @update:model-value="(value) => persist({ showIcon: value })"
                />
            </label>
            <label class="flex items-center justify-between gap-4">
                <span class="text-sm text-[var(--color-text-primary)]">{{
                    formatMessage(messages.showTagline)
                }}</span>
                <Toggle
                    id="splash-show-tagline"
                    :model-value="config.showTagline"
                    :disabled="busy"
                    @update:model-value="(value) => persist({ showTagline: value })"
                />
            </label>
            <label class="flex items-center justify-between gap-4">
                <span class="text-sm text-[var(--color-text-primary)]">{{
                    formatMessage(messages.showProgress)
                }}</span>
                <Toggle
                    id="splash-show-progress"
                    :model-value="config.showProgress"
                    :disabled="busy"
                    @update:model-value="(value) => persist({ showProgress: value })"
                />
            </label>

            <div class="flex flex-col gap-1.5">
                <span class="text-sm text-[var(--color-text-primary)]">{{
                    formatMessage(messages.tagline)
                }}</span>
                <StyledInput
                    :model-value="config.tagline"
                    :disabled="busy || !config.showTagline"
                    :placeholder="formatMessage(messages.taglineHint)"
                    wrapper-class="w-full"
                    @update:model-value="(value) => persist({ tagline: String(value) })"
                />
            </div>
        </div>
    </div>
</template>
