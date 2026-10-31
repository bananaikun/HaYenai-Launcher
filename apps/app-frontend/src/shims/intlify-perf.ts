/**
 * 关掉 intlify / vue-i18n 的 User Timing 埋点。
 *
 * 为什么必须关（2026-10-31，实例内容列表「滚动卡、掉帧」的头号耗时）：
 * `@intlify/core-base` 在模块加载时**无条件**安装 `performance.mark/measure`：
 *
 *     if (perf && perf.mark && perf.measure && perf.clearMarks && perf.clearMeasures) {
 *         mark = (tag) => perf.mark(tag)
 *         measure = (name, a, b) => { perf.measure(...); perf.clearMarks(a); perf.clearMarks(b) }
 *     }
 *
 * 每次 `t()` 解析消息都要走 `resolveMessageFormat` + `compileMessageFormat` +
 * `evaluateMessage`，也就是**一次翻译 6 次 mark/measure/clearMarks**。列表滚动时
 * 每帧几十上百次翻译 ⇒ 实机 CPU 采样里 `mark` 占 60%、`measure` 占 32%。
 * 这段代码没有 `__DEV__` 判断，**生产包里一样跑**。
 *
 * 这里在 vue-i18n 之前把两个 API 换成空函数（保留属性存在，否则 intlify 会去
 * 判断 `perf.mark` 是否存在）。app 自己不读 User Timing 数据，所以没有别的损失；
 * Chrome DevTools 的 Performance 面板里会少掉 intlify 那几条 user timing 轨道。
 *
 * ⚠ 必须作为 `main.js` 的**第一条** import —— ESM 按 import 顺序求值，
 * 晚于 `@/i18n.config` 就来不及了（那时 mark/measure 已经被捕获）。
 */
const perf = typeof window !== 'undefined' ? window.performance : undefined

if (perf && typeof perf.mark === 'function' && typeof perf.measure === 'function') {
    const noop = () => undefined
    // 只覆盖这两个；getEntriesByType / now / longtask 观测一律不动。
    Object.defineProperty(perf, 'mark', { value: noop, writable: true, configurable: true })
    Object.defineProperty(perf, 'measure', { value: noop, writable: true, configurable: true })
}

export {}
