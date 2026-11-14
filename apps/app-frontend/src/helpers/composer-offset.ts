import { onBeforeUnmount, type Ref, watch } from 'vue'

/**
 * 把页面里「底部固定输入框」的高度写到 `--composer-offset`。
 *
 * 底部的磁悬条（MagneticBar）是 `position: fixed` 的，只要页面有输入框，
 * 它就会压在输入框上。这里让页面主动报出输入框高度，磁悬条按这个值上移，
 * 不用每个页面各自去改磁悬条的定位。
 */
export function useComposerOffset(target: Ref<HTMLElement | null>) {
    let observer: ResizeObserver | null = null

    /**
     * 报「输入框顶边到窗口底部的距离」，不是输入框高度。
     *
     * 磁悬条用 `bottom: calc(18px + var(--composer-offset))` 定位：
     * 只有把「输入框下方的留白」也算进去，浮条底边才会正好落在输入框上方 18px。
     * 只报高度的话，浮条会压在输入框上（实测重叠 26px）。
     */
    const apply = () => {
        const element = target.value
        if (!element) {
            document.documentElement.style.removeProperty('--composer-offset')
            return
        }
        const rect = element.getBoundingClientRect()
        const offset = Math.max(0, Math.round(window.innerHeight - rect.top))
        document.documentElement.style.setProperty('--composer-offset', `${offset}px`)
    }

    // 窗口尺寸变化时 rect.top 会变，但输入框自身尺寸可能不变 —— ResizeObserver 收不到，
    // 所以再挂一个 resize 监听。
    const onResize = () => apply()
    window.addEventListener('resize', onResize)

    const observe = (el: HTMLElement | null) => {
        observer?.disconnect()
        observer = null
        if (el) {
            observer = new ResizeObserver(apply)
            observer.observe(el)
        }
        apply()
    }

    watch(target, observe, { immediate: true })

    onBeforeUnmount(() => {
        observer?.disconnect()
        window.removeEventListener('resize', onResize)
        document.documentElement.style.removeProperty('--composer-offset')
    })
}
