<script setup lang="ts">
/**
 * 描边书写文字（照搬后台 `components/fx/StrokeText.tsx`）
 *
 * 每个字符一枚独立 `<svg><text>`，用 `stroke-dasharray` 把字形轮廓从
 * `offset=len` 扫到 0 —— 也就是「逐笔写出」，随后补上填充。
 *
 * 为什么逐字符而不是整句一枚：`stroke-dashoffset` 是元素级属性，`<tspan>`
 * 无法各自错峰；拆成字符级才有真正的书写节奏。
 *
 * 为什么 dash 长度按字号算：Chromium 不支持 `<text>` 的 `pathLength` 归一化，
 * dash 只能按用户单位给，而「字形轮廓周长 ≈ 3.2×字号」（后台实测 2.3~3.0，留余量）。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

const props = withDefaults(
    defineProps<{
        text: string
        /** 字符错峰间隔 ms */
        stagger?: number
    }>(),
    { stagger: 58 },
)

/** 轮廓周长 / 字号。 */
const LEN_K = 3.2
const DUR = 460
const FILL_DELAY = 240
/** 字符盒高度与基线（em） */
const BOX_H = 1.34
const BASE_Y = 1.02

type Geo = { fs: number; fw: number; w: number[] }

const host = ref<HTMLElement | null>(null)
const geo = ref<Geo | null>(null)

/**
 * 量每个字符的实际渲染宽度。
 * 不用 canvas `measureText`：`ctx.font` 不接受 `-apple-system` 这类关键字，
 * 会静默退回 10px sans-serif，量出来只有真值 1/3。这里克隆字体样式到离屏 span，
 * 用 Range 逐字取宽（一次 reflow），量完即弃。
 */
function measure(text: string, src: CSSStyleDeclaration): Geo {
    const fontSize = parseFloat(src.fontSize) || 16
    const fontWeight = parseInt(src.fontWeight, 10) || 400
    const letterSpacing = parseFloat(src.letterSpacing) || 0

    const probe = document.createElement('span')
    probe.style.position = 'absolute'
    probe.style.left = '-99999px'
    probe.style.top = '0'
    probe.style.whiteSpace = 'pre'
    probe.style.fontFamily = src.fontFamily
    probe.style.fontSize = src.fontSize
    probe.style.fontWeight = src.fontWeight
    probe.style.fontStyle = src.fontStyle
    probe.style.letterSpacing = src.letterSpacing
    const node = document.createTextNode(text)
    probe.appendChild(node)
    document.body.appendChild(probe)

    const range = document.createRange()
    let acc = 0
    const widths: number[] = []
    // Range 的索引单位是 UTF-16 code unit，渲染按码点切分：代理对必须整对取
    for (let i = 0; i < text.length; ) {
        const cp = text.codePointAt(i) ?? 0
        const step = cp > 0xffff ? 2 : 1
        range.setStart(node, i)
        range.setEnd(node, i + step)
        const width = range.getBoundingClientRect().width - letterSpacing
        acc += width
        widths.push(width)
        i += step
    }
    const total = probe.getBoundingClientRect().width
    document.body.removeChild(probe)

    // 逐字取宽有亚像素舍入误差，按整句宽度等比校正，避免逐字漂移
    const scale = acc > 0 ? total / acc : 1
    return {
        w: widths.map((x) => Math.max(fontSize * 0.12, x * scale)),
        fs: fontSize,
        fw: fontWeight,
    }
}

function read(): void {
    const el = host.value
    if (!el || !props.text) {
        geo.value = null
        return
    }
    geo.value = measure(props.text, getComputedStyle(el))
}

onMounted(() => {
    read()
    // 字体文件晚于首帧加载时字宽会变，补测一次
    void document.fonts?.ready.then(read).catch(() => {})
    window.addEventListener('resize', read)
})

onUnmounted(() => {
    window.removeEventListener('resize', read)
})

watch(() => props.text, read)

const chars = computed(() => [...props.text])
const boxH = computed(() => (geo.value ? geo.value.fs * BOX_H : 0))
const baseY = computed(() => (geo.value ? geo.value.fs * BASE_Y : 0))
const contourLen = computed(() => (geo.value ? Math.round(geo.value.fs * LEN_K) : 0))

function charStyle(index: number): Record<string, string> {
    return {
        '--stx-len': `${contourLen.value}px`,
        '--stx-delay': `${Math.round(index * props.stagger)}ms`,
        '--stx-dur': `${DUR}ms`,
        '--stx-fill-delay': `${Math.round(index * props.stagger) + FILL_DELAY}ms`,
    }
}
</script>

<template>
    <!-- 测量完成前按普通文本渲染，保证首帧就有内容（同一次绘制内完成测量，看不到闪烁） -->
    <span v-if="!geo" ref="host" class="stx-measuring">{{ text }}</span>
    <span v-else ref="host" class="stx" :aria-label="text" role="text">
        <svg
            v-for="(char, index) in chars"
            :key="index"
            class="stx-c"
            :width="geo.w[index]"
            :height="boxH"
            :viewBox="`0 0 ${geo.w[index]} ${boxH}`"
            :style="charStyle(index)"
            aria-hidden="true"
        >
            <text
                :x="geo.w[index] / 2"
                :y="baseY"
                text-anchor="middle"
                :font-size="geo.fs"
                :font-weight="geo.fw"
                :stroke-dasharray="contourLen"
                stroke-dashoffset="0"
            >
                {{ char }}
            </text>
        </svg>
    </span>
</template>

<style scoped>
/* 2026-10-31：这个组件从后台搬过来时**漏了整块 CSS** —— 模板里
   `--stx-len / --stx-delay / --stx-dur / --stx-fill-delay` 四个变量都算好了、
   也在内联 style 上写着，但没有任何 keyframes 去用它们，所以「逐笔写出」从来没跑过：
   每句歌词都是一次性整行出现。这里把后台那套补回来。

   `stroke-dasharray/dashoffset` 是元素级属性，所以只能逐字符（`<tspan>` 无法各自错峰），
   这也是这个组件拆成「一字符一枚 svg」的原因。 */
.stx,
.stx-measuring {
    display: inline-flex;
    align-items: flex-end;
    white-space: pre;
}

.stx-c {
    overflow: visible;
    flex: none;
}

.stx-c text {
    /* 描边先走、填充后到：`paint-order` 让填充压在描边之上，字形边缘才干净 */
    paint-order: stroke fill;
    fill: currentColor;
    stroke: currentColor;
    fill-opacity: 0;
    stroke-dasharray: var(--stx-len, 0);
    stroke-dashoffset: var(--stx-len, 0);
    animation:
        stx-draw var(--stx-dur, 460ms) cubic-bezier(0.33, 0, 0.2, 1) var(--stx-delay, 0ms)
            both,
        stx-fill 260ms ease var(--stx-fill-delay, 0ms) both;
}

@keyframes stx-draw {
    from {
        stroke-dashoffset: var(--stx-len, 0);
    }
    to {
        stroke-dashoffset: 0;
    }
}

@keyframes stx-fill {
    to {
        fill-opacity: 1;
    }
}

/* 系统开了「减少动态效果」就直出，不要写字动画 */
@media (prefers-reduced-motion: reduce) {
    .stx-c text {
        animation: none;
        fill-opacity: 1;
        stroke-dashoffset: 0;
    }
}
</style>
