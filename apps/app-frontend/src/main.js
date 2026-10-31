// ⚠ 必须是第一条 import：在 vue-i18n 加载前把 performance.mark/measure 换成空函数，
// 否则每次翻译都有 6 次 User Timing 埋点（滚动掉帧的主因，详见该文件注释）。
import '@/shims/intlify-perf'
import 'overlayscrollbars/overlayscrollbars.css'

// 设置页分组卡片：类名与取值同后台管理 .set-*
import '@/ui/admin/settings.css'
// 界面玻璃参数：模块加载即把 --ui-dim/--ui-card-alpha/--ui-glass-blur 写到 <html>
import '@/store/ui-glass'

import { tooltipDirective } from '@modrinth/ui/directives/tooltip.ts'
import { VueQueryPlugin } from '@tanstack/vue-query'
import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from '@/App.vue'
import { overlayScrollbarsDirective } from '@/directives/overlayScrollbars'
import { installTelemetryHandlers } from '@/helpers/telemetry'
import i18nPlugin from '@/plugins/i18n'
import i18nDebugPlugin from '@/plugins/i18n-debug'
import router from '@/routes'

const pinia = createPinia()

const app = createApp(App)

installTelemetryHandlers()

app.use(VueQueryPlugin)
// Pinia must install before the router: route guards use useNavigationReturnStore().
app.use(pinia)
app.use(router)
app.use(i18nPlugin)
app.use(i18nDebugPlugin)
app.directive('overlay-scrollbars', overlayScrollbarsDirective)
app.directive('tooltip', tooltipDirective)

app.mount('#app')
