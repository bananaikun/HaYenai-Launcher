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
import { invoke } from '@tauri-apps/api/core'
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
    /* 2026-10-30 23:5x **口径（三次试错的最终结论，别再改回去）**：
       1. `project.json` 的出厂值**不整体铺** —— 实测会把画面改坏（Mt.Fuji 的
          schemecolor 出厂值是 0 0 0，铺上去白字直接变黑）。scene.json 里烘焙的外观
          才是作者本意。
       2. **不自动注入 language** —— Lonely Cat 的 `language.value == 3`（中文）那组
          是作者写坏的：设成 3 之后 60 个 Clock/Date 图层全部 `visible=false`
          （主人 23:34 报的「猫壁纸的时间组件没了」），设成 1（英文）立刻恢复。
       3. 只有**用户自己在属性面板里选过 language** 时，才把它那一组的出厂值补上
          （本地化包把开关/颜色按语言拆成 barcolor / barcolor1 / barcolor2 …，
          脚本只读当前 language 对应的一组；用户既然选了，就得把整组给全）。 */
    const merged: Record<string, unknown> = {
        ...(wallpaperState.config.properties[id] ?? {}),
    }
    if (merged.language === undefined) return merged

    // 该语言对应的那组出厂值（condition 形如 `language.value == 3`）
    const wanted = Number(merged.language)
    const CONDITION_RE = /language\.value\s*==\s*(\d+)/
    for (const [key, def] of Object.entries(schema.value ?? {})) {
        if (!def || def.value === undefined || def.value === null) continue
        const matched = CONDITION_RE.exec(def.condition ?? '')
        if (!matched) continue
        if (Number(matched[1]) !== wanted) continue
        /* ⚠ **combo 不注入**：这一组里的 combo 是布局选择（clocklocation 居中/右上…、
           barstyle 上下/顶部…）。只补开关类（bool/slider/color）就够启用功能
           （眨眼、时钟透明度、光标特效），位置留给 scene.json 烘焙的样式。 */
        if (def.type === 'combo') continue
        if (merged[key] === undefined) merged[key] = def.value
    }
    return merged
})

/** `project.json` 里 general.properties 的一项（后端原样搬运，不做解释）。 */
type WallpaperPropertyDef = {
    type?: string
    value?: unknown
    condition?: string
}

/** 每个包的属性 schema（名字、类型、出厂值、condition）。 */
const schemaByPackage = new Map<string, Record<string, WallpaperPropertyDef>>()
const schema = ref<Record<string, WallpaperPropertyDef>>({})
/**
 * schema 是否已就绪。**场景等它再挂**：`mount({ properties })` 是一次性的，
 * 而 language 和它对应的那组属性必须在首帧之前就在 properties 里。
 */
const propertiesReady = ref(false)

async function loadPropertySchema(): Promise<void> {
    const id = props.meta?.id
    if (!id) {
        propertiesReady.value = true
        return
    }
    const cached = schemaByPackage.get(id)
    if (cached) {
        schema.value = cached
        propertiesReady.value = true
        return
    }
    try {
        const raw = await invoke<Record<string, WallpaperPropertyDef> | null>(
            'plugin:wallpaper|wallpaper_properties',
            { id },
        )
        const next = raw ?? {}
        schemaByPackage.set(id, next)
        schema.value = next
    } catch {
        // 读不到就当作「没有声明属性」，只传用户覆盖值，不致命
        schema.value = {}
    } finally {
        propertiesReady.value = true
    }
}

watch(
    () => props.meta?.id,
    () => {
        schema.value = {}
        propertiesReady.value = false
        void loadPropertySchema()
        // IPC 卡住也不能让背景永远停在海报上
        window.setTimeout(() => {
            propertiesReady.value = true
        }, 1500)
    },
    { immediate: true },
)

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
    (next, previous) => {
        engineReady.value = false
        failure.value = null
        // 首次挂载（previous === undefined）不盖：启动时本来就有一层海报，
        // 再盖一次只是多闪一下。真正换包才值得遮挡。
        if (!previous || !next || previous === next) return
        void runPixelWipe()
    },
)

/* ---------------- 换壁纸的像素块过渡 ----------------
   换包要重新解析 scene.pkg（这张是 38 MB）再编译着色器，这段空窗期原本直接
   露出海报甚至底色，观感就是"卡一下再蹦出来"。做法：一整屏像素块按对角线
   顺序从 0 撑到 1 把画面盖住 → 等新引擎出首帧（或 2.5 s 兜底）→ 反序收回。

   三条约束：
   * **只动 transform**：纯粹合成层动画，不触发重排/重绘，60 fps 很稳；
   * 块数固定 14×9，不随分辨率变 —— 块是"视觉语言"，不是像素级还原；
   * `prefers-reduced-motion` 直接不做（与海报那套判断共用同一个信号）。
   ---------------- */
const WIPE_COLS = 14
const WIPE_ROWS = 9
const WIPE_BLOCK_MS = 200
const WIPE_STAGGER_MS = 360
const WIPE_HOLD_MAX_MS = 2500
const WIPE_TOTAL_MS = WIPE_BLOCK_MS + WIPE_STAGGER_MS

type WipeStage = 'idle' | 'cover' | 'reveal'
const wipeStage = ref<WipeStage>('idle')

/** 每块的归一化对角线延迟（0..1），盖和收共用，收的时候反过来用。 */
const wipeBlocks = Array.from({ length: WIPE_COLS * WIPE_ROWS }, (_, index) => {
    const x = (index % WIPE_COLS) / (WIPE_COLS - 1)
    const y = Math.floor(index / WIPE_COLS) / (WIPE_ROWS - 1)
    return (x + y) / 2
})

function wipeBlockStyle(weight: number): Record<string, string> {
    const order = wipeStage.value === 'reveal' ? 1 - weight : weight
    return { transitionDelay: `${Math.round(order * WIPE_STAGGER_MS)}ms` }
}

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

/** 每次换包自增：上一轮的 await 醒来发现 token 变了就直接退出。 */
let wipeToken = 0

async function runPixelWipe(): Promise<void> {
    if (reducedMotion.value) return
    const token = ++wipeToken
    wipeStage.value = 'cover'
    await sleep(WIPE_TOTAL_MS + 40)
    if (token !== wipeToken) return

    // 等新壁纸真的画出来；失败/超时也照常收，绝不把画面永久盖死。
    const deadline = Date.now() + WIPE_HOLD_MAX_MS
    while (!engineReady.value && !failure.value && Date.now() < deadline) {
        await sleep(80)
        if (token !== wipeToken) return
    }
    await sleep(120)
    if (token !== wipeToken) return

    wipeStage.value = 'reveal'
    await sleep(WIPE_TOTAL_MS + 40)
    if (token !== wipeToken) return
    wipeStage.value = 'idle'
}


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
            v-if="mode === 'scene' && meta && propertiesReady"
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

        <!-- 换包过渡：一整屏像素块，见 runPixelWipe 的注释 -->
        <div
            v-if="wipeStage !== 'idle'"
            class="wallpaper-layer__wipe"
            :class="`is-${wipeStage}`"
            aria-hidden="true"
        >
            <span
                v-for="(weight, index) in wipeBlocks"
                :key="index"
                class="wallpaper-layer__wipe-block"
                :style="wipeBlockStyle(weight)"
            />
        </div>
    </div>
</template>

<style scoped>
.wallpaper-layer {
    position: absolute;
    /* `.launcher-background` 是 `inset: -3rem`（自定义背景图要出血，避免边缘露底），
       场景壁纸跟着它铺就会被放大 ~7.5%、四周各切掉 48px —— WE 壁纸是按 16:9 构图的
       （文字、音频条贴边），切掉就是构图错位。这里用同样的 3rem 抵消回去，正好等于视口。
       两边都用 rem，根字号变化时一起缩放，不会错位。 */
    inset: 3rem;
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

/* 换包过渡的像素块（14×9）。只动 transform，纯合成层。 */
.wallpaper-layer__wipe {
    position: absolute;
    inset: 0;
    display: grid;
    /* 与 runPixelWipe 里的 WIPE_COLS / WIPE_ROWS 对应 */
    grid-template-columns: repeat(14, 1fr);
    grid-template-rows: repeat(9, 1fr);
    pointer-events: none;
}

.wallpaper-layer__wipe-block {
    background: var(--surface-3-opaque);
    border-radius: 2px;
    /*
	   1.02 而不是 1：整数分块在非整数像素宽高下会露出 1px 的发丝缝。
	   多出来的 2% 被相邻块盖住，看不见。
	*/
    transform: scale3d(0, 0, 1);
    transform-origin: center;
    transition: transform 200ms cubic-bezier(0.22, 1, 0.36, 1);
}

.wallpaper-layer__wipe.is-cover .wallpaper-layer__wipe-block {
    transform: scale3d(1.02, 1.02, 1);
}

.wallpaper-layer__wipe.is-reveal .wallpaper-layer__wipe-block {
    transform: scale3d(0, 0, 1);
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
