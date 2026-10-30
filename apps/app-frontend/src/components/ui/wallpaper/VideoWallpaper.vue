<script setup lang="ts">
/**
 * 视频类（`project.json.type === "video"`）壁纸。
 *
 * 刻意**不进 WebGL**：原生 `<video>` 的帧走 GPU 的视频引擎（Media Foundation /
 * D3D11），不进 JS 堆、不占额外显存；而 `webwallgl` 的 video 分支要多一个 WebGL2
 * 上下文、把每帧当纹理上传，只为播一个 mp4 不值。
 *
 * 四个坑（架构文档 §2.1 已列，这里是落地）：
 *
 * 1. **必须 `muted`**：Chromium 的自动播放策略只对无声视频放行，有声的 `play()`
 *    会被拒（Promise reject）。壁纸永远静音，所以这条天然满足。
 * 2. **`preload` 只能是 `metadata`**：WE 视频壁纸动辄几十上百 MB，
 *    `auto` 会预取整片。循环靠 `loop`，不用 JS seek。
 * 3. **绝不 canvas 抽帧**：`drawImage(video)` 会把帧从 GPU 回读到 CPU，
 *    帧率腰斩、内存翻倍 —— 正是「低占用」的反面。所以这里没有任何 canvas。
 * 4. **关掉画中画/下载入口**：`disablePictureInPicture` + `controls=false` +
 *    `tabindex="-1"`，免得用户右键出「画中画 / 另存为」。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'

import { wallpaperAssetUrl } from '@/helpers/wallpaper'

const props = withDefaults(
    defineProps<{
        /** 包 id。 */
        wallpaperId: string
        /** 包内视频文件的相对路径（`project.json.file`）。 */
        file: string
        /** 背景层现在可见吗。不可见时 `pause()` —— 硬解也要停下来。 */
        active?: boolean
    }>(),
    { active: true },
)

const emit = defineEmits<{
    /** 首帧解码完成，可以撤掉海报了。 */
    (event: 'first-frame'): void
    /** 播不出来。理由是一句 key，父组件翻成文案。 */
    (event: 'fail', reason: string): void
}>()

const video = ref<HTMLVideoElement | null>(null)
/** 已经出过画面。同一次挂载只报一次，避免 seek/loop 反复触发。 */
let reportedFirstFrame = false
let disposed = false

const source = computed(() => wallpaperAssetUrl(props.wallpaperId, props.file))

/** 播放。自动播放被拒时如实上报，让父组件退回海报。 */
async function play(): Promise<void> {
    const element = video.value
    if (!element || disposed || !props.active) return
    try {
        await element.play()
        resumeAttempts = 0
        stuckTicks = 0
    } catch (error) {
        // `muted` 已经加了，走到这里通常是解码器不支持这个编码（HEVC / AV1 缺硬解）。
        if (disposed) return
        console.warn('[wallpaper] 视频播放被拒', error)
        emit('fail', 'video-blocked')
    }
}

/** 暂停。`pause()` 是同步且幂等的。 */
function pause(): void {
    video.value?.pause()
}

function onLoadedData(): void {
    if (disposed) return
    if (!reportedFirstFrame) {
        reportedFirstFrame = true
        emit('first-frame')
    }
    void play()
}

function onError(): void {
    if (disposed) return
    console.warn('[wallpaper] 视频加载失败', video.value?.error)
    emit('fail', 'video-error')
}

/**
 * 后台标签页会被 Chromium 节流（rAF 停、媒体可能被挂起），切回来时画面可能冻在
 * 最后一帧。`visibilitychange` 与窗口 `blur` 都接一遍，恢复时重新 `play()`。
 */
function onVisibilityChange(): void {
    if (document.hidden) pause()
    else void play()
}

/**
 * 窗口失焦 ≠ 看不见。
 *
 * WebView 在焦点落到标题栏按钮、原生右键菜单、甚至一次 IME 弹窗上时都会收到
 * `blur`，但壁纸明明还在屏幕上。原先直接 `pause()`，结果是一次误判就把壁纸冻结，
 * 之后只有再点回页面正文才会恢复 —— 用户看到的就是「壁纸不动」。
 * 以 Tauri 的窗口焦点为准：窗口真的在后台才停。
 */
function onWindowBlur(): void {
    // **不再在这里 pause。**
    // 实测：手动 `video.play()` 立刻能跑 ⇒ 解码与自动播放策略都没问题，
    // 唯一的停因就是这条"失焦即暂停"。而 WebView 在焦点落到标题栏按钮、
    // 原生右键菜单、IME 弹窗时都会 fire blur，一次误判就把壁纸冻到下次点击正文。
    // 真正该停的是"看不见"：交给 visibilitychange（最小化/被完全遮挡）。
}

function onWindowFocus(): void {
    void play()
}

watch(
    () => props.active,
    (active) => {
        if (active) void play()
        else pause()
    },
)

watch(source, () => {
    reportedFirstFrame = false
    const element = video.value
    if (!element) return
    element.load()
    void play()
})

watch(
    () => video.value,
    (element) => {
        if (!element) return
        document.addEventListener('visibilitychange', onVisibilityChange)
        window.addEventListener('blur', onWindowBlur)
        window.addEventListener('focus', onWindowFocus)
        void play()
        startWatchdog()
    },
    { immediate: true },
)

/* ---------------- 起播看门狗 ----------------
   实机观测：窗口起来后 `readyState` 已经是 4、`error` 为 null、`muted` 为 true，
   但 `paused` 一直是 true；手动 `play()` 立刻能播且能持续播。
   说明不是解码/策略问题，而是**起播那次调用撞上了竞态**（src 还没设好、或 active 尚未翻转，
   于是 play() 被拒后没有人再补一次）。

   与其猜是哪一次竞态，不如让它自愈：只要"该播而没播"就补一发。
   条件收紧（active / 可见 / 有数据）到"确实该播却没播"才动手，不会跟用户的暂停抢。
   连续补 3 次仍起不来就停手（真的解不出来，交给父组件走海报兜底）。 */
let watchdog: number | null = null;
let resumeAttempts = 0;
/* 「playing 但画面不动」的计数：currentTime 连续两拍没前进就认为解码卡死。
   实验室那类重页面会把主线程占满，视频解码随后停在最后一帧 —— 这时
   `paused` 仍是 false，老的看门狗只看 paused，于是永远不救（主人 22:58 报的现象）。 */
let stuckTicks = 0;
let lastTime = -1;

function startWatchdog(): void {
    if (watchdog !== null) return;
    watchdog = window.setInterval(() => {
        const el = video.value;
        if (disposed || !el || !props.active) return;
        if (document.hidden) return;
        if (el.readyState < 2) return;

        if (el.paused) {
            // 起播竞态：最多连补 5 次；补不动就交给海报兜底，但**不永久关掉**看门狗
            if (resumeAttempts >= 5) return;
            resumeAttempts += 1;
            void play();
            return;
        }

        // 已在播：看时间有没有走
        if (el.currentTime === lastTime) {
            stuckTicks += 1;
            if (stuckTicks >= 2) {
                stuckTicks = 0;
                // 轻轻 seek 一下通常就能唤回解码器，不必重设 src（那会闪一帧黑）
                try {
                    const next = el.currentTime + 0.05;
                    el.currentTime = Number.isFinite(el.duration)
                        ? Math.min(next, el.duration)
                        : next;
                } catch {
                    // 有的容器在 seek 期间会抛，忽略即可
                }
                void play();
            }
        } else {
            stuckTicks = 0;
            lastTime = el.currentTime;
        }
    }, 1000);
}

function stopWatchdog(): void {
    if (watchdog !== null) {
        window.clearInterval(watchdog);
        watchdog = null;
    }
}

onBeforeUnmount(() => {
    disposed = true
    stopWatchdog()
    document.removeEventListener('visibilitychange', onVisibilityChange)
    window.removeEventListener('blur', onWindowBlur)
    window.removeEventListener('focus', onWindowFocus)
    pause()
    // 断掉 src 让 WebView 立刻释放硬解会话，而不是等 GC。
    const element = video.value
    if (element) {
        element.removeAttribute('src')
        element.load()
    }
})

defineExpose({ play, pause })
</script>

<template>
    <div class="video-wallpaper" aria-hidden="true">
        <video
            ref="video"
            class="video-wallpaper__element"
            :src="source"
            loop
            muted
            playsinline
            preload="metadata"
            :controls="false"
            :disable-picture-in-picture="true"
            disable-remote-playback
            tabindex="-1"
            @loadeddata="onLoadedData"
            @error="onError"
        />
    </div>
</template>

<style scoped>
.video-wallpaper {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
}

.video-wallpaper__element {
    display: block;
    width: 100%;
    height: 100%;
    /* 壁纸的语义是「铺满」：留黑边比裁掉一点更难看。 */
    object-fit: cover;
    pointer-events: none;
    /* 视频本身就是背景，任何边框/圆角都会在透明窗口上露出接缝。 */
    border: none;
    border-radius: 0;
    background: transparent;
}
</style>
