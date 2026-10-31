<script setup lang="ts">
/**
 * 场景类（`project.json.type === "scene"`）壁纸的渲染层。
 *
 * 三件事，每一件都有个不显然的理由：
 *
 * 1. **动态 import**：`webwallgl` 压缩后 1.05 MB（gzip 约 339 KB），只有真的选中
 *    场景壁纸时才该付这笔下载与解析成本。`import('webwallgl')` 让 Vite 把它切成
 *    独立 chunk，不走主包。
 * 2. **指针从 window 注入**：引擎默认只从场景 canvas 上听 `mousemove`，而背景层
 *    必须 `pointer-events: none`（否则铺满视口的背景会吃掉整个界面的点击）⇒
 *    引擎一个事件都收不到。`pushPointer(u, v, buttons, mods)` 就是库为这种宿主
 *    留的通道，坐标按容器 `getBoundingClientRect()` 归一化后推。
 * 3. **不可见即暂停**：`pause()` 会真的停掉渲染循环（不是降到低帧），
 *    恢复靠幂等的 `resume()`。这一笔是「低占用」里最值钱的。
 */
import type { SceneInstance } from 'webwallgl'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { analyserLive, spectrumPeak, spectrumSnapshot } from '@/helpers/music/music-analyser'
import { currentSong, music, next, pause, prev, toggle } from '@/store/music'
import {
    applyWallpaperTextureScale,
    DEFAULT_WALLPAPER_QUALITY,
    wallpaperBaseUrl,
    type WallpaperQuality,
} from '@/helpers/wallpaper'

const props = withDefaults(
    defineProps<{
        /** 包 id（`packages/<id>` 的目录名）。换包由父组件的 `key` 负责重挂。 */
        wallpaperId: string
        /** 渲染档位。`fps` / 后处理 / AA 热更，`textureScale` 必须重挂。 */
        quality?: WallpaperQuality
        /** 背景层现在可见吗。不可见时暂停渲染循环。 */
        active?: boolean
        /**
         * WE 属性面板的当前值（`project.json` 的 `general.properties` 被用户改过的那部分）。
         * **不传就等于组件全用包内默认值** —— 这也是「壁纸里的组件/开关点了没反应」的根因：
         * 库的挂载选项里有 `properties`，我们之前根本没给。
         */
        properties?: Record<string, unknown>
    }>(),
    {
        quality: () => ({ ...DEFAULT_WALLPAPER_QUALITY }),
        active: true,
        properties: () => ({}),
    },
)

const emit = defineEmits<{
    /** 首帧真的画出来了。父组件据此把海报淡出。 */
    (event: 'first-frame'): void
    /** 挂载或渲染失败。理由是一句已本地化的文案，直接显示，不弹框。 */
    (event: 'fail', reason: string): void
}>()

/**
 * 首帧看门狗。
 *
 * 库的默认值是 60 秒，按「整包取完 + 贴图解码 + 着色器编译」计时 —— 后台那套
 * 部署把它放宽到 300 秒是因为包要过外网隧道。启动器读的是本机磁盘，60 秒本来
 * 够用；这里给到 120 秒纯粹是为了「贴图 4K + 机械盘 + 首次编译着色器」的极端组合
 * 不被误判成失败，代价只是失败时晚一点退回海报。
 */
const MOUNT_TIMEOUT_MS = 120_000

const container = ref<HTMLDivElement | null>(null)

let instance: SceneInstance | null = null
/** 已卸载 / 已换代：所有异步回调看到它就该闭嘴。 */
let disposed = false
/** 每次重挂自增，用来丢弃上一代的迟到结果。 */
let generation = 0
let detachPointer: (() => void) | null = null

/**
 * 把档位翻成库的 `quality` 补丁。
 *
 * **只下发偏离库默认的那几项**（库默认：AA off / 粒子 high / 后处理 high）：
 * 传与不传等价时就不传，少一处将来会漂的副本；更重要的是，显式传
 * `postProcessing` 会挡住库在「探到软件渲染」时强制关后处理的那条自保。
 * 我们的默认档是 medium，所以正常情况下只下发这一项。
 */
function qualityPatch(quality: WallpaperQuality): {
    postProcessing?: 'high' | 'medium' | 'low' | 'off'
    antiAliasing?: 'off' | 'fxaa' | 'msaa2' | 'msaa4'
    particles?: 'off' | 'low' | 'medium' | 'high'
} {
    /* 2026-10-30 20:05：抗锯齿从布尔改成四档（与 WE 客户端一致），
       并补上 `particles`。三项都只在**偏离库默认**时才写，
       少写一个键就少一次缓冲区重建。 */
    const patch: {
        postProcessing?: 'high' | 'medium' | 'low' | 'off'
        antiAliasing?: 'off' | 'fxaa' | 'msaa2' | 'msaa4'
        particles?: 'off' | 'low' | 'medium' | 'high'
    } = {}
    if (quality.postProcessing !== 'high') patch.postProcessing = quality.postProcessing
    if (quality.antiAliasing !== 'off') patch.antiAliasing = quality.antiAliasing
    if (quality.particles !== 'high') patch.particles = quality.particles
    return patch
}

/** 等到容器有真实尺寸为止（最多两帧）。尺寸为 0 时挂载会把画布建成 0×0。 */
async function waitForLayout(element: HTMLElement): Promise<boolean> {
    for (let attempt = 0; attempt < 4; attempt += 1) {
        if (element.clientWidth > 0 && element.clientHeight > 0) return true
        await new Promise<void>((resolve) => {
            window.requestAnimationFrame(() => resolve())
        })
        if (disposed) return false
    }
    return element.clientWidth > 0 && element.clientHeight > 0
}

/** 挂载（或重挂）引擎。 */
async function mountEngine(): Promise<void> {
    const element = container.value
    if (!element || disposed) return

    const current = (generation += 1)
    teardownInstance()

    const laidOut = await waitForLayout(element)
    if (disposed || current !== generation) return
    if (!laidOut) {
        emit('fail', 'container-not-laid-out')
        return
    }

    // 贴图倍率只能通过页面 URL 传，且库只在装配那一刻读 ⇒ 每次挂载前重写一遍。
    applyWallpaperTextureScale(props.quality.textureScale)

    try {
        const { mount, httpSource, createMediaSource } = await import('webwallgl')
        if (disposed || current !== generation) return

        const mounted = await mount(element, {
            source: httpSource(wallpaperBaseUrl(props.wallpaperId)),
            // WE 属性：颜色/开关/滑块等自定义项。库按这个 map 覆盖包内默认值。
            properties: { ...props.properties },
            fps: props.quality.fps,
            mountTimeoutMs: MOUNT_TIMEOUT_MS,
            quality: qualityPatch(props.quality),
            /* 2026-10-31（主人：切/滚列表时「屏闪」）：
               库的 autoQuality 会在帧率掉下来时**当场降一档后处理**（实测日志
               `autoQuality: postProcessing → medium`），画面会明显闪一下；
               而画质本来就有「设置 → 壁纸」里的显式档位，运行期自己偷偷改档
               等于既闪又和用户的设置打架。这里直接关掉自动降档，
               画质只由 `quality` 与 `setQuality()` 决定。 */
            autoQuality: false,
            onDiagnostic: (message, level) => {
                // 降档原因（autoQuality 只降不升）与资源缺失都在这里，
                // 丢掉它就会出现「越用越糊却一句解释都没有」。
                if (level !== 'info') {
                    console.warn(`[wallpaper:${level}] ${message}`)
                }
            },
            onError: (error) => {
                if (disposed || current !== generation) return
                console.warn('[wallpaper] 场景渲染失败', error)
                emit('fail', 'engine-error')
            },
        })

        if (disposed || current !== generation) {
            // 挂载期间被卸载 / 换代：别把实例留成孤儿（它持着 GL 上下文与解码器）。
            mounted.destroy({ releasePkgCache: true })
            return
        }

        instance = mounted
        // 调试钩子：dev 下把实例挂到 window，方便用 CDP 直接验证 setProperties 等通路
        if (import.meta.env.DEV) {
            ;(window as unknown as { __scene?: unknown }).__scene = mounted
        }
        mountPointerRelay(mounted)
        attachAudio(mounted)
        attachMedia(mounted, createMediaSource)
        if (!props.active) {
            // 挂载期间窗口已经不可见了：别让这一帧白跑，也别等下一次 active 变化。
            safeCall(() => mounted.pause())
        }
        emit('first-frame')
    } catch (error) {
        if (disposed || current !== generation) return
        console.warn('[wallpaper] 场景挂载失败', error)
        emit('fail', 'mount-failed')
    }
}

/**
 * 指针注入。只挂 `pointermove`，不补 `pointerdown/up`。
 *
 * `pointermove` 自带 `buttons`，所以「按住拖动」天然是按下态；补 down/up 会让
 * **点击界面上的按钮**也在壁纸上炸一下冲量（`g_PointerState.z`）。
 *
 * `blur` + `document` 的 `mouseleave` 要一起挂：鼠标移出窗口后松手收不到 `mouseup`，
 * 不 `pointerLeave()` 会让按键永久卡在按下态（视差一直偏在一边）。
 */
function mountPointerRelay(target: SceneInstance): void {
    const element = container.value
    if (!element) return

    const push = (event: PointerEvent) => {
        const rect = element.getBoundingClientRect()
        if (!rect.width || !rect.height) return
        target.pushPointer(
            (event.clientX - rect.left) / rect.width,
            (event.clientY - rect.top) / rect.height,
            event.buttons & 1,
            // 修饰键位掩码：bit0 ctrl / bit1 shift / bit2 alt / bit3 meta。
            // 只有 web 类壁纸消费它，scene 收了不用 —— 但契约如此，照传不误。
            (event.ctrlKey ? 1 : 0) |
                (event.shiftKey ? 2 : 0) |
                (event.altKey ? 4 : 0) |
                (event.metaKey ? 8 : 0),
        )
    }
    const leave = () => safeCall(() => target.pointerLeave())

    window.addEventListener('pointermove', push, { passive: true })
    window.addEventListener('blur', leave)
    document.addEventListener('mouseleave', leave)

    detachPointer = () => {
        window.removeEventListener('pointermove', push)
        window.removeEventListener('blur', leave)
        document.removeEventListener('mouseleave', leave)
        detachPointer = null
    }
}

/** 库对非法档位是「调了没反应」而不是抛错，但仍不该让异常冒到渲染链上。 */
/* ---------------- 音频响应 ----------------
   库的音频源默认是**内置的确定性模拟源**，所以可视化本来就会动，但跟你在放什么无关。
   要让它真的跟着音乐跳，宿主必须每帧 `setAudio()` 喂自己的频谱
   （契约：left/right 各 64 段、0..1）。启动器早就有 `music-analyser` 在给歌词条算频谱，
   只是从来没往壁纸这边接 —— 这就是「音频条一个都不对」的全部原因。

   ⚠ 判据必须两问：① 分析图真的在跑（`analyserLive`）；② 这份数据当下非零
   （`spectrumPeak ≥ 0.02`，与库自己的 noiseFloor 同值）。任何一条不满足就 `setAudio(null)`：
   喂一串恒 0 的"合法数据"会把库的模拟源顶掉，而它一旦被顶掉就再也不更新 ——
   条子会彻底静止，比"不跟音乐"更糟。
   ---------------- */
const AUDIBLE_FLOOR = 0.02
let audioTimer: number | null = null
let audioLinked: boolean | null = null

function attachAudio(inst: {
    setAudio?: (source: { snapshot: typeof spectrumSnapshot } | null) => void
}): void {
    detachAudio()
    audioLinked = null
    if (typeof inst.setAudio !== 'function') {
        // 库没这个能力就别空转轮询（webwallgl 2.1 有，老版本没有）
        return
    }

    /* 2026-10-25 15:15 **修掉「音频条一个都不对」的根因**。
       webwallgl 的 `setAudio` 契约是**拉模型**（`types.d.ts` 的 `AudioSource`）：
           setAudio({ snapshot() { return { left: Float32Array(64), right: ... } } })
       渲染循环**每帧自己调一次** `snapshot()`；传数据本身给它是无效的
       —— 之前 `inst.setAudio?.(spectrumSnapshot())` 传的是一个 `{left,right}` 对象，
       库看起来拿到东西了，但 `snapshot` 不是函数，于是一直不读 ⇒ 可视化永远不动。
       正确做法：把 `spectrumSnapshot` **函数引用**塞进 `{ snapshot }`，让库每帧来取。
       另外切源只在"有声 ↔ 无声"**变化时**做一次，不再每帧折腾（省开销，也避免抖动）。 */
    audioTimer = window.setInterval(() => {
        if (disposed) {
            detachAudio()
            return
        }
        let audible = false
        try {
            audible = analyserLive() && spectrumPeak() >= AUDIBLE_FLOOR
        } catch {
            audible = false
        }
        if (audible === audioLinked) return

        if (audible) {
            safeCall(() => inst.setAudio?.({ snapshot: spectrumSnapshot }))
        } else {
            // 跌回无声：交 null，把控制权还给库的模拟源（否则会静止）
            safeCall(() => inst.setAudio?.(null))
        }
        audioLinked = audible
    }, 400)
}

function detachAudio(): void {
    if (audioTimer !== null) {
        window.clearInterval(audioTimer)
        audioTimer = null
    }
    audioLinked = null
}

/* ---------------- 系统媒体（正在播放） ----------------
   2026-10-30 23:2x 新增：库支持 `setMedia()`，而启动器之前**一次都没调用过**
   ⇒ 壁纸里的 Song Title / Artist Name / durationsongv2 / timerestarv2 这些组件
   吃的是库内置的**模拟数据**（假歌名、假进度）。这里把启动器自己的音乐状态接进去。

   契约（types.d.ts）：`MediaSource.snapshot` 是一个**属性**不是函数，渲染循环每帧读它；
   所以我们交一个 getter 返回同一个可变对象，定时把字段刷新成当前播放状态 ——
   不每帧分配、也不用理会库何时读。
   ---------------- */
let mediaTimer: number | null = null

function attachMedia(
    inst: { setMedia?: (src: unknown) => void },
    createMediaSource: (init: Record<string, unknown>) => any,
): void {
    detachMedia()
    if (typeof inst.setMedia !== 'function') return

    let source: any
    try {
        // 让库自己补配色/歌词等必需字段，避免 we 的 shader 读到 undefined
        source = createMediaSource({ hasMedia: false, state: 0 })
    } catch (error) {
        console.warn('[wallpaper] 媒体源构造失败', error)
        return
    }

    const snap = source.snapshot as Record<string, unknown>
    const live = {
        get snapshot() {
            return snap
        },
        // 壁纸里的播放/暂停/上一首/下一首按钮转发给我们自己的播放器
        play: () => void music.play(),
        pause: () => pause(),
        playPause: () => toggle(),
        skipNext: () => next(),
        skipPrevious: () => prev(),
    }

    const sync = (): void => {
        const song = currentSong()
        const active = !!song && !!song.name
        snap.hasMedia = active
        snap.state = active ? (music.playing ? 1 : 2) : 0
        snap.title = song?.name ?? ''
        snap.artist = song?.artist ?? ''
        snap.albumArtist = song?.artist ?? ''
        snap.position = music.currentTime || 0
        snap.duration = music.duration || 0
    }

    sync()
    safeCall(() => inst.setMedia?.(live))
    mediaTimer = window.setInterval(sync, 500)
}

function detachMedia(): void {
    if (mediaTimer !== null) {
        window.clearInterval(mediaTimer)
        mediaTimer = null
    }
}

function safeCall(fn: () => void): void {
    try {
        fn()
    } catch (error) {
        console.warn('[wallpaper] 引擎调用失败', error)
    }
}

/** 销毁当前实例并解绑指针。幂等。 */
function teardownInstance(): void {
    detachPointer?.()
    detachAudio()
    detachMedia()
    const current = instance
    instance = null
    if (!current) return
    try {
        // `releasePkgCache`：本组件是「换包即放弃」的语义，不清缓存会让旧包一直
        // 压在库内缓存里（同壁纸重挂才受益，而换回来时 id 已带内容哈希）。
        current.destroy({ releasePkgCache: true })
    } catch (error) {
        console.warn('[wallpaper] 实例销毁失败', error)
    }
}

/** 在容器的第一块 canvas 上找一下有没有东西 —— 调试用，不影响渲染。 */
function hasCanvas(): boolean {
    return !!container.value?.querySelector('canvas')
}
defineExpose({ hasCanvas })

onMounted(() => {
    void mountEngine()
})

onBeforeUnmount(() => {
    disposed = true
    generation += 1
    teardownInstance()
})

// WE 属性热更：库有 `setProperties`，改颜色/开关不用重挂（重挂会闪一帧黑）
watch(
    () => props.properties,
    (next) => {
        if (!instance || !next) return
        try {
            instance.setProperties({ ...next })
        } catch (error) {
            console.warn('[wallpaper] 属性热更失败', error)
        }
    },
    { deep: true },
)

// 换包：重挂。父组件通常已经用 `key` 做过一次，这里再兜一层。
watch(
    () => props.wallpaperId,
    () => {
        void mountEngine()
    },
)

// 贴图倍率只能靠重挂生效（库只认页面 URL 上的 `resources`）。
watch(
    () => props.quality.textureScale,
    () => {
        void mountEngine()
    },
)

// 帧率 / 后处理 / AA / 粒子都是热更，**不重挂**：重挂一张 100MB 的场景包是可感知的卡顿。
watch(
    () => [props.quality.fps, props.quality.postProcessing, props.quality.antiAliasing, props.quality.particles] as const,
    ([fps, postProcessing, antiAliasing, particles]) => {
        const current = instance
        if (!current) return
        safeCall(() => {
            current.setFps(fps)
            /* 2026-10-30 修两处（库里 QualityOptions 收的就是四档字符串）：
               ⚠ 原来写的是 `antiAliasing: antialiasing ? 'fxaa' : 'off'` —— 值域是
                 'off'|'fxaa'|'msaa2'|'msaa4' 的**字符串**，永远为真 ⇒ 每次改帧率/后处理
                 都把 MSAA 悄悄打回 FXAA（设置里显示 MSAA×4、实际跑 FXAA）。
               ⚠ 依赖里 watch 了 particles 却在热更时**没传** ⇒ 粒子档位改了不生效。 */
            current.setQuality({ postProcessing, antiAliasing, particles })
        })
    },
)

// 可见性 → pause()/resume()。两者都幂等，重复调用无害。
watch(
    () => props.active,
    (active) => {
        const current = instance
        if (!current) return
        safeCall(() => {
            if (active) current.resume()
            else current.pause()
        })
    },
)
</script>

<template>
    <!--
		容器铺满背景层。库会在里面自建一块 canvas（带 data-webwallgl 标记），
		所以这里必须是 div 而不是 canvas：网页类壁纸走 iframe 分支时需要能挂子元素。
	-->
    <div ref="container" class="scene-wallpaper" aria-hidden="true" />
</template>

<style scoped>
.scene-wallpaper {
    position: absolute;
    inset: 0;
    overflow: hidden;
    /* 背景层整体已经 pointer-events: none，这里再声明一次是防呆：
	   任何一层忘了写都会把整个界面的点击吃掉。 */
    pointer-events: none;
}

.scene-wallpaper :deep(canvas) {
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
}
</style>
