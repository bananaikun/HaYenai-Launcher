<script setup lang="ts">
/**
 * 背景层的统一入口：按 `project.json` 的 `type` 分派，并负责两条兜底链。
 *
 * **分派**：
 *
 * ```text
 * scene  → SceneWallpaper（webwallgl，WebGL2）
 * video  → VideoWallpaper（原生 <video>）
 * 其余    → 海报（第一期不做 web / application）
 * ```
 *
 * **兜底**（架构文档 §3.5：逐级降，不允许黑屏）：
 *
 * ```text
 * 包损坏 / 无 project.json      → 纯色
 * 类型不支持                     → 海报 + 一句提示
 * 无硬件 WebGL2                  → 海报 + 一句提示
 * ETC2/BPTC 贴图 + 本机无对应扩展 → 海报 + 一句提示
 * 引擎 mount/首帧失败            → 海报 + 一句提示
 * 连海报都没有                   → 纯色
 * prefers-reduced-motion         → 只显示海报（不启动任何动画）
 * ```
 *
 * 提示一律是**非阻断**的一行小字，不弹错误框：壁纸是装饰，它坏了不该打断用户。
 */
import { defineMessages, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import {
    DEFAULT_WALLPAPER_QUALITY,
    probeWallpaperGpu,
    wallpaperAssetUrl,
    wallpaperRenderability,
    type WallpaperGpuCaps,
    type WallpaperMeta,
    type WallpaperQuality,
} from '@/helpers/wallpaper'

import { wallpaperState } from '@/helpers/wallpaper'
import SceneWallpaper from './SceneWallpaper.vue'
import VideoWallpaper from './VideoWallpaper.vue'
import WebWallpaper from './WebWallpaper.vue'

const props = withDefaults(
    defineProps<{
        /** 要画的壁纸。`null` 时本层只留一块纯色底。 */
        meta?: WallpaperMeta | null
        /** 渲染档位，来自 `wallpapers/config.json`。 */
        quality?: WallpaperQuality
        /** 海报地址（包内 `preview` 的取流地址）。 */
        posterUrl?: string | null
        /**
         * 宿主是否希望它跑着。窗口最小化、被不透明遮罩盖住时传 `false`。
         *
         * 「路由切换」不需要额外的暂停：本层挂在 `App.vue` 的根上，位于路由
         * 视图**下面**，切页面不会卸载它 —— 需要暂停的是「被遮住」，那是
         * `suspended` 的语义，不是「换了路由」。
         */
        active?: boolean
    }>(),
    {
        meta: null,
        quality: () => ({ ...DEFAULT_WALLPAPER_QUALITY }),
        posterUrl: null,
        active: true,
    },
)

const { formatMessage } = useVIntl()

const messages = defineMessages({
    noticeUnsupported: {
        id: 'app.settings.wallpaper.notice-unsupported',
        defaultMessage:
            'This wallpaper type is not supported yet, so its preview image is shown instead.',
    },
    noticeNoGpu: {
        id: 'app.settings.wallpaper.notice-no-gpu',
        defaultMessage:
            'No hardware WebGL2 on this device, so the preview image is shown instead of the animation.',
    },
    noticeEtc2: {
        id: 'app.settings.wallpaper.notice-etc2',
        defaultMessage:
            'This package uses texture formats this device cannot decode, so the preview image is shown instead.',
    },
    noticeFailed: {
        id: 'app.settings.wallpaper.notice-failed',
        defaultMessage: 'This wallpaper could not be rendered, so its preview image is shown.',
    },
    noticeDamaged: {
        id: 'app.settings.wallpaper.notice-damaged',
        defaultMessage: 'This package is damaged, so only its preview image is available.',
    },
    noticeReducedMotion: {
        id: 'app.settings.wallpaper.notice-reduced-motion',
        defaultMessage: 'Reduced motion is enabled, so only a still preview is shown.',
    },
})

/**
 * 探一次设备能力就够，整个会话不变。
 *
 * 在 setup 里同步探（而不是等 `onMounted`）：`mode` 必须在**首帧渲染之前**就
 * 定下来，否则没有 GPU 的机器会先建一个 WebGL2 上下文、失败、再退回海报 ——
 * 而「无 GPU 就不建上下文」是硬指标（架构文档 §4.1 第 10 条）。这一层只有真的
 * 选中了壁纸才会挂载，所以这次探测不会发生在启动路径上。
 */
const caps = ref<WallpaperGpuCaps>(probeWallpaperGpu())
/** `prefers-reduced-motion: reduce`。 */
const reducedMotion = ref(false)
/** `document.hidden`。初值同步取，免得后台启动时先白跑一帧。 */
const documentHidden = ref(typeof document !== 'undefined' && document.hidden)
/** 引擎交出首帧了。海报据此淡出。 */
const engineReady = ref(false)
/** 失败原因（i18n key 片段），`null` 表示还没有失败。 */
const failure = ref<string | null>(null)

let motionQuery: MediaQueryList | null = null

/** 当前能画到什么程度。 */
const renderability = computed(() =>
    props.meta ? wallpaperRenderability(props.meta, caps.value) : 'unsupported',
)

/** 实际渲染方式。 */
const mode = computed<'scene' | 'video' | 'web' | 'poster'>(() => {
    const meta = props.meta
    if (!meta || meta.damaged) return 'poster'
    // 减少动态效果：只留一张静态图。视频同样要停 —— 它动得比场景更明显。
    if (reducedMotion.value) return 'poster'
    if (failure.value) return 'poster'
    if (renderability.value !== 'ok') return 'poster'
    if (meta.type === 'video') return meta.file ? 'video' : 'poster'
    // 场景**不再要求 file**：真实库里大量条目声明 `file: "scene.json"`（编辑器工程形态），
    // 而盘上只有 scene.pkg；webwallgl 的 httpSource 会自己按
    // scene.pkg → scenes/scene.pkg → gifscene.pkg 回退。
    // 以前这里要求 file 存在，等于把这类包全部钉在 poster 上（就是「一直只显示缩略图」）。
    if (meta.type === 'scene') return 'scene'
    if (meta.type === 'web') return meta.file ? 'web' : 'poster'
    return 'poster'
})

/** 引擎该不该跑。`document.hidden` 与宿主给的 `active` 取与。 */
/**
 * 这个包当前生效的 WE 属性（用户在外观页改过的覆盖值）。
 * 库的 `mount({ properties })` 就是靠它决定组件颜色/开关/滑块 —— 之前完全没传，
 * 所以壁纸里的组件全是包内默认值，改都改不动。
 */
const sceneProperties = computed<Record<string, unknown>>(() => {
    const id = props.meta?.id
    if (!id) return {}
    return wallpaperState.config.properties[id] ?? {}
})

const engineActive = computed(() => props.active && !documentHidden.value)

/** 那行非阻断提示。没有问题时是 `null`。 */
const notice = computed<string | null>(() => {
    const meta = props.meta
    if (!meta) return null

    if (failure.value) return formatMessage(messages.noticeFailed)
    if (meta.damaged) return formatMessage(messages.noticeDamaged)
    if (reducedMotion.value) return formatMessage(messages.noticeReducedMotion)
    if (renderability.value === 'unsupported') return formatMessage(messages.noticeUnsupported)
    if (renderability.value === 'weak-gpu') {
        // 分清两种「不行」：没有硬件 WebGL2，还是贴图格式解不了。
        if (meta.type === 'scene' && !caps.value.webgl2) {
            return formatMessage(messages.noticeNoGpu)
        }
        return formatMessage(messages.noticeEtc2)
    }
    if (meta.type === 'video' && !meta.file) return formatMessage(messages.noticeDamaged)
    return null
})

/** 海报还能盖着屏幕吗。首帧到达（或已经证明画不出来）之后就没它的事了。 */
const posterVisible = computed(() => !!props.posterUrl && !engineReady.value)

/** `prefers-reduced-motion` 的订阅。 */
function bindMotionQuery(): void {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
    motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)')
    reducedMotion.value = motionQuery.matches
    motionQuery.addEventListener('change', onMotionChange)
}

function onMotionChange(event: MediaQueryListEvent): void {
    reducedMotion.value = event.matches
}

function onVisibilityChange(): void {
    documentHidden.value = document.hidden
}

/** 首帧到达：淡出海报。视频与场景共用一个信号。 */
function onFirstFrame(): void {
    engineReady.value = true
    failure.value = null
}

function onFail(reason: string): void {
    failure.value = reason
    engineReady.value = false
}

onMounted(() => {
    documentHidden.value = document.hidden
    document.addEventListener('visibilitychange', onVisibilityChange)
    bindMotionQuery()
})

onBeforeUnmount(() => {
    document.removeEventListener('visibilitychange', onVisibilityChange)
    motionQuery?.removeEventListener('change', onMotionChange)
    motionQuery = null
})

// 换包：一切从头开始，别把上一张的「已就绪 / 已失败」带过来。
watch(
    () => props.meta?.id,
    () => {
        engineReady.value = false
        failure.value = null
    },
)

defineExpose({
    /** 调试用：当前渲染方式（`scene` / `video` / `poster`）。 */
    mode,
    /** 调试用：设备能力探测结果。 */
    caps,
})

</script>

<template>
    <div class="wallpaper-layer" aria-hidden="true">
        <SceneWallpaper
            v-if="mode === 'scene' && meta"
            :key="`scene:${meta.id}`"
            :wallpaper-id="meta.id"
            :quality="props.quality"
            :active="engineActive"
            :properties="sceneProperties"
            @first-frame="onFirstFrame"
            @fail="onFail"
        />

        <WebWallpaper
            v-else-if="mode === 'web' && meta"
            :key="`web:${meta.id}`"
            :id="meta.id"
            :entry="meta.file"
            :resolve-url="wallpaperAssetUrl"
            :active="engineActive"
            @fail="onFail"
        />

        <VideoWallpaper
            v-else-if="mode === 'video' && meta"
            :key="`video:${meta.id}`"
            :wallpaper-id="meta.id"
            :file="meta.file"
            :active="engineActive"
            @first-frame="onFirstFrame"
            @fail="onFail"
        />

        <!--
			海报：与包同构图的一张静态图，顶住「引擎还在装配」的那一两秒。
			撤它不需要任何加载完成信号 —— 首帧到达即淡出。没有 preview 的包就只剩
			下面那层纯色，仍然不黑屏。
		-->
        <Transition name="wallpaper-poster">
            <img
                v-if="posterVisible"
                class="wallpaper-layer__poster"
                :src="posterUrl ?? undefined"
                alt=""
                decoding="async"
                draggable="false"
            />
        </Transition>

        <p v-if="notice" class="wallpaper-layer__notice">{{ notice }}</p>
    </div>
</template>

<style scoped>
.wallpaper-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    /* 包损坏 / 没有海报时的地板色。与 `.launcher-background` 同一个 token，
	   所以透明窗口或纯色模式下面看起来是同一块底。 */
    background-color: var(--surface-3-opaque);
}

.wallpaper-layer__poster {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
    /* 装配期的海报是「占位」，稍微压一点亮度，首帧出来后不会显得闪一下。 */
    filter: saturate(1.02);
}

.wallpaper-poster-leave-active {
    transition: opacity 320ms ease;
}

.wallpaper-poster-leave-to {
    opacity: 0;
}

.wallpaper-layer__notice {
    position: absolute;
    /*
	   `.launcher-background` 是 `inset: -3rem`（超出视口一圈，免得模糊/缩放时露边），
	   所以这里的 `left: 50%` 仍然落在视口正中，而 `bottom` 必须补回那 3rem
	   才不会跑到屏幕外面去。
	*/
    left: 50%;
    bottom: 3.75rem;
    transform: translateX(-50%);
    max-width: min(32rem, calc(100% - 2rem));
    margin: 0;
    padding: 0.375rem 0.625rem;
    border: 1px solid var(--surface-4);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--surface-2) 82%, transparent);
    color: var(--color-text-tertiary);
    font-size: 0.75rem;
    line-height: 1.4;
    text-align: center;
    pointer-events: none;
    /* 透明窗口下，这条提示也不该挡住桌面背景的可读性。 */
    backdrop-filter: blur(6px);
}
</style>
