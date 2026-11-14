<script setup lang="ts">
/**
 * 「启动动画」设置分区。
 *
 * 自包含：配置存 `splash/config.json`（不走设置表），素材复制到 `<settings>/splash/`。
 * 启动画面在数据库初始化之前就要出图，所以这一块不能依赖 State 里的任何东西。
 */
import { ImageIcon, TrashIcon, VideoIcon } from '@modrinth/assets'
import {
    Button,
    Combobox,
    defineMessages,
    injectNotificationManager,
    NewModal,
    OverflowMenu,
    StyledInput,
    Toggle,
    useVIntl,
} from '@modrinth/ui'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

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
    animation: { id: 'app.splash-settings.animation', defaultMessage: 'Built-in animation' },
    animationHint: {
        id: 'app.splash-settings.animation-hint',
        defaultMessage:
            'Only applies when no custom background is set. Changes take effect on the next cold start.',
    },
    style: { id: 'app.splash-settings.style', defaultMessage: 'Style' },
    customAdjust: { id: 'app.splash-settings.custom-adjust', defaultMessage: 'Customise' },
    adjustTitle: {
        id: 'app.splash-settings.adjust-title',
        defaultMessage: 'Customise startup screen',
    },
    adjustHint: {
        id: 'app.splash-settings.adjust-hint',
        defaultMessage:
            'Drag the tagline or the progress indicator inside the preview to place them.',
    },
    scrim: { id: 'app.splash-settings.scrim', defaultMessage: 'Dimming' },
    blur: { id: 'app.splash-settings.blur', defaultMessage: 'Blur' },
    progressStyle: { id: 'app.splash-settings.progress-style', defaultMessage: 'Progress style' },
    progressBar: { id: 'app.splash-settings.progress-bar', defaultMessage: 'Bar' },
    progressDots: { id: 'app.splash-settings.progress-dots', defaultMessage: 'Dots' },
    progressStripes: { id: 'app.splash-settings.progress-stripes', defaultMessage: 'Stripes' },
    progressPulse: { id: 'app.splash-settings.progress-pulse', defaultMessage: 'Pulse' },
    progressWave: { id: 'app.splash-settings.progress-wave', defaultMessage: 'Wave' },
    pickWallpaper: {
        id: 'app.splash-settings.pick-wallpaper',
        defaultMessage: 'From wallpaper library',
    },
    noWallpapers: {
        id: 'app.splash-settings.no-wallpapers',
        defaultMessage: 'No wallpapers found in the library yet.',
    },
    pickWallpaperHint: {
        id: 'app.splash-settings.pick-wallpaper-hint',
        defaultMessage:
            'Uses the poster image of that Wallpaper Engine item — the startup screen cannot render a live scene before the app finishes booting.',
    },
    themeColors: {
        id: 'app.splash-settings.theme-colors',
        defaultMessage: 'Match wallpaper colours',
    },
    themeColorsHint: {
        id: 'app.splash-settings.theme-colors-hint',
        defaultMessage: 'Samples the current background and tints the progress bar with it.',
    },
    progressRing: { id: 'app.splash-settings.progress-ring', defaultMessage: 'Ring' },
    progressNone: { id: 'app.splash-settings.progress-none', defaultMessage: 'Hidden' },
    preview: { id: 'app.splash-settings.preview', defaultMessage: 'Preview' },
    resetLayout: { id: 'app.splash-settings.reset-layout', defaultMessage: 'Reset layout' },
    styleAuto: { id: 'app.splash-settings.style-auto', defaultMessage: 'Auto' },
    styleShards: { id: 'app.splash-settings.style-shards', defaultMessage: 'Shards' },
    styleParticles: { id: 'app.splash-settings.style-particles', defaultMessage: 'Particles' },
    styleStatic: { id: 'app.splash-settings.style-static', defaultMessage: 'Static' },
})

interface SplashAnimation {
    style: 'auto' | 'shards' | 'particles' | 'static'
    count: number
    speed: number
    glow: number
    shardColor: string
    accentColor: string
    mouseInteraction: boolean
}

const DEFAULT_ANIMATION: SplashAnimation = {
    style: 'auto',
    count: 900,
    speed: 0.8,
    glow: 0.35,
    shardColor: '#7aa2ff',
    accentColor: '#9fd8ff',
    mouseInteraction: true,
}

interface SplashLayout {
    scrim: number
    blur: number
    progressStyle: 'bar' | 'dots' | 'ring' | 'none'
    taglineX: number
    taglineY: number
    progressX: number
    progressY: number
    accentA: string
    accentB: string
}

const DEFAULT_LAYOUT: SplashLayout = {
    scrim: 0,
    blur: 0,
    progressStyle: 'bar',
    taglineX: 0.5,
    taglineY: 0.66,
    progressX: 0.5,
    progressY: 0.84,
    accentA: '',
    accentB: '',
}

interface SplashConfig {
    media: { kind: 'image' | 'video'; file: string } | null
    showIcon: boolean
    showTagline: boolean
    showProgress: boolean
    tagline: string
    animation: SplashAnimation
    layout: SplashLayout
}

const config = ref<SplashConfig>({
    media: null,
    showIcon: true,
    showTagline: true,
    showProgress: true,
    tagline: '',
    animation: { ...DEFAULT_ANIMATION },
    layout: { ...DEFAULT_LAYOUT },
})
const loading = ref(true)
const busy = ref(false)

const STYLE_OPTIONS = [
    { id: 'auto', labelKey: 'styleAuto' },
    { id: 'shards', labelKey: 'styleShards' },
    { id: 'particles', labelKey: 'styleParticles' },
    { id: 'static', labelKey: 'styleStatic' },
] as const

function styleMenu() {
    return STYLE_OPTIONS.map((option) => ({
        id: option.id,
        label: formatMessage(messages[option.labelKey]),
        disabled: animation.value.style === option.id,
        action: () => patchAnimation({ style: option.id }),
    }))
}

const styleLabel = computed(() => {
    const option = STYLE_OPTIONS.find((item) => item.id === animation.value.style)
    return formatMessage(messages[(option ?? STYLE_OPTIONS[0]).labelKey])
})

const mediaLabel = computed(() => {
    const media = config.value.media
    if (!media) return formatMessage(messages.mediaEmpty)
    return `${media.file} · ${media.kind === 'video' ? 'video' : 'image'}`
})

onBeforeUnmount(() => stopPreviewProgress())

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

const animation = computed(() => config.value.animation ?? DEFAULT_ANIMATION)
const layout = computed<SplashLayout>(() => ({ ...DEFAULT_LAYOUT, ...(config.value.layout ?? {}) }))

/** 拖拽时只改本地值（别每帧写盘），松手再 persist。 */
function setLayoutLocal(patch: Partial<SplashLayout>): void {
    config.value = { ...config.value, layout: { ...layout.value, ...patch } }
}

function persistLayout(patch: Partial<SplashLayout>): void {
    void persist({ layout: { ...layout.value, ...patch } })
}

/** 只改动一个子字段，整块 `animation` 一起提交（后端按整份 config 覆盖）。 */
function patchAnimation(patch: Partial<SplashAnimation>): void {
    void persist({ animation: { ...animation.value, ...patch } })
}

const adjustModal = ref<InstanceType<typeof NewModal> | null>(null)
const previewUrl = ref<string | null>(null)
const previewProgress = ref(0)

function openAdjust(): void {
    adjustModal.value?.show()
    void refreshPreview()
    void loadWallpapers()
    startPreviewProgress()
}

function setScrim(event: Event): void {
    setLayoutLocal({ scrim: Number((event.target as HTMLInputElement).value) })
}

function setBlur(event: Event): void {
    setLayoutLocal({ blur: Number((event.target as HTMLInputElement).value) })
}

const stageMediaStyle = computed(() => ({
    filter: layout.value.blur > 0 ? `blur(${layout.value.blur}px)` : undefined,
    transform: layout.value.blur > 0 ? 'scale(1.06)' : undefined,
}))

const ringOffset = computed(() => ({
    strokeDashoffset: String(113 - (113 * previewProgress.value) / 100),
}))

/** 预览用的素材地址（和启动画面同一条路径）。 */
async function refreshPreview(): Promise<void> {
    if (!config.value.media) {
        previewUrl.value = null
        return
    }
    try {
        const path = await invoke<string | null>('splash_media_path')
        previewUrl.value = path ? convertFileSrc(path) : null
    } catch {
        previewUrl.value = null
    }
}

/* 预览里的进度条要动起来，才看得出样式 */
let progressTimer: number | undefined
function startPreviewProgress(): void {
    window.clearInterval(progressTimer)
    progressTimer = window.setInterval(() => {
        previewProgress.value = (previewProgress.value + 2) % 100
    }, 60)
}

function stopPreviewProgress(): void {
    window.clearInterval(progressTimer)
}

const stageEl = ref<HTMLElement | null>(null)
type DragTarget = 'tagline' | 'progress'
let dragging: DragTarget | null = null

function onPointerDown(target: DragTarget, event: PointerEvent): void {
    event.preventDefault()
    dragging = target
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}

function onPointerMove(event: PointerEvent): void {
    if (!dragging || !stageEl.value) return
    const rect = stageEl.value.getBoundingClientRect()
    const x = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width))
    const y = Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height))
    if (dragging === 'tagline') setLayoutLocal({ taglineX: x, taglineY: y })
    else setLayoutLocal({ progressX: x, progressY: y })
}

function onPointerUp(): void {
    if (!dragging) return
    const target = dragging
    dragging = null
    persistLayout(
        target === 'tagline'
            ? { taglineX: layout.value.taglineX, taglineY: layout.value.taglineY }
            : { progressX: layout.value.progressX, progressY: layout.value.progressY },
    )
}

const PREVIEW_STYLES = [
    { id: 'bar', labelKey: 'progressBar' },
    { id: 'stripes', labelKey: 'progressStripes' },
    { id: 'pulse', labelKey: 'progressPulse' },
    { id: 'wave', labelKey: 'progressWave' },
    { id: 'dots', labelKey: 'progressDots' },
    { id: 'ring', labelKey: 'progressRing' },
    { id: 'none', labelKey: 'progressNone' },
] as const

function progressMenu() {
    return PREVIEW_STYLES.map((option) => ({
        id: option.id,
        label: formatMessage(messages[option.labelKey]),
        disabled: layout.value.progressStyle === option.id,
        action: () => persistLayout({ progressStyle: option.id }),
    }))
}

const progressLabel = computed(() => {
    const option = PREVIEW_STYLES.find((item) => item.id === layout.value.progressStyle)
    return formatMessage(messages[(option ?? PREVIEW_STYLES[0]).labelKey])
})

/* ---------------- 壁纸库（WE 场景也在里面） ---------------- */
interface WallpaperOption {
    id: string
    title: string
    /** `scene` 的可以真渲染，其余（视频/网页）只能用海报帧。 */
    type: string
}

const wallpapers = ref<WallpaperOption[]>([])
const wallpapersLoading = ref(false)

const wallpaperOptions = computed(() =>
    wallpapers.value.map((item) => ({ value: item.id, label: item.title })),
)

const wallpaperSelection = computed({
    get: () => '',
    set: (value: string) => {
        if (value) void applyWallpaper(value)
    },
})

async function loadWallpapers(): Promise<void> {
    if (wallpapers.value.length || wallpapersLoading.value) return
    wallpapersLoading.value = true
    try {
        const list = await invoke<Array<{ id: string; title?: string; type?: string }>>(
            'plugin:wallpaper|wallpaper_list',
        )
        wallpapers.value = list.map((item) => ({
            id: item.id,
            title: item.title || item.id,
            type: item.type ?? 'other',
        }))
    } catch (error) {
        report(error)
    } finally {
        wallpapersLoading.value = false
    }
}

/** 用壁纸库里的那张（WE 场景/视频都行）的**海报图**当启动画面背景。 */
async function applyWallpaper(id: string): Promise<void> {
    busy.value = true
    try {
        // 场景壁纸走「真渲染」：只记壁纸 id，启动画面里跑 webwallgl。
        // 视频/网页壁纸没法在启动阶段实时跑，退回海报帧。
        const target = wallpapers.value.find((item) => item.id === id)
        const command =
            target?.type === 'scene'
                ? 'splash_set_media_from_scene'
                : 'splash_set_media_from_wallpaper'
        config.value = await invoke<SplashConfig>(command, { id })
        await refreshPreview()
        await applyThemeColors()
    } catch (error) {
        report(error)
    } finally {
        busy.value = false
    }
}

/**
 * 从当前背景图取主题色：缩到 24×24 取平均，再生成一对（主色 + 提亮版）。
 *
 * 为什么不在启动画面里现算：那要多解码一次图，冷启动等不起；
 * 取色放在设置里做一次，结果直接存进配置。
 */
async function applyThemeColors(): Promise<void> {
    if (!previewUrl.value) return
    try {
        const image = new Image()
        image.crossOrigin = 'anonymous'
        image.src = previewUrl.value
        await image.decode()
        const size = 24
        const canvas = document.createElement('canvas')
        canvas.width = size
        canvas.height = size
        const context = canvas.getContext('2d')
        if (!context) return
        context.drawImage(image, 0, 0, size, size)
        const { data } = context.getImageData(0, 0, size, size)
        let r = 0
        let g = 0
        let b = 0
        let count = 0
        for (let i = 0; i < data.length; i += 4) {
            const rr = data[i]
            const gg = data[i + 1]
            const bb = data[i + 2]
            const max = Math.max(rr, gg, bb)
            const min = Math.min(rr, gg, bb)
            // 跳过接近纯灰/纯黑的像素：它们对「主题色」没有信息量
            if (max - min < 18 || max < 40) continue
            r += rr
            g += gg
            b += bb
            count += 1
        }
        if (!count) return
        const toHex = (value: number) =>
            Math.max(0, Math.min(255, Math.round(value)))
                .toString(16)
                .padStart(2, '0')
        const base = [r / count, g / count, b / count]
        const lighten = base.map((value) => value + (255 - value) * 0.35)
        const accentA = `#${base.map(toHex).join('')}`
        const accentB = `#${lighten.map(toHex).join('')}`
        persistLayout({ accentA, accentB })
    } catch {
        // 取色失败不该挡住设置，保持现有颜色
    }
}

const stageGradient = computed(() => {
    const a = layout.value.accentA || '#7aa2ff'
    const b = layout.value.accentB || '#9fd8ff'
    return `linear-gradient(90deg, ${a}, ${b})`
})

const stageAccent = computed(() => layout.value.accentB || '#9fd8ff')

const previewCaption = computed(
    () => config.value.tagline.trim() || formatMessage(messages.taglineHint),
)

async function pickMedia(): Promise<void> {
    const selected = await open({
        multiple: false,
        filters: [
            {
                name: 'Media',
                extensions: [
                    'png',
                    'jpg',
                    'jpeg',
                    'webp',
                    'gif',
                    'bmp',
                    'avif',
                    'mp4',
                    'webm',
                    'mov',
                    'm4v',
                ],
            },
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

async function pickMediaAndRefresh(): Promise<void> {
    await pickMedia()
    await refreshPreview()
}

async function clearMediaAndRefresh(): Promise<void> {
    await clearMedia()
    previewUrl.value = null
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
                        {{ formatMessage(config.media ? messages.replace : messages.pick) }}
                    </Button>
                    <Button
                        v-if="config.media"
                        type="quiet"
                        color="red"
                        :disabled="busy"
                        @click="clearMedia"
                    >
                        <TrashIcon />{{ formatMessage(messages.clear) }}
                    </Button>
                </div>
            </div>
            <p class="m-0 text-xs leading-relaxed text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.hint) }}
            </p>
        </div>

        <div class="mt-2 flex flex-col gap-3">
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

        <div class="mt-2 flex flex-col gap-3">
            <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                    <p class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                        {{ formatMessage(messages.animation) }}
                    </p>
                    <p class="m-0 mt-0.5 text-xs text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.animationHint) }}
                    </p>
                </div>
                <Button type="base" :disabled="busy" @click="openAdjust">
                    {{ formatMessage(messages.customAdjust) }}
                </Button>
            </div>

            <label class="splash-row">
                <span>{{ formatMessage(messages.style) }}</span>
                <OverflowMenu :options="styleMenu()" placement="bottom-end">
                    {{ styleLabel }} ▾
                </OverflowMenu>
            </label>
        </div>

        <NewModal
            ref="adjustModal"
            :header="formatMessage(messages.adjustTitle)"
            :closable="true"
            max-width="54rem"
            :on-hide="stopPreviewProgress"
        >
            <div class="splash-adjust">
                <aside class="splash-adjust__panel">
                    <div class="splash-adjust__field">
                        <span class="splash-adjust__label">{{
                            formatMessage(messages.media)
                        }}</span>
                        <span class="splash-adjust__value">{{ mediaLabel }}</span>
                        <div class="flex gap-2">
                            <Button type="base" :disabled="busy" @click="pickMediaAndRefresh">
                                {{ formatMessage(messages.pick) }}
                            </Button>
                            <Button
                                v-if="config.media"
                                type="quiet"
                                color="red"
                                :disabled="busy"
                                @click="clearMediaAndRefresh"
                            >
                                {{ formatMessage(messages.clear) }}
                            </Button>
                        </div>
                    </div>

                    <div class="splash-adjust__field">
                        <span class="splash-adjust__label">{{
                            formatMessage(messages.pickWallpaper)
                        }}</span>
                        <Combobox
                            v-model="wallpaperSelection"
                            :options="wallpaperOptions"
                            :disabled="wallpapersLoading"
                            :placeholder="formatMessage(messages.pickWallpaper)"
                            :no-options-message="formatMessage(messages.noWallpapers)"
                            show-no-options-when-empty
                        />
                        <span class="splash-adjust__value">{{
                            formatMessage(messages.pickWallpaperHint)
                        }}</span>
                    </div>

                    <div class="splash-adjust__field">
                        <span class="splash-adjust__label">{{
                            formatMessage(messages.tagline)
                        }}</span>
                        <StyledInput
                            :model-value="config.tagline"
                            :placeholder="formatMessage(messages.taglineHint)"
                            wrapper-class="w-full"
                            @update:model-value="(value) => persist({ tagline: String(value) })"
                        />
                    </div>

                    <label class="splash-adjust__row">
                        <span>{{ formatMessage(messages.scrim) }}</span>
                        <input
                            type="range"
                            min="0"
                            max="1"
                            step="0.05"
                            :value="layout.scrim"
                            @input="setScrim($event)"
                            @change="persistLayout({ scrim: layout.scrim })"
                        />
                        <b>{{ Math.round(layout.scrim * 100) }}%</b>
                    </label>

                    <label class="splash-adjust__row">
                        <span>{{ formatMessage(messages.blur) }}</span>
                        <input
                            type="range"
                            min="0"
                            max="40"
                            step="1"
                            :value="layout.blur"
                            @input="setBlur($event)"
                            @change="persistLayout({ blur: layout.blur })"
                        />
                        <b>{{ Math.round(layout.blur) }}px</b>
                    </label>

                    <div class="splash-adjust__row">
                        <span>{{ formatMessage(messages.progressStyle) }}</span>
                        <OverflowMenu :options="progressMenu()" placement="bottom-end">
                            {{ progressLabel }} ▾
                        </OverflowMenu>
                        <b />
                    </div>

                    <div class="splash-adjust__field">
                        <Button
                            type="base"
                            :disabled="busy || !previewUrl"
                            @click="applyThemeColors"
                        >
                            {{ formatMessage(messages.themeColors) }}
                        </Button>
                        <span class="splash-adjust__value">{{
                            formatMessage(messages.themeColorsHint)
                        }}</span>
                    </div>

                    <Button
                        type="quiet"
                        :disabled="busy"
                        @click="persistLayout({ ...DEFAULT_LAYOUT })"
                    >
                        {{ formatMessage(messages.resetLayout) }}
                    </Button>
                </aside>

                <div class="splash-adjust__preview">
                    <div
                        ref="stageEl"
                        class="splash-stage"
                        @pointermove="onPointerMove"
                        @pointerup="onPointerUp"
                        @pointercancel="onPointerUp"
                        @pointerleave="onPointerUp"
                    >
                        <img
                            v-if="previewUrl && config.media?.kind === 'image'"
                            :src="previewUrl"
                            alt=""
                            class="splash-stage__media"
                            :style="stageMediaStyle"
                        />
                        <video
                            v-else-if="previewUrl"
                            :src="previewUrl"
                            class="splash-stage__media"
                            :style="stageMediaStyle"
                            autoplay
                            muted
                            loop
                            playsinline
                        />
                        <div v-else class="splash-stage__fallback" />
                        <div
                            class="splash-stage__scrim"
                            :style="{ opacity: String(layout.scrim) }"
                        />

                        <p
                            class="splash-stage__tagline"
                            :style="{
                                left: `${layout.taglineX * 100}%`,
                                top: `${layout.taglineY * 100}%`,
                            }"
                            @pointerdown="onPointerDown('tagline', $event)"
                        >
                            {{ previewCaption }}
                        </p>

                        <div
                            class="splash-stage__progress"
                            :style="{
                                left: `${layout.progressX * 100}%`,
                                top: `${layout.progressY * 100}%`,
                            }"
                            @pointerdown="onPointerDown('progress', $event)"
                        >
                            <div v-if="layout.progressStyle === 'bar'" class="splash-stage__track">
                                <div
                                    class="splash-stage__bar"
                                    :style="{ width: `${previewProgress}%` }"
                                />
                            </div>
                            <div
                                v-else-if="layout.progressStyle === 'dots'"
                                class="splash-stage__dots"
                            >
                                <span /><span /><span />
                            </div>
                            <svg
                                v-else-if="layout.progressStyle === 'ring'"
                                class="splash-stage__ring"
                                viewBox="0 0 44 44"
                            >
                                <circle cx="22" cy="22" r="18" />
                                <circle
                                    class="splash-stage__ring-fill"
                                    cx="22"
                                    cy="22"
                                    r="18"
                                    :style="ringOffset"
                                />
                            </svg>
                        </div>
                    </div>
                    <p class="splash-adjust__hint">{{ formatMessage(messages.adjustHint) }}</p>
                </div>
            </div>
        </NewModal>
    </div>
</template>

<style scoped>
/* ---- 自定义调整弹窗 ---- */
.splash-adjust {
    display: grid;
    grid-template-columns: 17rem minmax(0, 1fr);
    gap: 18px;
    align-items: start;
}

@media (max-width: 880px) {
    .splash-adjust {
        grid-template-columns: minmax(0, 1fr);
    }
}

.splash-adjust__panel {
    display: flex;
    flex-direction: column;
    gap: 14px;
}

.splash-adjust__field {
    display: flex;
    flex-direction: column;
    gap: 6px;
}

.splash-adjust__label {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-primary);
}

.splash-adjust__value {
    font-size: 11px;
    color: var(--color-text-tertiary);
    word-break: break-all;
}

.splash-adjust__row {
    display: grid;
    grid-template-columns: 76px minmax(0, 1fr) 48px;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-1);
}

.splash-adjust__row > input[type='range'] {
    width: 100%;
    accent-color: var(--c-a);
    cursor: pointer;
}

.splash-adjust__hint {
    margin: 8px 0 0;
    font-size: 11px;
    color: var(--color-text-tertiary);
}

/* 预览台：16:10，模拟启动画面 */
.splash-stage {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 10;
    overflow: hidden;
    border-radius: var(--radius-lg);
    border: 1px solid var(--stroke-1);
    background: #14141a;
    touch-action: none;
}

.splash-stage__media {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
}

.splash-stage__fallback {
    position: absolute;
    inset: 0;
    background:
        radial-gradient(120% 90% at 50% 40%, rgb(122 162 255 / 22%) 0%, transparent 62%),
        radial-gradient(100% 80% at 50% 100%, rgb(159 216 255 / 14%) 0%, transparent 70%), #14141a;
}

.splash-stage__scrim {
    position: absolute;
    inset: 0;
    background:
        radial-gradient(ellipse at center, rgb(0 0 0 / 28%) 0%, rgb(0 0 0 / 62%) 100%),
        linear-gradient(180deg, rgb(0 0 0 / 30%) 0%, rgb(0 0 0 / 45%) 100%);
    pointer-events: none;
}

.splash-stage__tagline {
    position: absolute;
    transform: translate(-50%, -50%);
    margin: 0;
    max-width: 80%;
    font-size: 12px;
    color: #e8ecf6;
    text-align: center;
    cursor: grab;
    padding: 2px 6px;
    border-radius: var(--radius-xs);
    outline: 1px dashed rgb(255 255 255 / 28%);
    user-select: none;
}

.splash-stage__tagline:active,
.splash-stage__progress:active {
    cursor: grabbing;
}

.splash-stage__progress {
    position: absolute;
    transform: translate(-50%, -50%);
    cursor: grab;
    padding: 6px;
    border-radius: var(--radius-xs);
    outline: 1px dashed rgb(255 255 255 / 28%);
    user-select: none;
}

.splash-stage__track {
    width: 12rem;
    height: 2px;
    overflow: hidden;
    border-radius: 999px;
    background: rgb(255 255 255 / 10%);
}

.splash-stage__bar {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #7aa2ff, #9fd8ff);
}

.splash-stage__bar--stripes {
    background-size:
        24px 100%,
        100% 100%;
    animation: splash-stage-stripes 0.8s linear infinite;
}

@keyframes splash-stage-stripes {
    from {
        background-position: 0 0;
    }
    to {
        background-position: 24px 0;
    }
}

.splash-stage__bar--pulse {
    animation: splash-stage-pulse 1.4s ease-in-out infinite;
}

@keyframes splash-stage-pulse {
    0%,
    100% {
        opacity: 0.55;
    }
    50% {
        opacity: 1;
    }
}

.splash-stage__wave svg {
    width: 12rem;
    height: 20px;
}

.splash-stage__wave path {
    fill: none;
    stroke-width: 2.5;
    stroke-linecap: round;
    stroke-dasharray: 90 260;
    animation: splash-stage-wave 1.6s linear infinite;
}

@keyframes splash-stage-wave {
    from {
        stroke-dashoffset: 0;
    }
    to {
        stroke-dashoffset: -350;
    }
}

.splash-stage__dots {
    display: flex;
    gap: 6px;
}

.splash-stage__dots span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #9fd8ff;
    animation: splash-preview-dot 1.1s ease-in-out infinite;
}

.splash-stage__dots span:nth-child(2) {
    animation-delay: 0.15s;
}

.splash-stage__dots span:nth-child(3) {
    animation-delay: 0.3s;
}

@keyframes splash-preview-dot {
    0%,
    100% {
        opacity: 0.25;
        transform: translateY(0);
    }
    50% {
        opacity: 1;
        transform: translateY(-3px);
    }
}

.splash-stage__ring {
    width: 40px;
    height: 40px;
    transform: rotate(-90deg);
}

.splash-stage__ring circle {
    fill: none;
    stroke: rgb(255 255 255 / 16%);
    stroke-width: 3;
}

.splash-stage__ring .splash-stage__ring-fill {
    stroke: #9fd8ff;
    stroke-linecap: round;
    stroke-dasharray: 113;
}

.splash-row {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-1);
}
</style>
