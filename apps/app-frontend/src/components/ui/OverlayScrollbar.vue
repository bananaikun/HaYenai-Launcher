<script setup lang="ts">
/**
 * 全站磨砂滚动条（2026-10-30 23:20 重写，取代旧的 GlassScrollbar.vue）
 *
 * 材质 = 后台侧栏那一套（`admin-verbatim.scss` 的 `.side`）：
 *   linear-gradient(180deg, --glass-2 → --glass-1)
 *   + backdrop-filter: blur(30px) saturate(180%)
 *   + 1px --stroke-1 发丝 + 顶部内高光
 * 原生 `::-webkit-scrollbar-thumb` 画不出 backdrop-filter（实测无效，永远只是半透色块），
 * 所以只能在 DOM 上画 —— 这是这次重写的唯一原因。
 *
 * 结构：一个挂在 App 根部的**单例覆盖层**，不包任何业务组件。
 *   1. onMounted 给 `<html>` 加 `.os-overlay`，CSS 据此隐藏全部原生滚动条；
 *      组件没挂上就不会有这个类 ⇒ 原生条照旧，不会出现「整站没滚动条」。
 *   2. `pointermove` 用 elementFromPoint 向上找最近的「真在溢出」的滚动容器；
 *   3. `scroll` 捕获阶段监听，滚到谁就把 thumb 挂到谁身上（容器换页也跟得上）；
 *   4. thumb 用 `position: fixed`，只读几何、不参与布局 ⇒ 不改变任何容器的尺寸。
 *
 * 竖/横两个轴都画（横向溢出时底部出来一根），避免隐藏原生条后横向彻底没提示。
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'

const SIZE = 8
const MIN_LEN = 32
const INSET = 2
const IDLE_MS = 1400

const thumbRef = ref<HTMLElement | null>(null)
const vStyle = ref<Record<string, string>>({ display: 'none' })
const hStyle = ref<Record<string, string>>({ display: 'none' })
const visible = ref(false)
const dragging = ref(false)

let target: HTMLElement | null = null
let idleTimer = 0
let rafId = 0
let point: { x: number; y: number } | null = null
let ro: ResizeObserver | null = null

function scrollableStyle(node: HTMLElement): boolean {
    const cs = getComputedStyle(node)
    const ok = (v: string): boolean => v === 'auto' || v === 'scroll' || v === 'overlay'
    // 只靠 overflowY 会漏掉「只横向滚动」的容器（overflow-x: auto + overflow-y: hidden）
    return ok(cs.overflowY) || ok(cs.overflowX)
}

function overflowing(node: HTMLElement): boolean {
    return node.scrollHeight - node.clientHeight > 1 || node.scrollWidth - node.clientWidth > 1
}

function findContainer(from: Element | null): HTMLElement | null {
    let node: Element | null = from
    while (node && node !== document.documentElement) {
        if (node instanceof HTMLElement) {
            // 先做零成本的溢出判断，再读 computedStyle，避免每次 pointermove 大批量取样
            if (overflowing(node) && scrollableStyle(node)) return node
        }
        node = node.parentElement
    }
    const root = document.scrollingElement
    if (root instanceof HTMLElement && overflowing(root)) return root
    return null
}

function place(): void {
    const el = target
    if (!el) {
        vStyle.value = { display: 'none' }
        hStyle.value = { display: 'none' }
        visible.value = false
        return
    }

    const rect = el.getBoundingClientRect()
    const boxLeft = rect.left + el.clientLeft
    const boxTop = rect.top + el.clientTop
    const viewW = el.clientWidth
    const viewH = el.clientHeight
    const overY = el.scrollHeight - viewH
    const overX = el.scrollWidth - viewW

    if (viewH < 40 || overY < 2) {
        vStyle.value = { display: 'none' }
    } else {
        const len = Math.max(MIN_LEN, Math.min(viewH, (viewH / el.scrollHeight) * viewH))
        const shift = (el.scrollTop / overY) * (viewH - len)
        vStyle.value = {
            display: 'block',
            left: `${Math.round(boxLeft + viewW - SIZE - INSET)}px`,
            top: `${Math.round(boxTop + shift)}px`,
            width: `${SIZE}px`,
            height: `${Math.round(len)}px`,
        }
    }

    if (viewW < 40 || overX < 2) {
        hStyle.value = { display: 'none' }
    } else {
        const len = Math.max(MIN_LEN, Math.min(viewW, (viewW / el.scrollWidth) * viewW))
        const shift = (el.scrollLeft / overX) * (viewW - len)
        hStyle.value = {
            display: 'block',
            left: `${Math.round(boxLeft + shift)}px`,
            top: `${Math.round(boxTop + viewH - SIZE - INSET)}px`,
            width: `${Math.round(len)}px`,
            height: `${SIZE}px`,
        }
    }
}

function reveal(): void {
    visible.value = true
    if (idleTimer) window.clearTimeout(idleTimer)
    idleTimer = window.setTimeout(() => {
        if (!dragging.value) visible.value = false
    }, IDLE_MS)
}

function setTarget(el: HTMLElement | null): void {
    if (el === target) return
    ro?.disconnect()
    ro = null
    target = el
    if (el) {
        // 内容异步加载后高度会变（列表、懒加载图片），跟着重算
        ro = new ResizeObserver(() => place())
        ro.observe(el)
    }
}

function probe(x: number, y: number): void {
    const hit = document.elementFromPoint(x, y)
    if (!hit) return
    if (thumbRef.value && thumbRef.value.contains(hit)) {
        reveal()
        return
    }
    const el = findContainer(hit)
    setTarget(el)
    if (el) {
        place()
        reveal()
    } else {
        visible.value = false
    }
}

function onPointerMove(event: PointerEvent): void {
    if (dragging.value) return
    point = { x: event.clientX, y: event.clientY }
    if (rafId) return
    rafId = requestAnimationFrame(() => {
        rafId = 0
        const p = point
        point = null
        if (p) probe(p.x, p.y)
    })
}

function onScroll(event: Event): void {
    if (dragging.value) return
    const node = event.target === document ? document.scrollingElement : event.target
    if (!(node instanceof HTMLElement)) return
    if (!overflowing(node)) return
    setTarget(scrollableStyle(node) ? node : findContainer(node))
    if (!target) return
    place()
    reveal()
}

/** 拖动 thumb；axis 决定读哪个方向的位移 */
function startDrag(event: PointerEvent, axis: 'y' | 'x'): void {
    const el = target
    if (!el) return
    event.preventDefault()
    dragging.value = true
    visible.value = true

    const node = event.currentTarget as HTMLElement
    node.setPointerCapture(event.pointerId)

    const start = axis === 'y' ? event.clientY : event.clientX
    const startScroll = axis === 'y' ? el.scrollTop : el.scrollLeft

    const view = axis === 'y' ? el.clientHeight : el.clientWidth
    const total = axis === 'y' ? el.scrollHeight : el.scrollWidth
    const len = Math.max(MIN_LEN, Math.min(view, (view / total) * view))
    const track = Math.max(1, view - len)
    const ratio = (total - view) / track

    const move = (e: PointerEvent): void => {
        const delta = ((axis === 'y' ? e.clientY : e.clientX) - start) * ratio
        if (axis === 'y') el.scrollTop = startScroll + delta
        else el.scrollLeft = startScroll + delta
        // 拖动期间 onScroll 会提前返回（避免中途换目标），位置得在这里手动跟手
        place()
    }
    const end = (e: PointerEvent): void => {
        dragging.value = false
        node.releasePointerCapture(e.pointerId)
        node.removeEventListener('pointermove', move)
        node.removeEventListener('pointerup', end)
        node.removeEventListener('pointercancel', end)
        reveal()
    }
    node.addEventListener('pointermove', move)
    node.addEventListener('pointerup', end)
    node.addEventListener('pointercancel', end)
}

/** thumb 在覆盖层里，不是容器的后代 ⇒ 滚轮要手动转给容器 */
function onThumbWheel(event: WheelEvent): void {
    const el = target
    if (!el) return
    el.scrollTop += event.deltaY
    el.scrollLeft += event.deltaX
}

onMounted(() => {
    document.documentElement.classList.add('os-overlay')
    window.addEventListener('pointermove', onPointerMove, { passive: true })
    // scroll 不冒泡，但捕获阶段能收到子元素（含 document 自身）的滚动
    window.addEventListener('scroll', onScroll, { capture: true, passive: true })
    window.addEventListener('resize', place, { passive: true })
})

onBeforeUnmount(() => {
    document.documentElement.classList.remove('os-overlay')
    window.removeEventListener('pointermove', onPointerMove)
    window.removeEventListener('scroll', onScroll, { capture: true })
    window.removeEventListener('resize', place)
    if (rafId) cancelAnimationFrame(rafId)
    if (idleTimer) window.clearTimeout(idleTimer)
    ro?.disconnect()
    ro = null
    target = null
})
</script>

<template>
    <Teleport to="body">
        <div
            ref="thumbRef"
            class="os-thumb"
            :class="{ 'is-visible': visible, 'is-dragging': dragging }"
            :style="vStyle"
            aria-hidden="true"
            @pointerdown="startDrag($event, 'y')"
            @wheel.prevent="onThumbWheel"
        />
        <div
            class="os-thumb os-thumb--h"
            :class="{ 'is-visible': visible, 'is-dragging': dragging }"
            :style="hStyle"
            aria-hidden="true"
            @pointerdown="startDrag($event, 'x')"
            @wheel.prevent="onThumbWheel"
        />
    </Teleport>
</template>

<style lang="scss">
/* 只在覆盖层挂载后才隐藏原生滚动条：组件掉线就自动退回原生，不会「整站没滚动条」 */
html.os-overlay,
html.os-overlay * {
    scrollbar-width: none !important;
    -ms-overflow-style: none !important;
}

html.os-overlay ::-webkit-scrollbar,
html.os-overlay *::-webkit-scrollbar {
    width: 0 !important;
    height: 0 !important;
    display: none !important;
}

/* 材质逐字取后台侧栏 `.side`：玻璃渐变 + blur30/saturate180 + 发丝 + 内高光 */
.os-thumb {
    position: fixed;
    z-index: 2147483000;
    box-sizing: border-box;
    border-radius: var(--r-pill, 999px);
    background: linear-gradient(180deg, var(--glass-2, rgba(255, 255, 255, 0.09)) 0%, var(--glass-1, rgba(255, 255, 255, 0.06)) 100%);
    border: 1px solid var(--stroke-1, rgba(255, 255, 255, 0.1));
    box-shadow:
        inset 0 1px 0 rgba(255, 255, 255, 0.22),
        0 1px 6px rgba(0, 0, 0, 0.18);
    -webkit-backdrop-filter: blur(30px) saturate(180%);
    backdrop-filter: blur(30px) saturate(180%);
    opacity: 0;
    pointer-events: none;
    cursor: default;
    transition:
        opacity 180ms ease,
        background 160ms ease,
        border-color 160ms ease;
}

.os-thumb--h {
    background: linear-gradient(90deg, var(--glass-2, rgba(255, 255, 255, 0.09)) 0%, var(--glass-1, rgba(255, 255, 255, 0.06)) 100%);
}

.os-thumb.is-visible {
    opacity: 1;
    pointer-events: auto;
}

.os-thumb.is-visible:hover,
.os-thumb.is-dragging {
    background: linear-gradient(180deg, var(--glass-3, rgba(255, 255, 255, 0.14)) 0%, var(--glass-2, rgba(255, 255, 255, 0.09)) 100%);
    border-color: var(--stroke-2, rgba(255, 255, 255, 0.18));
}

.os-thumb--h.is-visible:hover,
.os-thumb--h.is-dragging {
    background: linear-gradient(90deg, var(--glass-3, rgba(255, 255, 255, 0.14)) 0%, var(--glass-2, rgba(255, 255, 255, 0.09)) 100%);
}

/* 不支持 backdrop-filter 的运行时：换成更实的玻璃底，别变成看不见的透明条 */
@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    .os-thumb {
        background: color-mix(in srgb, #ffffff 18%, rgba(0, 0, 0, 0.25));
    }
}
</style>
