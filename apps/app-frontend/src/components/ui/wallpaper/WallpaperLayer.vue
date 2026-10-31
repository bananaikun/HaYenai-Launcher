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
        // 旧帧会在 runPixelSwap 里先拷成底图顶住，顺序很重要：先抓，再让子组件重挂。
        void runPixelSwap()
    },
)

/* ---------------- 换壁纸的「像素交换」过渡 ----------------
   形状照 reactbits 的 pixel-swap（https://www.reactbits.dev/animations/pixel-swap）：
   整屏格子里**每格露的都是真画面**，不是一块纯色；格子从中心一波波带弹簧地弹出来，
   弹完新画面铺满。旧实现用的是灰色块（--surface-3-opaque）—— 那是「遮挡」不是「交换」，
   主人 2026-10-31 明确要求换掉。

   实现要点：
   * 旧帧 / 新帧都从场景 canvas 直接 drawImage 拷成 data URL（jpeg 0.9，约 300KB）；
   * 底层铺旧帧整图 ⇒ 加载期间屏幕上永远有画面（顺带治「空占位壁纸」）；
   * 每格一个 `<i>`，背景是新帧整图，background-position 取负偏移 ⇒ 每格显示自己那一块；
   * 只动 transform + transition-delay，纯合成层动画；
   * prefers-reduced-motion 直接不做。
   ---------------- */
const SWAP_COLS = 14
const SWAP_ROWS = 9
const SWAP_CELL_MS = 520
const SWAP_STAGGER_MS = 420
/** 等新引擎出首帧的上限。38MB 的包 mount 实测约 3s，留足余量。 */
const SWAP_HOLD_MAX_MS = 12_000

interface SwapCell {
    left: number
    top: number
    width: number
    height: number
    delay: number
}

interface SwapState {
    oldUrl: string
    newUrl: string
    /** 整图尺寸（像素），每格的背景按它定位。 */
    imgW: number
    imgH: number
    cells: SwapCell[]
    /** 先全部 scale(0)，下一帧再打开触发过渡。 */
    popped: boolean
}

/** 本组件根节点：`snapCanvas()` 要从它的子树里找场景 canvas。 */
const rootEl = ref<HTMLElement | null>(null)
const swap = ref<SwapState | null>(null)
let swapToken = 0

/** 把场景 canvas 拷成 data URL；拿不到就返回 null（那就不做交换动画）。 */
function snapCanvas(): string | null {
    const host = rootEl.value
    if (!host) return null
    const src = host.querySelector<HTMLCanvasElement>('canvas[data-webwallgl]')
    if (!src || !src.width || !src.height) return null
    const snap = document.createElement('canvas')
    snap.width = src.width
    snap.height = src.height
    const ctx = snap.getContext('2d')
    if (!ctx) return null
    try {
        ctx.drawImage(src, 0, 0)
        return snap.toDataURL('image/jpeg', 0.9)
    } catch {
        return null
    }
}

/** 波浪延迟：按「离中心的归一化距离」排，中心先弹、四周后弹。 */
function buildCells(width: number, height: number): SwapCell[] {
    const cellW = width / SWAP_COLS
    const cellH = height / SWAP_ROWS
    const raw: (SwapCell & { dist: number })[] = []
    let maxDist = 0
    for (let row = 0; row < SWAP_ROWS; row += 1) {
        for (let col = 0; col < SWAP_COLS; col += 1) {
            const cx = (col + 0.5) / SWAP_COLS - 0.5
            const cy = (row + 0.5) / SWAP_ROWS - 0.5
            const dist = Math.hypot(cx, cy)
            maxDist = Math.max(maxDist, dist)
            raw.push({
                left: Math.round(col * cellW),
                top: Math.round(row * cellH),
                width: Math.ceil(cellW) + 1,
                height: Math.ceil(cellH) + 1,
                dist,
                delay: 0,
            })
        }
    }
    return raw.map((cell) => ({
        left: cell.left,
        top: cell.top,
        width: cell.width,
        height: cell.height,
        delay: maxDist > 0 ? Math.round((cell.dist / maxDist) * SWAP_STAGGER_MS) : 0,
    }))
}

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

/** 换包过渡：旧帧立刻顶住 → 等新首帧 → 逐格弹出新帧。 */
async function runPixelSwap(): Promise<void> {
    if (reducedMotion.value) return
    const token = ++swapToken
    const oldUrl = snapCanvas()
    if (!oldUrl) return
    swap.value = null

    // 等新引擎画出第一帧（失败/超时也照常收尾，不把画面卡死）
    const deadline = Date.now() + SWAP_HOLD_MAX_MS
    while (!engineReady.value && !failure.value && Date.now() < deadline) {
        await sleep(80)
        if (token !== swapToken) return
    }
    if (token !== swapToken || failure.value) return

    const newUrl = snapCanvas()
    if (!newUrl) return

    const host = rootEl.value
    const width = host?.clientWidth || window.innerWidth
    const height = host?.clientHeight || window.innerHeight
    swap.value = {
        oldUrl,
        newUrl,
        imgW: width,
        imgH: height,
        cells: buildCells(width, height),
        popped: false,
    }

    // 下一帧再打开开关，否则 transition 不会触发
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
    if (token !== swapToken) return
    if (swap.value) swap.value.popped = true

    await sleep(SWAP_CELL_MS + SWAP_STAGGER_MS + 80)
    if (token !== swapToken) return
    swap.value = null
}

defineExpose({
    /** 调试用：当前渲染方式（`scene` / `video` / `poster`）。 */
    mode,
    /** 调试用：设备能力探测结果。 */
    caps,
})

</script>

<template>
    <div ref="rootEl" class="wallpaper-layer" aria-hidden="true">
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

        <!--
            换包过渡：像素交换。底图是旧帧整图，格子逐格弹出新帧对应的那一块 ——
            不是纯色块，见 runPixelSwap 的注释。
        -->
        <div v-if="swap" class="wallpaper-layer__swap" aria-hidden="true">
            <img class="wallpaper-layer__swap-base" :src="swap.oldUrl" alt="" draggable="false" />
            <span
                v-for="(cell, index) in swap.cells"
                :key="index"
                class="wallpaper-layer__swap-cell"
                :class="{ 'is-popped': swap.popped }"
                :style="{
                    left: cell.left + 'px',
                    top: cell.top + 'px',
                    width: cell.width + 'px',
                    height: cell.height + 'px',
                    transitionDelay: cell.delay + 'ms',
                }"
            >
                <i
                    :style="{
                        backgroundImage: 'url(' + swap.newUrl + ')',
                        backgroundPosition: -cell.left + 'px ' + -cell.top + 'px',
                        backgroundSize: swap.imgW + 'px ' + swap.imgH + 'px',
                    }"
                />
            </span>
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

/* 像素交换过渡：底图＝旧帧整图；每格一个 <i>，背景是新帧整图按负偏移取自己那一块，
   弹出时 scale(0)→scale(1)，用带回弹的曲线做出「弹簧波浪」的手感。 */
.wallpaper-layer__swap {
    position: absolute;
    inset: 0;
    z-index: 3;
    overflow: hidden;
    pointer-events: none;
}

.wallpaper-layer__swap-base {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}

.wallpaper-layer__swap-cell {
    position: absolute;
    overflow: hidden;
    display: block;
    transform: scale3d(0, 0, 1);
    transform-origin: center;
    transition: transform var(--wp-swap-ms, 520ms) cubic-bezier(0.34, 1.56, 0.64, 1);
    will-change: transform;
}

.wallpaper-layer__swap-cell.is-popped {
    transform: scale3d(1, 1, 1);
}

.wallpaper-layer__swap-cell > i {
    position: absolute;
    inset: 0;
    display: block;
    background-repeat: no-repeat;
}

.wallpaper-poster-leave-active {
    transition: opacity 320ms ease;
}


.wallpaper-poster-leave-to {
    opacity: 0;
}

/* 换包过渡的像素块（14×9）。只动 transform，纯合成层。 */

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
