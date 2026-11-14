<script setup lang="ts">
/**
 * Cold-start screen: particle field, rounded app icon, progress line.
 *
 * The component does not remove itself. It reports `ready` once the minimum
 * display time has elapsed and the loading state is idle; `App.vue` then runs the
 * pixel-grid reveal, which is the only thing that unmounts this canvas. That
 * keeps the animation loop's lifetime tied to a single owner.
 */
import { defineMessages, injectLoadingState, useVIntl } from '@modrinth/ui'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import {
    computed,
    defineAsyncComponent,
    nextTick,
    onBeforeUnmount,
    onMounted,
    ref,
    watch,
} from 'vue'

import appIcon from '@/assets/hayenai.png'
import { prefersReduced } from '@/components/ui/splash/aeroCore'
import ParticlesBackground from '@/components/ui/splash/ParticlesBackground.vue'
import { loading_listener } from '@/helpers/events.js'
import { wallpaperBaseUrl } from '@/helpers/wallpaper'

const emit = defineEmits<{ ready: []; 'gpu-ready': [] }>()

/**
 * Loaded lazily: the WebGPU renderer plus its shader module is a few hundred
 * kilobytes, and the splash must not delay the first paint with it. The static
 * gradient below is painted immediately, so a slow or failed chunk load shows a
 * background instead of a blank window.
 */
const AeroShards = defineAsyncComponent({
    loader: () => import('@/components/ui/splash/AeroShards.vue'),
    // A missing chunk is just another reason the GPU path is unavailable; the
    // splash keeps running on the 2D particles instead of showing nothing.
    onError: (error, _retry, fail) => {
        degrade(error instanceof Error ? error : new Error(String(error)))
        fail()
    },
})

/** Never flash the screen for less than this, even on a warm start. */
const MIN_DISPLAY_MS = 1000
/**
 * 就绪的兜底上限。
 *
 * `emitReady()` 依赖 `loading.pending` 变 false；只要有一个下载/安装任务卡在队列里
 * 永远不收尾，`pending` 就一直是 true ⇒ **启动画面永远不换场，启动器看起来是死了**
 * （实机复现：冷启动后只剩壁纸 + 磁悬条，点哪里都没反应）。
 * 这里给个硬上限，到点强制放行并留一条 warn。
 */
const HARD_READY_MS = 8000

/**
 * Background renderer for the splash.
 *
 * `gpu` is the intended path (AeroShards, WebGPU). It falls back to `particles`
 * (WebGL2) and then to `static`, and each step only ever happens once: a
 * recovered GPU must not yank the background back mid-splash, which would read
 * as a flicker.
 */
type SplashRenderer = 'gpu' | 'particles' | 'static'

const progress = ref(0)
const message = ref<string>()
const mountedAt = Date.now()
let readyEmitted = false

/** Start optimistic: AeroShards reports back if WebGPU is unusable. */
const renderer = ref<SplashRenderer>(
    prefersReduced()
        ? 'static'
        : typeof navigator !== 'undefined' && 'gpu' in navigator
          ? 'gpu'
          : 'particles',
)

/**
 * QA switch: `?splash=fallback` forces the particles path and `?splash=static`
 * the gradient, so the degradation chain can be checked without unplugging a GPU
 * or editing code.
 */
const forcedRenderer = (() => {
    if (typeof location === 'undefined') return null
    const value = new URLSearchParams(location.search).get('splash')
    if (value === 'fallback' || value === 'particles') return 'particles' as const
    if (value === 'static') return 'static' as const
    return null
})()
if (forcedRenderer) renderer.value = forcedRenderer

const showGpu = computed(() => renderer.value === 'gpu')

/**
 * `gpu-ready` fires exactly once, from whichever path puts pixels on screen
 * first:
 *
 * * AeroShards' `ready` (device up, first frame scheduled);
 * * `degrade()` — a fallback renderer is about to take over, so the parent must
 *   not wait for a GPU that will never answer;
 * * component mount — when the very first decision is `static` (reduced motion
 *   or `?splash=static`), nothing else will ever emit.
 */
let gpuReadyEmitted = false
function emitGpuReady() {
    if (gpuReadyEmitted) return
    gpuReadyEmitted = true
    emit('gpu-ready')
}

/** Single, one-way degradation: gpu → particles → static. */
function degrade(error?: Error) {
    if (error) console.warn('[splash] falling back from WebGPU:', error.message)
    if (renderer.value === 'gpu') {
        renderer.value = 'particles'
    } else if (renderer.value === 'particles') {
        renderer.value = 'static'
    }
    // The fallback is the thing that will paint; release the gate now rather
    // than waiting for a ready signal the failed renderer will never send.
    emitGpuReady()
}

/**
 * 自定义启动动画。
 *
 * 刻意**不等** State：启动画面要赶在数据库初始化之前出图，所以配置走
 * `splash/config.json` 而不是设置表。读取失败一律当成「没自定义」，退回内置粒子背景 ——
 * 启动画面永远不能因为配置读不到就白屏。
 */
interface SplashAnimationConfig {
    style: 'auto' | 'shards' | 'particles' | 'static'
    count: number
    speed: number
    glow: number
    shardColor: string
    accentColor: string
    mouseInteraction: boolean
}

const DEFAULT_ANIMATION: SplashAnimationConfig = {
    style: 'auto',
    count: 900,
    speed: 0.8,
    glow: 0.35,
    shardColor: '#7aa2ff',
    accentColor: '#9fd8ff',
    mouseInteraction: true,
}

interface SplashLayoutConfig {
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

const DEFAULT_LAYOUT: SplashLayoutConfig = {
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

const splashConfig = ref({
    media: null as null | { kind: 'image' | 'video' | 'scene'; file: string },
    showIcon: true,
    showTagline: true,
    showProgress: true,
    tagline: '',
    animation: { ...DEFAULT_ANIMATION },
    layout: { ...DEFAULT_LAYOUT },
})

const layout = computed<SplashLayoutConfig>(() => ({
    ...DEFAULT_LAYOUT,
    ...(splashConfig.value.layout ?? {}),
}))

/** 压暗层透明度：0 时整层不画，壁纸原色。 */
const scrimStyle = computed(() => ({ opacity: String(layout.value.scrim) }))

/** 背景模糊：放大 1.06 倍，避免模糊后露出边缘透明带。 */
const mediaStyle = computed(() =>
    layout.value.blur > 0
        ? { filter: `blur(${layout.value.blur}px)`, transform: 'scale(1.06)' }
        : {},
)

/** 进度条渐变：优先用配置里的主题色，空着就回退内置那对蓝。 */
const progressGradient = computed(() => {
    const a = layout.value.accentA || '#7aa2ff'
    const b = layout.value.accentB || '#9fd8ff'
    return `linear-gradient(90deg, ${a}, ${b})`
})

const accentSolid = computed(() => layout.value.accentB || '#9fd8ff')

const taglineStyle = computed(() => ({
    left: `${layout.value.taglineX * 100}%`,
    top: `${layout.value.taglineY * 100}%`,
}))

const progressBoxStyle = computed(() => ({
    left: `${layout.value.progressX * 100}%`,
    top: `${layout.value.progressY * 100}%`,
}))

/** 用户在设置里选了固定风格时，压过「按 WebGPU 可用性自动挑」。 */
const animation = computed<SplashAnimationConfig>(() => ({
    ...DEFAULT_ANIMATION,
    ...(splashConfig.value.animation ?? {}),
}))

// 注意：上面还有一个 `forcedRenderer`（`?splash=` QA 开关），同名会直接语法冲突。
const configRenderer = computed<SplashRenderer | null>(() => {
    const style = animation.value.style
    if (style === 'auto') return null
    return style === 'shards' ? 'gpu' : style === 'particles' ? 'particles' : 'static'
})

watch(configRenderer, (forced) => {
    if (!forced) return
    renderer.value = forced
    // 静态风格永远不会自己报 ready，这里直接放行；另两种交给各自的 ready 事件。
    if (forced === 'static') emitGpuReady()
})
const mediaUrl = ref<string | null>(null)
const mediaBroken = ref(false)
/** `kind === 'scene'` 时存壁纸 id。 */
const sceneId = ref<string | null>(null)
const sceneEl = ref<HTMLElement | null>(null)
let sceneInstance: { destroy?: () => void } | null = null

/**
 * 在启动画面里**真跑** webwallgl 渲染。
 *
 * 和之前「抓一帧 PNG 当背景」的区别：这里是实时的，粒子/动画/时钟都会动。
 * 之所以敢在启动阶段做：壁纸包走 `hayenai-we://` 自定义协议，那个协议在
 * Tauri 注册时就已经生效，比数据库初始化还早，不需要等任何服务。
 */
async function mountScene(id: string): Promise<void> {
    const element = sceneEl.value
    if (!element) return
    const startedAt = performance.now()
    try {
        const { mount, httpSource } = await import('webwallgl')
        const instance = await mount(element, {
            source: httpSource(wallpaperBaseUrl(id)),
            // 启动画面不是观看场景，30fps 足够，也让首帧来得更快。
            fps: 30,
            mountTimeoutMs: 8000,
        })
        sceneInstance = instance as unknown as { destroy?: () => void }
        console.info(`[splash] 场景首帧耗时 ${Math.round(performance.now() - startedAt)} ms`)
        emitGpuReady()
    } catch (error) {
        console.warn('[splash] 场景启动渲染失败，回退内置动画', error)
        // 回退：把 customMedia 置空 → 模板换回内置动画，同时放行闸门。
        mediaBroken.value = true
        emitGpuReady()
    }
}

function destroyScene(): void {
    try {
        sceneInstance?.destroy?.()
    } catch {
        // 启动画面换场时实例可能已经被销毁，忽略。
    }
    sceneInstance = null
}

/**
 * 自定义背景加载失败。
 *
 * ★ 这里必须同时放行闸门：`customMedia` 变 null 后模板会退回内置动画，
 * 但那一刻父组件的 `gpuReady` 还是 false —— 只置 mediaBroken 就会让启动画面
 * 永远挂着（实测：设了自定义壁纸后冷启动卡在启动画面出不来）。
 */
function onMediaError(): void {
    mediaBroken.value = true
    emitGpuReady()
}

const customMedia = computed(() => {
    const media = splashConfig.value.media
    if (!media || mediaBroken.value) return null
    // 场景型没有文件路径，只有壁纸 id —— 由 webwallgl 直接读包。
    if (media.kind === 'scene') {
        return sceneId.value ? { kind: 'scene' as const, url: '', id: sceneId.value } : null
    }
    if (!mediaUrl.value) return null
    return { kind: media.kind, url: mediaUrl.value, id: '' }
})

async function loadSplashConfig() {
    try {
        const config = await invoke<typeof splashConfig.value>('splash_get_config')
        splashConfig.value = { ...splashConfig.value, ...config }
        if (config.media) {
            if (config.media.kind === 'scene') {
                // 场景型不需要文件路径，记下 id 就行。
                sceneId.value = config.media.file
            } else {
                const path = await invoke<string | null>('splash_media_path')
                if (path) mediaUrl.value = convertFileSrc(path)
            }
        }
    } catch (error) {
        // 没拿到配置就用默认值 —— 这是「启动画面」,不值得为它报错打扰用户。
        console.warn('[splash] 读取自定义配置失败，使用内置背景', error)
    }
}

const loading = injectLoadingState()
const { formatMessage } = useVIntl()

const messages = defineMessages({
    updatingAppDirectory: {
        id: 'app.splash.updating-app-directory',
        defaultMessage: 'Updating app directory...',
    },
    checkingForUpdates: {
        id: 'app.splash.checking-for-updates',
        defaultMessage: 'Checking for updates...',
    },
    preparing: {
        id: 'app.splash.preparing',
        defaultMessage: 'Preparing your launcher...',
    },
})

/** Emits `ready` once, when both the timer and the loader allow it. */
function emitReady() {
    if (readyEmitted) return
    readyEmitted = true
    loading.setEnabled(true)
    emit('ready')
}

function scheduleReady() {
    const elapsed = Date.now() - mountedAt
    const delay = Math.max(0, MIN_DISPLAY_MS - elapsed)
    window.setTimeout(() => {
        if (!loading.pending.value) emitReady()
    }, delay)
}

/** Creeping progress while the app has no real fraction to report. */
function creep() {
    if (progress.value >= 95 || readyEmitted) return
    window.setTimeout(() => {
        progress.value += 2
        creep()
    }, 24)
}

watch(
    [loading.barEnabled, loading.pending],
    ([barEnabled, pending]) => {
        if (barEnabled) return
        if (pending) {
            progress.value = 0
            creep()
            return
        }
        scheduleReady()
    },
    { immediate: true },
)

// 配置读回来后（sceneId 赋值）再挂场景 —— 元素要等 v-if 渲染出来。
watch(sceneId, async (id) => {
    if (!id) return
    await nextTick()
    await mountScene(id)
})

onMounted(() => {
    // 自定义素材要尽早拿到：它决定这一帧画什么。失败不影响下面的就绪流程。
    void loadSplashConfig()
    window.setTimeout(() => {
        if (readyEmitted) return
        console.warn('[splash] 加载状态长期未收尾，强制放行')
        emitReady()
    }, HARD_READY_MS)

    // 场景渲染可能因为包损坏/驱动问题一直不出第一帧：到点无论结果如何都让换场继续，
    // 否则就是「启动画面永远挂着」那种事故。emitGpuReady 自带幂等标志。
    //
    // ⚠ 场景包要从磁盘解包 + 建 GL 上下文，比图片慢得多（实测数秒）。
    //   给 4 秒会把还没挂完的场景直接掐断，用户永远看不到真渲染。所以场景单独放宽。
    const isScene = splashConfig.value.media?.kind === 'scene'
    window.setTimeout(() => emitGpuReady(), isScene ? 15000 : 4000)
    // A static background never reports ready — it is not animated. Everything
    // else reports from the component that actually paints.
    if (renderer.value === 'static') emitGpuReady()
})

onBeforeUnmount(() => {
    destroyScene()
    // The reveal covered us; nothing left to time out.
    readyEmitted = true
})

loading_listener((event) => {
    if (event.event.type === 'directory_move') {
        progress.value = 100 * (event.fraction ?? 1)
        message.value = formatMessage(messages.updatingAppDirectory)
    } else if (event.event.type === 'checking_for_updates') {
        progress.value = 100 * (event.fraction ?? 1)
        message.value = formatMessage(messages.checkingForUpdates)
    }
})
</script>

<template>
    <div class="splash" data-tauri-drag-region>
        <!--
			自定义素材优先于内置背景。两者是互斥的：用户选了视频就是想看视频，
			不该再让粒子在上面叠一层。
		-->
        <template v-if="customMedia">
            <!-- 场景型：真渲染。容器必须有实际尺寸，库靠 ResizeObserver 装配。 -->
            <div
                v-if="customMedia.kind === 'scene'"
                ref="sceneEl"
                class="splash__media splash__media--scene"
            />
            <video
                v-else-if="customMedia.kind === 'video'"
                class="splash__media"
                :src="customMedia.url"
                :style="mediaStyle"
                autoplay
                muted
                loop
                playsinline
                @loadeddata="emitGpuReady"
                @error="onMediaError"
            />
            <img
                v-else
                class="splash__media"
                :src="customMedia.url"
                :style="mediaStyle"
                alt=""
                draggable="false"
                @load="emitGpuReady"
                @error="onMediaError"
            />
            <!-- 压暗层：默认 0（主人要求别压暗），要压暗在「自定义调整」里拉 -->
            <div class="splash__scrim" :style="scrimStyle" />
        </template>
        <template v-else>
            <AeroShards
                v-if="showGpu"
                :shard-color="animation.shardColor"
                :accent-color="animation.accentColor"
                :count="animation.count"
                :speed="animation.speed"
                :glow="animation.glow"
                :depth-of-field="0.75"
                markers="plus"
                :mouse-interaction="animation.mouseInteraction"
                :pulse-on-click="animation.mouseInteraction"
                :hold-to-gather="animation.mouseInteraction"
                :parallax="0.5"
                :dpr="2"
                @ready="emitGpuReady"
                @error="degrade"
            />
            <ParticlesBackground
                v-else-if="renderer === 'particles'"
                :particle-count="260"
                :particle-colors="[animation.shardColor, animation.accentColor, '#ffffff']"
                :glow="0.6"
                :twinkle="0.35"
                drift="forward"
                :connections="true"
                hover-effect="parallax"
                :speed="0.08"
                @ready="emitGpuReady"
                @unsupported="degrade()"
            />
        </template>

        <div class="splash__content" data-tauri-drag-region>
            <img
                v-if="splashConfig.showIcon"
                :src="appIcon"
                alt="HaYenai"
                class="splash__icon"
                draggable="false"
                data-tauri-drag-region
            />
        </div>

        <p
            v-if="splashConfig.showTagline"
            class="splash__caption splash__placed"
            :style="taglineStyle"
            data-tauri-drag-region
        >
            {{ splashConfig.tagline.trim() || message || formatMessage(messages.preparing) }}
        </p>

        <div
            v-if="splashConfig.showProgress && layout.progressStyle !== 'none'"
            class="splash__progress splash__placed"
            :style="progressBoxStyle"
            role="progressbar"
            :aria-valuenow="Math.round(progress)"
        >
            <div v-if="layout.progressStyle === 'bar'" class="splash__track">
                <div
                    class="splash__bar"
                    :style="{ width: `${Math.min(100, progress)}%`, background: progressGradient }"
                />
            </div>
            <div v-else-if="layout.progressStyle === 'stripes'" class="splash__track">
                <div
                    class="splash__bar splash__bar--stripes"
                    :style="{
                        width: `${Math.min(100, progress)}%`,
                        backgroundImage: progressGradient,
                    }"
                />
            </div>
            <div v-else-if="layout.progressStyle === 'pulse'" class="splash__track">
                <div
                    class="splash__bar splash__bar--pulse"
                    :style="{ width: `${Math.min(100, progress)}%`, background: progressGradient }"
                />
            </div>
            <svg
                v-else-if="layout.progressStyle === 'wave'"
                class="splash__wave"
                viewBox="0 0 160 24"
                preserveAspectRatio="none"
                aria-hidden="true"
            >
                <defs>
                    <linearGradient id="splash-wave-grad" x1="0" y1="0" x2="1" y2="0">
                        <stop offset="0%" :stop-color="accentSolid" />
                        <stop offset="100%" :stop-color="accentSolid" stop-opacity="0.1" />
                    </linearGradient>
                </defs>
                <path
                    class="splash__wave-path"
                    :style="{ stroke: 'url(#splash-wave-grad)' }"
                    d="M0 12 Q 20 2, 40 12 T 80 12 T 120 12 T 160 12"
                />
            </svg>
            <div v-else-if="layout.progressStyle === 'dots'" class="splash__dots">
                <span :style="{ background: accentSolid }" />
                <span :style="{ background: accentSolid }" />
                <span :style="{ background: accentSolid }" />
            </div>
            <svg v-else class="splash__ring" viewBox="0 0 44 44" aria-hidden="true">
                <circle cx="22" cy="22" r="18" />
                <circle
                    class="splash__ring-fill"
                    cx="22"
                    cy="22"
                    r="18"
                    :style="{
                        stroke: accentSolid,
                        strokeDashoffset: `${113 - (113 * Math.min(100, progress)) / 100}`,
                    }"
                />
            </svg>
        </div>
    </div>
</template>

<style scoped>
.splash {
    position: fixed;
    inset: 0;
    z-index: 10001;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    /* Static gradient doubles as the fallback when WebGL2 is unavailable. */
    background:
        radial-gradient(120% 90% at 50% 40%, rgb(122 162 255 / 22%) 0%, transparent 62%),
        radial-gradient(100% 80% at 50% 100%, rgb(159 216 255 / 14%) 0%, transparent 70%),
        var(--surface-1, #14141a);
}

.splash__content {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 0 1.5rem;
}

/* 自定义素材：铺满整屏，超出部分裁掉（cover）。
   不用 `contain` —— 启动画面留黑边比裁掉一点更难接受。 */
.splash__media {
    position: absolute;
    inset: 0;
    z-index: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
}

/* 压暗层：中间略亮、四周更暗，让图标与文字在任何素材上都能读出来。 */
/* 场景容器：铺满整屏，库会把 canvas 塞进这个 div */
.splash__media--scene {
    inset: 0;
    width: 100%;
    height: 100%;
}

.splash__scrim {
    position: absolute;
    inset: 0;
    z-index: 0;
    background:
        radial-gradient(ellipse at center, rgb(0 0 0 / 28%) 0%, rgb(0 0 0 / 62%) 100%),
        linear-gradient(180deg, rgb(0 0 0 / 30%) 0%, rgb(0 0 0 / 45%) 100%);
    pointer-events: none;
}

.splash__icon {
    width: min(9rem, 30vh);
    height: min(9rem, 30vh);
    object-fit: contain;
    border-radius: 22%;
    /* A soft halo instead of a drop shadow keeps the icon flat, WE-style. */
    filter: drop-shadow(0 0 3rem rgb(122 162 255 / 35%));
    user-select: none;
    -webkit-user-drag: none;
    animation: splash-icon-in 520ms cubic-bezier(0.22, 1, 0.36, 1) both;
}

.splash__caption {
    margin: 0;
    font-size: 0.8125rem;
    letter-spacing: 0.02em;
    color: var(--color-text-tertiary, #9aa3b2);
}

/* 可定位的浮层：坐标来自「自定义调整」，默认落在下方居中 */
.splash__placed {
    position: absolute;
    z-index: 2;
    transform: translate(-50%, -50%);
    pointer-events: none;
}

.splash__track {
    width: min(14rem, 60vw);
    height: 2px;
    overflow: hidden;
    border-radius: 999px;
    background: rgb(255 255 255 / 10%);
}

.splash__bar {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #7aa2ff, #9fd8ff);
    transition: width 180ms ease-out;
}

/* 进度条：斜纹流动 */
.splash__bar--stripes {
    background-size:
        24px 100%,
        100% 100%;
    background-blend-mode: overlay;
    animation: splash-stripes 0.8s linear infinite;
}

@keyframes splash-stripes {
    from {
        background-position: 0 0;
    }
    to {
        background-position: 24px 0;
    }
}

/* 进度条：呼吸 */
.splash__bar--pulse {
    animation: splash-pulse 1.4s ease-in-out infinite;
}

@keyframes splash-pulse {
    0%,
    100% {
        opacity: 0.55;
    }
    50% {
        opacity: 1;
    }
}

/* 进度条：波浪 */
.splash__wave {
    width: min(14rem, 60vw);
    height: 20px;
    overflow: visible;
}

.splash__wave-path {
    fill: none;
    stroke-width: 2.5;
    stroke-linecap: round;
    stroke-dasharray: 90 260;
    animation: splash-wave 1.6s linear infinite;
}

@keyframes splash-wave {
    from {
        stroke-dashoffset: 0;
    }
    to {
        stroke-dashoffset: -350;
    }
}

/* 进度条：点状 */
.splash__dots {
    display: flex;
    gap: 6px;
}

.splash__dots span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #9fd8ff;
    animation: splash-dot 1.1s ease-in-out infinite;
}

.splash__dots span:nth-child(2) {
    animation-delay: 0.15s;
}

.splash__dots span:nth-child(3) {
    animation-delay: 0.3s;
}

@keyframes splash-dot {
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

/* 进度条：环形 */
.splash__ring {
    width: 44px;
    height: 44px;
    transform: rotate(-90deg);
}

.splash__ring circle {
    fill: none;
    stroke: rgb(255 255 255 / 16%);
    stroke-width: 3;
}

.splash__ring .splash__ring-fill {
    stroke: #9fd8ff;
    stroke-linecap: round;
    stroke-dasharray: 113;
    transition: stroke-dashoffset 180ms ease-out;
}

@media (prefers-reduced-motion: reduce) {
    .splash__dots span,
    .splash__bar--stripes,
    .splash__bar--pulse,
    .splash__wave-path {
        animation: none;
    }
}

@keyframes splash-icon-in {
    from {
        opacity: 0;
        transform: scale(0.96);
    }
    to {
        opacity: 1;
        transform: scale(1);
    }
}

@media (prefers-reduced-motion: reduce) {
    .splash__icon {
        animation: none;
    }

    .splash__bar {
        transition: none;
    }
}
</style>
