<script setup lang="ts">
/**
 * 网页类壁纸（Wallpaper Engine 的 `web` 类型）
 *
 * 为什么用 `<iframe>` 而不是把 HTML 注入进来：这类包是一整个网页工程
 * （自己的 index.html + js + 素材 + 相对路径），只有给它一个**独立的文档**，
 * 它内部的相对引用才能正常解析。注入会把它的 DOM 和启动器的 DOM 搅在一起，
 * 它的 CSS 也会污染整个界面。
 *
 * `sandbox` 刻意不给 `allow-top-navigation` / `allow-popups`：
 * 壁纸是装饰，它没有理由导航整个窗口或弹窗。`allow-same-origin` 必须给，
 * 否则它读不到自己的素材（同源 `hayenai-we.localhost`）。
 *
 * 失败一律静默交给父组件的兜底链（海报 → 纯色），不弹框。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const props = withDefaults(
    defineProps<{
        /** 包 id */
        id: string
        /** 入口 HTML 的相对路径（`project.json` 的 `file`） */
        entry: string
        /** 取得资源地址的函数由父组件注入，避免这里再依赖协议细节 */
        resolveUrl: (id: string, rel: string) => string
        active?: boolean
        /** 首帧（其实就是 iframe 的 load）失败或超时 */
        onFail?: () => void
    }>(),
    { active: true, onFail: undefined },
)

const frame = ref<HTMLIFrameElement | null>(null)
const loaded = ref(false)
let timeout: number | null = null
let disposed = false

/** 启动器的令牌要透给子文档，否则壁纸里的字全用浏览器默认字体/颜色 */
const src = computed(() => props.resolveUrl(props.id, props.entry))

function onLoad() {
    if (disposed) return
    loaded.value = true
    if (timeout !== null) {
        window.clearTimeout(timeout)
        timeout = null
    }
}

onMounted(() => {
    // 20 秒还没 load 就当它挂了：这类包偶尔会卡在缺失素材上，不能一直白等
    timeout = window.setTimeout(() => {
        if (!loaded.value) props.onFail?.()
    }, 20_000)
})

onBeforeUnmount(() => {
    disposed = true
    if (timeout !== null) window.clearTimeout(timeout)
})
</script>

<template>
    <iframe
        ref="frame"
        class="web-wallpaper"
        :class="{ 'is-hidden': !active }"
        :src="src"
        sandbox="allow-scripts allow-same-origin"
        referrerpolicy="no-referrer"
        scrolling="no"
        @load="onLoad"
    />
</template>

<style scoped>
.web-wallpaper {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    border: 0;
    /* 壁纸不该吃鼠标事件：它盖在内容区下面，但滚动/点选都要穿过去 */
    pointer-events: none;
    background: transparent;
}

/* 「暂停」= 不卸载（卸载会丢它的内部状态），只藏起来省电 */
.web-wallpaper.is-hidden {
    visibility: hidden;
}
</style>
