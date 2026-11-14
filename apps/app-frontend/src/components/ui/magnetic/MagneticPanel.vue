<script setup lang="ts">
/**
 * 磁悬条的展开面板（Windows 开始菜单式，从条子上方弹出）。
 *
 * 设计取舍：
 * - **所有轮询都在这里做**，让 8 个卡片共享一份数据。每个卡片自己开定时器的话，
 *   面板一打开就是 6 个 1.5s 定时器同时打 IPC，启动器会明显顿一下。
 * - 面板关闭就停轮询：磁悬条长期挂在每个页面上，不能一直扫系统信息。
 * - 卡片只在「启用的组件」里渲染，顺序 = 设置里的数组顺序。
 */
import {
    BotIcon,
    CompassIcon,
    FlaskConicalIcon,
    HomeIcon,
    LibraryIcon,
    SettingsIcon,
    WorldIcon,
} from '@modrinth/assets'
import { defineMessages, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { install_job_listener } from '@/helpers/events'
import { list as listInstances, run as runInstance } from '@/helpers/instance'
import { MAGNETIC_WIDGET_KEYWORDS, magneticWidgetMessages } from '@/helpers/magnetic/labels'
import { currentSong, music } from '@/store/music'
import {
    MAGNETIC_COLUMNS,
    MAGNETIC_WIDGET_DEFAULT_COLS,
    MAGNETIC_WIDGET_OPTION_DEFAULTS,
    readMagneticSettings,
    writeMagneticSettings,
    type MagneticSettings,
    type MagneticWidgetId,
} from '@/helpers/magnetic-settings'

const props = defineProps<{ open: boolean; anchor: { left: number; bottom: number } | null }>()
const emit = defineEmits<{ close: []; 'open-settings': [] }>()

const { formatMessage } = useVIntl()
const router = useRouter()

const messages = defineMessages({
    search: { id: 'app.magnetic.search', defaultMessage: 'Search or jump to…' },
    shortcuts: { id: 'app.magnetic.shortcuts', defaultMessage: 'Jump to' },
    noWidgets: {
        id: 'app.magnetic.no-widgets',
        defaultMessage: 'No widgets enabled — add some in Settings → Home & navigation.',
    },
    settings: { id: 'app.magnetic.settings', defaultMessage: 'Widget settings' },
    addWidget: { id: 'app.magnetic.add-widget', defaultMessage: 'Add widget' },
    allAdded: { id: 'app.magnetic.all-added', defaultMessage: 'Every widget is already shown.' },
    removeWidget: { id: 'app.magnetic.remove-widget', defaultMessage: 'Remove' },
    pinWidget: { id: 'app.magnetic.pin-widget', defaultMessage: 'Pin (lock position)' },
    unpinWidget: { id: 'app.magnetic.unpin-widget', defaultMessage: 'Unpin' },
    widgetSettings: { id: 'app.magnetic.widget-settings', defaultMessage: 'Widget settings' },
    cpu: { id: 'app.magnetic.cpu', defaultMessage: 'CPU' },
    memory: { id: 'app.magnetic.memory', defaultMessage: 'Memory' },
    network: { id: 'app.magnetic.network', defaultMessage: 'Network' },
    serverOnline: { id: 'app.magnetic.server-online', defaultMessage: 'Online' },
    serverOffline: { id: 'app.magnetic.server-offline', defaultMessage: 'Offline' },
    serverUnknown: { id: 'app.magnetic.server-unknown', defaultMessage: 'Checking…' },
    launch: { id: 'app.magnetic.launch', defaultMessage: 'Launch' },
    noLyric: { id: 'app.magnetic.no-lyric', defaultMessage: 'Nothing playing' },
    noDownload: { id: 'app.magnetic.no-download', defaultMessage: 'No active download' },
    // 组件设置弹窗
    widgetPanelTitle: { id: 'app.magnetic.widget-dialog-title', defaultMessage: 'Widget settings' },
    commonTitle: { id: 'app.magnetic.common-title', defaultMessage: 'Layout' },
    widthLabel: { id: 'app.magnetic.width-label', defaultMessage: 'Width' },
    widthUnit: { id: 'app.magnetic.width-unit', defaultMessage: '{cols} columns' },
    resetPosition: { id: 'app.magnetic.reset-position', defaultMessage: 'Reset position' },
    noOptions: {
        id: 'app.magnetic.no-options',
        defaultMessage: 'This widget has no options of its own.',
    },
    serverName: { id: 'app.magnetic.server-name', defaultMessage: 'Server name' },
    serverUrl: { id: 'app.magnetic.server-url', defaultMessage: 'Health check URL' },
    done: { id: 'app.magnetic.done', defaultMessage: 'Done' },
    // 组件专属设置项
    opt24h: { id: 'app.magnetic.opt-24h', defaultMessage: '24-hour clock' },
    optSeconds: { id: 'app.magnetic.opt-seconds', defaultMessage: 'Show seconds' },
    optWeekday: { id: 'app.magnetic.opt-weekday', defaultMessage: 'Show weekday' },
    optRows: { id: 'app.magnetic.opt-rows', defaultMessage: 'Rows' },
    optLaunch: { id: 'app.magnetic.opt-launch', defaultMessage: 'Launch button' },
    optLatency: { id: 'app.magnetic.opt-latency', defaultMessage: 'Show latency' },
    optCover: { id: 'app.magnetic.opt-cover', defaultMessage: 'Show cover' },
    dblLaunch: { id: 'app.magnetic.dbl-launch', defaultMessage: 'Double-click to launch' },
    noFavorites: { id: 'app.magnetic.no-favorites', defaultMessage: 'No favourited instances' },
    disk: { id: 'app.magnetic.disk', defaultMessage: 'Disk' },
    free: { id: 'app.magnetic.free', defaultMessage: 'free' },
    players: { id: 'app.magnetic.players', defaultMessage: 'online' },
    optWeekStartMon: { id: 'app.magnetic.week-mon', defaultMessage: 'Monday' },
    optWeekStartSun: { id: 'app.magnetic.week-sun', defaultMessage: 'Sunday' },
    optInstances: { id: 'app.magnetic.opt-instances', defaultMessage: 'Instances in shortcuts' },
    optWeekStart: { id: 'app.magnetic.opt-week-start', defaultMessage: 'Week starts on' },
    optWeekNumbers: { id: 'app.magnetic.opt-week-numbers', defaultMessage: 'Week numbers' },
    optDownloadRows: { id: 'app.magnetic.opt-download-rows', defaultMessage: 'Rows' },
    optDownloadSpeed: { id: 'app.magnetic.opt-download-speed', defaultMessage: 'Show speed' },
    optLastPlayed: { id: 'app.magnetic.opt-last-played', defaultMessage: 'Last played' },
    optFavoritesOnly: { id: 'app.magnetic.opt-favorites', defaultMessage: 'Favourites only' },
    optStatsInterval: { id: 'app.magnetic.opt-stats-interval', defaultMessage: 'Refresh every' },
    optShowDisk: { id: 'app.magnetic.opt-show-disk', defaultMessage: 'Disk usage' },
    optServerTimeout: { id: 'app.magnetic.opt-server-timeout', defaultMessage: 'Timeout' },
    // 命令面板：路由名与结果类型
    routeHome: { id: 'app.magnetic.route.home', defaultMessage: 'Home' },
    routeInstances: { id: 'app.magnetic.route.instances', defaultMessage: 'Instances' },
    routeBrowse: { id: 'app.magnetic.route.browse', defaultMessage: 'Browse' },
    routeModpacks: { id: 'app.magnetic.route.modpacks', defaultMessage: 'Modpacks' },
    routeLab: { id: 'app.magnetic.route.lab', defaultMessage: 'Lab' },
    routeAgent: { id: 'app.magnetic.route.agent', defaultMessage: 'Agent' },
    routeSettings: { id: 'app.magnetic.route.settings', defaultMessage: 'Settings' },
    kindWidget: { id: 'app.magnetic.kind.widget', defaultMessage: 'Widget' },
    kindRoute: { id: 'app.magnetic.kind.route', defaultMessage: 'Go to' },
    kindInstance: { id: 'app.magnetic.kind.instance', defaultMessage: 'Launch' },
    noResults: { id: 'app.magnetic.no-results', defaultMessage: 'Nothing matched.' },
    dialogLabel: { id: 'app.magnetic.dialog-label', defaultMessage: 'Floating bar menu' },
})

const settings = ref<MagneticSettings>(readMagneticSettings())
const query = ref('')
const searchEl = ref<HTMLInputElement | null>(null)
const now = ref(new Date())

/* ---------------- 自由摆放：布局引擎 / 拖动 / 吸附 / 右键菜单 ---------------- */

/**
 * 布局口径：画布固定 12 列，卡片高度固定。
 *
 * 为什么不按内容自适应高度：自由摆放要算「画布多高」「拖到哪里算对齐」，
 * 高度一变就得重新测、重新排，任何一次测量滞后都会让卡片跳动。
 * 固定高度 + 内容区自己滚动，是这里最省事也最稳的口径。
 */
const COL = MAGNETIC_COLUMNS
const COL_GAP = 8
const ROW_GAP = 8
const CARD_H = 148
const SNAP_DELAY = 2000

interface Box {
    x: number
    y: number
    w: number
    h: number
}

/** 正在拖的组件（用 id 不用下标：只渲染启用的那些，下标和配置数组对不上）。 */
const draggingId = ref<MagneticWidgetId | null>(null)
/** 拖动中的**实时**位置：直接渲染，所以卡片是跟着鼠标走的，不是 ghost 标签。 */
const livePos = ref<{
    id: MagneticWidgetId
    x: number
    y: number
    w: number
    h: number
} | null>(null)
/** 悬停 2 秒会吸附到这张卡片上（只用于高亮提示）。 */
const snapTargetId = ref<MagneticWidgetId | null>(null)

const canvasEl = ref<HTMLElement | null>(null)
let hoverTimer: number | null = null
/** 真的拖动过之后，要把紧接着的那次 click 吞掉，否则松手会触发按钮。 */
let suppressClick = false
let dragMoved = false
let dragStart = { x: 0, y: 0, x0: 0, y0: 0, w: 0 }
const menu = ref<{ x: number; y: number; id: MagneticWidgetId } | null>(null)

/** 组件设置弹窗里可选的宽度（列）。 */
const WIDTH_OPTIONS = [3, 4, 6, 12]
const addOpen = ref(false)

/** 这张卡片合并默认值后的设置。组件里读它，不用到处写 `?? true`。 */
function optsOf(id: MagneticWidgetId) {
    return { ...MAGNETIC_WIDGET_OPTION_DEFAULTS, ...(widgetPref(id)?.opts ?? {}) }
}

function widgetLabel(id: MagneticWidgetId): string {
    return formatMessage(magneticWidgetMessages[id])
}

function widgetPref(id: MagneticWidgetId) {
    return settings.value.widgets.find((item) => item.id === id)
}

/** 高度夹到合理区间：太矮装不下内容，太高会把面板撑爆。 */
function clampHeight(value?: number): number {
    if (value === undefined || !Number.isFinite(value)) return CARD_H
    return Math.min(460, Math.max(110, Math.round(value)))
}

function clampCols(value: number): number {
    return Math.min(COL, Math.max(1, Math.round(value)))
}

function boxOf(id: MagneticWidgetId): Box {
    const pref = widgetPref(id)
    return {
        x: pref?.x ?? 0,
        y: pref?.y ?? 0,
        w: clampCols(pref?.w ?? MAGNETIC_WIDGET_DEFAULT_COLS[id]),
        h: clampHeight(pref?.h),
    }
}

/**
 * 给「还没有坐标」的组件按流式排布补上坐标，然后写回。
 *
 * 只在面板打开时跑：所有组件都齐了就直接返回，不会每次渲染都重排。
 */
function ensureLayout(): void {
    const enabled = settings.value.widgets.filter((item) => item.enabled)
    const missing = enabled.filter(
        (item) => item.x === undefined || item.y === undefined || item.w === undefined,
    )
    if (!missing.length) return

    // 已经在画布上的卡片占到哪一行，新卡片就排在它下面 ——
    // 从 0,0 开始排会和已有卡片**完全重叠**（新加的组件像"没出现"）。
    let startY = 0
    for (const item of enabled) {
        if (missing.includes(item)) continue
        const box = boxOf(item.id)
        startY = Math.max(startY, box.y + box.h + ROW_GAP)
    }

    let cursorX = 0
    let rowY = startY
    let rowH = 0
    for (const item of missing) {
        const w = clampCols(item.w ?? MAGNETIC_WIDGET_DEFAULT_COLS[item.id])
        if (cursorX + w > COL) {
            cursorX = 0
            rowY += rowH + ROW_GAP
            rowH = 0
        }
        item.x = cursorX
        item.y = rowY
        item.w = w
        cursorX += w
        rowH = Math.max(rowH, clampHeight(item.h))
    }
    saveSettings()
}

/** 画布高度：所有卡片底边的最大值（拖动中的也一起算，免得拖出边界被裁）。 */
const canvasHeight = computed(() => {
    let bottom = 0
    // 搜索态下只按「真正渲染出来的卡片」算高度：否则隐藏掉的卡片仍在撑画布，
    // 面板底下会留一大片空白。
    const rendered = new Set(visibleWidgets.value)
    for (const item of settings.value.widgets) {
        if (!item.enabled) continue
        if (!rendered.has(item.id)) continue
        const live = livePos.value?.id === item.id ? livePos.value : null
        const box = boxOf(item.id)
        bottom = Math.max(bottom, (live ? live.y : box.y) + (live ? live.h : box.h))
    }
    return Math.max(bottom + ROW_GAP, CARD_H)
})

function columnWidth(): number {
    const width = canvasEl.value?.clientWidth ?? 700
    return Math.max(24, width / COL)
}

/* ---------------- 拖动 ---------------- */

/**
 * 拖动用 pointer 事件自己实现（HTML5 drag 在 WebView 里只有系统鬼影、没有落点）。
 *
 * 这次是按「自由摆放」做的：
 * - 拖动时卡片**本身**跟随鼠标（改 left/top），不再是跟随标签；
 * - 松手按 8px 纵向网格 + 邻近卡片边吸附；
 * - 停在别的卡片旁边 2 秒，自动贴齐（左/右/上/下取最近的一条边）。
 */
function onCardPointerDown(event: PointerEvent, id: MagneticWidgetId): void {
    const pref = widgetPref(id)
    if (!pref || pref.pinned) return
    if (event.button !== 0) return
    // 只排除真正需要键盘/文本交互的控件；按钮允许起拖 ——
    // 快捷跳转卡片里全是按钮，排除按钮等于整张卡都拖不动。
    if ((event.target as HTMLElement).closest('input, select, textarea')) return

    // 不让这次按下冒泡到 document —— 外层有「点面板外面就收起」的监听，
    // 一旦冒上去，一按卡片菜单就被关掉。
    event.stopPropagation()

    const box = boxOf(id)
    dragStart = { x: event.clientX, y: event.clientY, x0: box.x, y0: box.y, w: box.w }
    draggingId.value = id
    dragMoved = false
    livePos.value = { id, x: box.x, y: box.y, w: box.w, h: box.h }
    window.addEventListener('pointermove', onDragPointerMove)
    window.addEventListener('pointerup', onDragPointerUp)
}

function onDragPointerMove(event: PointerEvent): void {
    const live = livePos.value
    if (!live || !draggingId.value) return
    const dx = event.clientX - dragStart.x
    const dy = event.clientY - dragStart.y
    // 3px 死区：手抖不算拖动，否则每次点击都会被误判成拖过。
    if (!dragMoved && Math.hypot(dx, dy) > 3) dragMoved = true
    if (!dragMoved) return

    live.x = Math.min(COL - live.w, Math.max(0, dragStart.x0 + dx / columnWidth()))
    live.y = Math.max(0, dragStart.y0 + dy)
    scheduleSnap()
}

/** 停住 2 秒就把最近的边对齐 —— 不用精确拖到位。 */
function scheduleSnap(): void {
    const nearest = nearestAlign()
    snapTargetId.value = nearest?.target ?? null
    if (hoverTimer !== null) {
        window.clearTimeout(hoverTimer)
        hoverTimer = null
    }
    if (!nearest) return
    hoverTimer = window.setTimeout(() => {
        hoverTimer = null
        const live = livePos.value
        if (!live) return
        live.x = nearest.pos.x
        live.y = nearest.pos.y
        commitPosition()
    }, SNAP_DELAY)
}

/**
 * 找最近的「可对齐边」。
 *
 * 候选只有四条：贴它的左边、贴它的右边、与它顶对齐、与它底对齐。
 * 位移超过阈值就不算（否则整块面板都在吸，反而拖动不自由）。
 */
function nearestAlign(): {
    pos: { x: number; y: number }
    target: MagneticWidgetId
    dist: number
} | null {
    const live = livePos.value
    if (!live) return null
    const colW = columnWidth()
    let best: { pos: { x: number; y: number }; target: MagneticWidgetId; dist: number } | null =
        null

    for (const other of settings.value.widgets) {
        if (!other.enabled || other.id === live.id) continue
        const box = boxOf(other.id)
        const candidates: { x: number; y: number }[] = [
            { x: box.x - live.w, y: live.y }, // 贴左
            { x: box.x + box.w, y: live.y }, // 贴右
            { x: live.x, y: box.y }, // 顶对齐
            { x: live.x, y: box.y + box.h + ROW_GAP }, // 贴下
            { x: live.x, y: box.y - CARD_H - ROW_GAP }, // 贴上
        ]
        for (const pos of candidates) {
            if (pos.x < 0 || pos.x + live.w > COL || pos.y < 0) continue
            // 列折成像素再算距离，否则「差一列」和「差一像素」不可比。
            const dist = Math.hypot((pos.x - live.x) * colW, pos.y - live.y)
            if (dist > colW * 1.6) continue
            if (!best || dist < best.dist) best = { pos, target: other.id, dist }
        }
    }
    return best
}

function commitPosition(): void {
    const live = livePos.value
    if (!live) return
    const pref = widgetPref(live.id)
    if (!pref) return
    // 落定后吸附到 8px 纵向网格：自由摆放也要有节奏，不然一眼看去全是歪的。
    pref.x = Math.round(live.x)
    pref.y = Math.round(live.y / ROW_GAP) * ROW_GAP
    pref.w = Math.round(live.w)
    pref.h = Math.round(live.h / ROW_GAP) * ROW_GAP
    live.x = pref.x
    live.y = pref.y
    saveSettings()
}

function onDragPointerUp(): void {
    const moved = dragMoved
    if (moved && livePos.value) {
        // 松手时再吸一次（阈值比「悬停吸附」紧）：手抖差几个像素也能贴齐。
        const nearest = nearestAlign()
        if (nearest && nearest.dist < columnWidth()) {
            livePos.value.x = nearest.pos.x
            livePos.value.y = nearest.pos.y
        }
        commitPosition()
        // click 在 pointerup 之后才派发，所以延到下一帧再恢复。
        suppressClick = true
        window.setTimeout(() => (suppressClick = false), 0)
    }
    cleanupDrag()
}

/* ---------------- 拖右下角改大小 ---------------- */

const resizing = ref<{
    id: MagneticWidgetId
    startX: number
    startY: number
    w0: number
    h0: number
} | null>(null)

function onResizeStart(event: PointerEvent, id: MagneticWidgetId): void {
    const pref = widgetPref(id)
    if (!pref || pref.pinned || event.button !== 0) return
    // 手柄是叠加层、不在卡片 DOM 里，但保险起见还是挡住冒泡
    event.stopPropagation()
    const box = boxOf(id)
    resizing.value = { id, startX: event.clientX, startY: event.clientY, w0: box.w, h0: box.h }
    draggingId.value = id
    livePos.value = { id, x: box.x, y: box.y, w: box.w, h: box.h }
    window.addEventListener('pointermove', onResizeMove)
    window.addEventListener('pointerup', onResizeEnd)
}

function onResizeMove(event: PointerEvent): void {
    const r = resizing.value
    const live = livePos.value
    if (!r || !live) return
    const colW = columnWidth()
    const maxCols = COL - live.x
    // 宽度跟列走、高度跟像素走：两者都夹在可用范围内，不会撑破面板
    live.w = Math.min(maxCols, Math.max(1, r.w0 + (event.clientX - r.startX) / colW))
    live.h = Math.min(460, Math.max(110, r.h0 + (event.clientY - r.startY)))
}

function onResizeEnd(): void {
    const live = livePos.value
    window.removeEventListener('pointermove', onResizeMove)
    window.removeEventListener('pointerup', onResizeEnd)
    resizing.value = null
    if (!live) {
        cleanupDrag()
        return
    }
    // 松手才取整：宽度整数列、高度 8px 网格
    live.w = Math.round(live.w)
    live.h = Math.round(live.h / ROW_GAP) * ROW_GAP
    commitPosition()
    suppressClick = true
    window.setTimeout(() => (suppressClick = false), 0)
    cleanupDrag()
}

/** 手柄贴在卡片右下角内侧（跟着拖动/缩放实时走）。 */
function resizeHandleStyle(id: MagneticWidgetId) {
    const live = livePos.value?.id === id ? livePos.value : null
    const box = boxOf(id)
    const x = live ? live.x : box.x
    const y = live ? live.y : box.y
    const w = live ? live.w : box.w
    const h = live ? live.h : box.h
    return {
        left: `calc(100% / ${COL} * ${x + w} - 14px)`,
        top: `${y + h - 14}px`,
    }
}

function onClickCapture(event: MouseEvent): void {
    if (!suppressClick) return
    suppressClick = false
    event.stopPropagation()
    event.preventDefault()
}

function cleanupDrag(): void {
    draggingId.value = null
    livePos.value = null
    snapTargetId.value = null
    if (hoverTimer !== null) {
        window.clearTimeout(hoverTimer)
        hoverTimer = null
    }
    window.removeEventListener('pointermove', onDragPointerMove)
    window.removeEventListener('pointerup', onDragPointerUp)
}

function saveSettings(): void {
    // 刷新频率改了要重开定时器，否则要等下次打开面板才生效
    if (statsTimer !== null) {
        window.clearInterval(statsTimer)
        statsTimer = window.setInterval(pollStats, Math.max(500, optsOf('system').statsInterval))
    }
    writeMagneticSettings(settings.value)
}

/**
 * 把「可拖动 / 右键菜单 / 高亮」打包给模板 —— 8 个卡片各写一遍太容易写漏。
 */
/**
 * 把「位置 / 拖动 / 右键菜单 / 高亮」打包给模板 —— 8 个卡片各写一遍太容易写漏。
 *
 * 位置用 `calc(100% / 12 * 列数)` 折算，所以面板宽度变了卡片比例不变形。
 */
function cardProps(id: MagneticWidgetId) {
    const pref = widgetPref(id)
    const live = livePos.value?.id === id ? livePos.value : null
    const box = boxOf(id)
    const x = live ? live.x : box.x
    const y = live ? live.y : box.y
    const w = live ? live.w : box.w
    const h = live ? live.h : box.h
    return {
        'data-widget': id,
        class: {
            'is-dragging': draggingId.value === id,
            'is-pinned': Boolean(pref?.pinned),
            'is-snap-target': snapTargetId.value === id,
        },
        style: {
            left: `calc(100% / ${COL} * ${x} - ${COL_GAP / 2}px)`,
            width: `calc(100% / ${COL} * ${w} - ${COL_GAP}px)`,
            top: `${y}px`,
            height: `${h}px`,
        },
        tabindex: 0,
        onPointerdown: (event: PointerEvent) => onCardPointerDown(event, id),
        onKeydown: (event: KeyboardEvent) => onCardKeydown(event, id),
        onContextmenu: (event: MouseEvent) => {
            event.preventDefault()
            openWidgetMenu(event, id)
        },
    }
}

/**
 * 键盘移动卡片：方向键 = 1 列 / 8px，按住 Shift = 4 列 / 32px。
 * （拖动有 pointer 实现，但键盘用户也应该能摆位置。）
 */
function onCardKeydown(event: KeyboardEvent, id: MagneticWidgetId): void {
    const pref = widgetPref(id)
    if (!pref || pref.pinned) return
    const big = event.shiftKey
    const dx = event.key === 'ArrowLeft' ? -(big ? 4 : 1) : event.key === 'ArrowRight' ? (big ? 4 : 1) : 0
    const dy = event.key === 'ArrowUp' ? -(big ? ROW_GAP * 4 : ROW_GAP) : event.key === 'ArrowDown' ? (big ? ROW_GAP * 4 : ROW_GAP) : 0
    if (!dx && !dy) return
    event.preventDefault()
    const box = boxOf(id)
    pref.x = Math.min(COL - box.w, Math.max(0, box.x + dx))
    pref.y = Math.max(0, box.y + dy)
    pref.w = box.w
    pref.h = box.h
    saveSettings()
}

function openWidgetMenu(event: MouseEvent, id: MagneticWidgetId): void {
    menu.value = { x: event.clientX, y: event.clientY, id }
}

/**
 * 右键菜单的坐标。
 *
 * ⚠ 必须夹进视口，而且菜单要 Teleport 到 body：
 * 面板自己带 `transform: translateX(-50%)`，被 transform 的祖先会把子元素的
 * `position: fixed` 变成「相对该祖先定位」，菜单会跑到屏幕外（实测 top 735 / 窗口 720）。
 */
const MENU_W = 172
const MENU_H = 112

const menuStyle = computed(() => {
    const m = menu.value
    if (!m) return {}
    const left = Math.max(8, Math.min(m.x, window.innerWidth - MENU_W - 8))
    const top = Math.max(8, Math.min(m.y, window.innerHeight - MENU_H - 8))
    return { left: `${left}px`, top: `${top}px` }
})

function closeWidgetMenu(): void {
    menu.value = null
    addOpen.value = false
}

function removeWidget(id: MagneticWidgetId): void {
    const item = widgetPref(id)
    if (item) item.enabled = false
    saveSettings()
    closeWidgetMenu()
}

/** 正在弹设置的那张卡片（null = 没弹窗）。 */
const settingsFor = ref<MagneticWidgetId | null>(null)

/**
 * 右键 → 组件设置：弹窗就地改，不跳页。
 * （主人明确：点「组件设置」是弹窗，不是跳转到设置界面。）
 */
function openSettingsFromMenu(): void {
    const id = menu.value?.id ?? null
    closeWidgetMenu()
    if (id) settingsFor.value = id
}

type SegLabel = string | { id: string; defaultMessage?: string }

type OptionRow = {
    key: keyof MagneticWidgetOptions
    label: { id: string; defaultMessage: string }
    /** toggle 开关 / seg 分段选择 / instances 实例多选 / text 文本 */
    kind: 'toggle' | 'seg' | 'instances' | 'text'
    /** seg 的每个选项：既可以是写死的字面量（"1s"），也可以是一条 i18n 文案。 */
    options?: { value: number; label: SegLabel }[]
    placeholder?: string
}

/** seg 选项的文字：字面量直接用，文案走 formatMessage（切换语言要跟着变）。 */
function segLabel(label: SegLabel): string {
    return typeof label === 'string' ? label : formatMessage(label)
}

/**
 * 每个组件能配什么，全在这张表里。
 *
 * 加新设置 = 在 `MagneticWidgetOptions` 加字段 + 在这里加一行 + 组件里读 `optsOf(id)`。
 * 弹窗渲染是数据驱动的，不用为每个组件手写一段模板。
 */
const OPTION_ROWS: Partial<Record<MagneticWidgetId, OptionRow[]>> = {
    clock: [
        { key: 'clock24h', label: messages.opt24h, kind: 'toggle' },
        { key: 'clockSeconds', label: messages.optSeconds, kind: 'toggle' },
    ],
    date: [{ key: 'dateWeekday', label: messages.optWeekday, kind: 'toggle' }],
    calendar: [
        {
            key: 'calendarWeekStart',
            label: messages.optWeekStart,
            kind: 'seg',
            options: [
                { value: 1, label: messages.optWeekStartMon },
                { value: 0, label: messages.optWeekStartSun },
            ],
        },
        { key: 'calendarWeekNumbers', label: messages.optWeekNumbers, kind: 'toggle' },
    ],
    system: [
        { key: 'showCpu', label: messages.cpu, kind: 'toggle' },
        { key: 'showMemory', label: messages.memory, kind: 'toggle' },
        { key: 'showNetwork', label: messages.network, kind: 'toggle' },
        { key: 'showDisk', label: messages.optShowDisk, kind: 'toggle' },
        {
            key: 'statsInterval',
            label: messages.optStatsInterval,
            kind: 'seg',
            options: [
                { value: 1000, label: '1s' },
                { value: 3000, label: '3s' },
                { value: 5000, label: '5s' },
            ],
        },
    ],
    versions: [
        {
            key: 'instanceRows',
            label: messages.optRows,
            kind: 'seg',
            options: [
                { value: 2, label: '2' },
                { value: 4, label: '4' },
                { value: 6, label: '6' },
            ],
        },
        { key: 'instanceLaunch', label: messages.optLaunch, kind: 'toggle' },
        { key: 'instanceLastPlayed', label: messages.optLastPlayed, kind: 'toggle' },
        { key: 'instanceFavoritesOnly', label: messages.optFavoritesOnly, kind: 'toggle' },
    ],
    shortcuts: [{ key: 'shortcutInstances', label: messages.optInstances, kind: 'instances' }],
    downloads: [
        {
            key: 'downloadRows',
            label: messages.optDownloadRows,
            kind: 'seg',
            options: [
                { value: 1, label: '1' },
                { value: 2, label: '2' },
                { value: 3, label: '3' },
            ],
        },
        { key: 'downloadSpeed', label: messages.optDownloadSpeed, kind: 'toggle' },
    ],
    server: [
        { key: 'serverLatency', label: messages.optLatency, kind: 'toggle' },
        {
            key: 'serverTimeout',
            label: messages.optServerTimeout,
            kind: 'seg',
            options: [
                { value: 2000, label: '2s' },
                { value: 4000, label: '4s' },
                { value: 8000, label: '8s' },
            ],
        },
    ],
    lyrics: [{ key: 'lyricsCover', label: messages.optCover, kind: 'toggle' }],
}

const optionRows = computed<OptionRow[]>(() =>
    settingsFor.value ? (OPTION_ROWS[settingsFor.value] ?? []) : [],
)

/** 弹窗里某个选项的当前值（模板里直接调，避免模板里写强制断言）。 */
function optValue(key: keyof MagneticWidgetOptions): unknown {
    return settingsFor.value ? optsOf(settingsFor.value)[key] : false
}

function toggleOption(key: keyof MagneticWidgetOptions): void {
    const id = settingsFor.value
    if (!id) return
    const item = widgetPref(id)
    if (!item) return
    item.opts = { ...(item.opts ?? {}), [key]: !optsOf(id)[key] }
    saveSettings()
}

function setOptionNumber(key: keyof MagneticWidgetOptions, value: number): void {
    const id = settingsFor.value
    if (!id) return
    const item = widgetPref(id)
    if (!item) return
    item.opts = { ...(item.opts ?? {}), [key]: value }
    saveSettings()
}

function closeWidgetSettings(): void {
    settingsFor.value = null
}

/** 弹窗里改宽度：3 / 4 / 6 / 12 列。 */
function setWidgetWidth(id: MagneticWidgetId, width: number): void {
    const item = widgetPref(id)
    if (!item) return
    item.w = clampCols(width)
    // 变宽之后可能超出右边界，往左推回来。
    item.x = Math.min(item.x ?? 0, COL - item.w)
    saveSettings()
}

/** 把这张卡片放回「流式排布」的位置（清理干净交给 ensureLayout）。 */
function resetWidgetPosition(id: MagneticWidgetId): void {
    const item = widgetPref(id)
    if (!item) return
    delete item.x
    delete item.y
    delete item.w
    ensureLayout()
    saveSettings()
}

/** 弹窗里改服务器信息。 */
function setServerField(field: 'name' | 'url', value: string): void {
    settings.value.server[field] = value
    saveSettings()
}

function togglePin(id: MagneticWidgetId): void {
    const item = widgetPref(id)
    if (!item) return
    item.pinned = !item.pinned
    saveSettings()
    closeWidgetMenu()
}

function addWidget(id: MagneticWidgetId): void {
    const item = widgetPref(id)
    if (item) item.enabled = true
    // 新组件没有坐标：先补一次布局，否则它会和左上角那张卡叠在一起。
    ensureLayout()
    saveSettings()
}

const availableWidgets = computed(() =>
    settings.value.widgets.filter((item) => !item.enabled).map((item) => item.id),
)

/**
 * 固定路由格。名字走 i18n（以前是硬编码中文，英文界面下也是中文），
 * `keywords` 是搜索别名，中英拼音都能命中。
 */
const shortcutRoutes = [
    { label: messages.routeHome, icon: HomeIcon, to: '/', keywords: ['home', 'zhuye', 'shouye'] },
    {
        label: messages.routeInstances,
        icon: CompassIcon,
        to: '/instances',
        keywords: ['instance', 'shili', 'banben'],
    },
    {
        label: messages.routeBrowse,
        icon: LibraryIcon,
        to: '/browse',
        keywords: ['browse', 'resource', 'ziyuan', 'moku'],
    },
    {
        label: messages.routeModpacks,
        icon: WorldIcon,
        to: '/browse/modpack',
        keywords: ['modpack', 'zhenghebao', 'zhenghe'],
    },
    {
        label: messages.routeLab,
        icon: FlaskConicalIcon,
        to: '/lab',
        keywords: ['lab', 'shiyantai', 'shiyan'],
    },
    { label: messages.routeAgent, icon: BotIcon, to: '/agent', keywords: ['agent', 'ai'] },
    {
        label: messages.routeSettings,
        icon: SettingsIcon,
        to: '/settings',
        keywords: ['settings', 'shezhi', 'setting'],
    },
]

const shortcuts = computed(() =>
    shortcutRoutes.map((item) => ({ ...item, label: formatMessage(item.label) })),
)

/** 组件在搜索里的可匹配文本：译文 + 英文/拼音别名 + id。 */
function widgetSearchText(id: MagneticWidgetId): string {
    return [widgetLabel(id), ...MAGNETIC_WIDGET_KEYWORDS[id], id].join(' ').toLowerCase()
}

const filteredShortcuts = computed(() => {
    const text = query.value.trim().toLowerCase()
    if (!text) return shortcuts.value
    return shortcuts.value.filter(
        (item) =>
            item.label.toLowerCase().includes(text) ||
            item.keywords.some((word) => word.includes(text)),
    )
})

/* ---------------- 命令面板（搜索即跳转） ----------------
 * placeholder 写的是「搜索或跳转」，所以结果里必须真的能跳：
 * 路由 / 实例 / 组件 三类，方向键选择、回车执行。 */
interface PaletteItem {
    key: string
    label: string
    hint: string
    icon: unknown
    to?: string
    instanceId?: string
    widgetId?: MagneticWidgetId
}

const paletteIndex = ref(0)

const paletteResults = computed<PaletteItem[]>(() => {
    const text = query.value.trim().toLowerCase()
    if (!text) return []
    const out: PaletteItem[] = []

    for (const id of visibleWidgets.value) {
        out.push({
            key: `w:${id}`,
            label: widgetLabel(id),
            hint: formatMessage(messages.kindWidget),
            icon: LibraryIcon,
            widgetId: id,
        })
    }
    for (const item of filteredShortcuts.value) {
        out.push({
            key: `r:${item.to}`,
            label: item.label,
            hint: formatMessage(messages.kindRoute),
            icon: item.icon,
            to: item.to,
        })
    }
    for (const row of instances.value) {
        if (!row.name.toLowerCase().includes(text)) continue
        out.push({
            key: `i:${row.id}`,
            label: row.name,
            hint: formatMessage(messages.kindInstance),
            icon: WorldIcon,
            instanceId: row.id,
        })
    }
    return out.slice(0, 12)
})

watch(paletteResults, () => (paletteIndex.value = 0))

function onSearchKeydown(event: KeyboardEvent): void {
    const list = paletteResults.value
    if (event.key === 'ArrowDown' && list.length) {
        event.preventDefault()
        paletteIndex.value = (paletteIndex.value + 1) % list.length
        return
    }
    if (event.key === 'ArrowUp' && list.length) {
        event.preventDefault()
        paletteIndex.value = (paletteIndex.value - 1 + list.length) % list.length
        return
    }
    if (event.key === 'Enter') {
        const item = list[paletteIndex.value]
        if (item) {
            event.preventDefault()
            runPaletteItem(item)
        }
        return
    }
    if (event.key === 'Escape' && query.value) {
        // 先清搜索词，再按一次才收起面板（否则一次 Esc 就全关了）
        event.preventDefault()
        event.stopPropagation()
        query.value = ''
    }
}

/** 执行一条搜索结果：路由跳转 / 启动实例 / 定位到卡片。 */
function runPaletteItem(item: PaletteItem): void {
    if (item.to) {
        go(item.to)
        return
    }
    if (item.instanceId) {
        void launch(item.instanceId)
        emit('close')
        return
    }
    if (item.widgetId) {
        query.value = ''
        const card = canvasEl.value?.querySelector(`[data-widget="${item.widgetId}"]`)
        card?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
        flashWidget(item.widgetId)
    }
}

/** 定位后高亮一下卡片（1.2s 后自动褪去）。 */
const flashId = ref<MagneticWidgetId | null>(null)
let flashTimer: number | null = null
function flashWidget(id: MagneticWidgetId): void {
    flashId.value = id
    if (flashTimer !== null) window.clearTimeout(flashTimer)
    flashTimer = window.setTimeout(() => {
        flashId.value = null
        flashTimer = null
    }, 1200)
}

/** 快捷跳转里最多塞几个实例（再多就挤成一排小点，失去辨识度）。 */
const MAX_SHORTCUT_INSTANCES = 8

function shortcutInstanceIds(): string[] {
    return (widgetPref('shortcuts')?.opts?.shortcutInstances ?? []).slice(0, MAX_SHORTCUT_INSTANCES)
}

function hasShortcutInstance(id: string): boolean {
    return shortcutInstanceIds().includes(id)
}

function toggleShortcutInstance(id: string): void {
    const item = widgetPref('shortcuts')
    if (!item) return
    const current = item.opts?.shortcutInstances ?? []
    const next = current.includes(id)
        ? current.filter((value) => value !== id)
        : [...current, id].slice(0, MAX_SHORTCUT_INSTANCES)
    item.opts = { ...(item.opts ?? {}), shortcutInstances: next }
    saveSettings()
}

/** 快捷跳转格子 = 固定路由 + 用户勾选的实例（实例双击启动）。 */
const shortcutItems = computed(() => {
    const base = filteredShortcuts.value.map((item) => ({
        key: `route:${item.to}`,
        label: item.label,
        icon: item.icon,
        to: item.to as string | null,
        instanceId: null as string | null,
    }))
    const extra = shortcutInstanceIds()
        .map((id) => instances.value.find((row) => row.id === id))
        .filter((row): row is InstanceRow => Boolean(row))
        .map((row) => ({
            key: `inst:${row.id}`,
            label: row.name,
            // 实例没有统一图标，用 World 占位（与「整合包」同款，不另造图形）
            icon: WorldIcon,
            to: null as string | null,
            instanceId: row.id,
        }))
    return [...base, ...extra]
})

/**
 * 点快捷格子。
 *
 * 实例格子**单击不做动作**：它要响应双击启动，单击就跳页会让第一击把面板带走。
 */
function onShortcut(item: { to: string | null; instanceId: string | null }): void {
    if (item.instanceId) return
    if (item.to) go(item.to)
}

/** 启用的组件；搜索时按组件名过滤。 */
const visibleWidgets = computed<MagneticWidgetId[]>(() => {
    const text = query.value.trim().toLowerCase()
    return settings.value.widgets
        .filter((item) => item.enabled)
        .map((item) => item.id)
        .filter((id) => !text || widgetSearchText(id).includes(text))
})

/* ---------------- 数据：实例 / 系统 / 服务器 ---------------- */

interface ServerPing {
    online: boolean
    latencyMs: number | null
    playersOnline: number | null
    playersMax: number | null
    motd: string | null
    statusBlocked: boolean
}

interface InstanceRow {
    id: string
    name: string
    icon_path?: string | null
    loader?: string
    game_version?: string
    /** 上次游玩时间（后端返回 Date 的序列化形式）。 */
    last_played?: string | Date | null
    /** 收藏时间；非空 = 已收藏。 */
    pinned_at?: string | Date | null
}

const instances = ref<InstanceRow[]>([])
const stats = ref({
    cpu: 0,
    memUsed: 0,
    memTotal: 0,
    rx: 0,
    tx: 0,
    diskTotal: null as number | null,
    diskFree: null as number | null,
})
const serverState = ref<'unknown' | 'online' | 'offline'>('unknown')
/** 上次探测耗时（ms），卡片里可选显示。 */
const serverLatency = ref<number | null>(null)
/** 在线人数（没配人数接口就是 null，不要显示 0）。 */
const serverOnline = ref<number | null>(null)
/** 人数上限（只有 MC 目标能拿到）。 */
const serverMax = ref<number | null>(null)
const song = computed(() => currentSong())
interface RunningJob {
    id: string
    percent: number
    /** 字节/秒，后端没给就是 null（不要用 0 冒充"卡住"）。 */
    speed: number | null
    name: string | null
}

/** 正在跑的安装任务（多条并存）。 */
const jobs = ref<RunningJob[]>([])

/** 卡片里最多显示几条（用户可配 1–3）。 */
const visibleJobs = computed(() => jobs.value.slice(0, optsOf('downloads').downloadRows))
let unlistenJobs: (() => void) | null = null
const launchingId = ref<string | null>(null)

let statsTimer: number | null = null
let clockTimer: number | null = null
let serverTimer: number | null = null

async function pollStats(): Promise<void> {
    try {
        const s = await invoke<{
            cpu: number
            memory_used: number
            memory_total: number
            rx_bytes_per_sec: number
            tx_bytes_per_sec: number
            disk_total_space?: number | null
            disk_available_space?: number | null
        }>('system_stats')
        stats.value = {
            cpu: s.cpu,
            memUsed: s.memory_used,
            memTotal: s.memory_total,
            rx: s.rx_bytes_per_sec,
            tx: s.tx_bytes_per_sec,
            diskTotal: s.disk_total_space ?? null,
            diskFree: s.disk_available_space ?? null,
        }
    } catch {
        // 采样失败就保持上一次的值，不要清零 —— 清零会让仪表盘看起来"跳变"。
    }
}

/**
 * 服务器探测。
 *
 * 支持两种目标（都按真实结果判定，不再"能连上就算在线"）：
 * - `host` / `host:port` → 走本机 SLP 命令 `server_status`（Rust 侧裸 TCP 握手）：
 *   拿到真实延迟、在线/上限人数、MOTD；端口缺省 25565；
 * - `http(s)://…` → 走 `http_probe` 命令（Rust 发 GET）看**真实状态码**，2xx 才算在线。
 *   不用 webview 的 `fetch(mode:'no-cors')`：那是 opaque response，404/500 也会被算成功。
 */
function isMinecraftAddress(value: string): boolean {
    return Boolean(value) && !/^https?:\/\//i.test(value) && !value.includes('/')
}

function withTimeout<T>(task: Promise<T>, ms: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        const timer = window.setTimeout(() => reject(new Error('timeout')), ms)
        task.then(
            (value) => {
                window.clearTimeout(timer)
                resolve(value)
            },
            (error) => {
                window.clearTimeout(timer)
                reject(error)
            },
        )
    })
}


/** `host[:port]` 拆分：缺省端口交给后端（25565）。 */
function splitHostPort(value: string): [string, number | null] {
    const trimmed = value.trim().replace(/^\s+|\s+$/g, '')
    const match = trimmed.match(/^(.+?):(\d{1,5})$/)
    if (!match) return [trimmed, null]
    return [match[1], Number(match[2])]
}

async function pollServer(): Promise<void> {
    const target = settings.value.server.url?.trim()
    if (!target) {
        serverState.value = 'unknown'
        serverOnline.value = null
        serverMax.value = null
        return
    }

    const timeout = Math.max(500, optsOf('server').serverTimeout)

    if (isMinecraftAddress(target)) {
        const [host, port] = splitHostPort(target)
        try {
            const status = await withTimeout(
                invoke<ServerPing>('server_status', { host, port }),
                timeout * 3,
            )
            serverLatency.value = status.latencyMs ?? null
            serverOnline.value = status.playersOnline ?? null
            serverMax.value = status.playersMax ?? null
            // statusBlocked：服务器在，但状态查询被墙（只有延迟是真的），仍算在线。
            serverState.value = status.online ? 'online' : 'offline'
        } catch {
            serverLatency.value = null
            serverOnline.value = null
            serverMax.value = null
            serverState.value = 'offline'
        }
        return
    }

    const startedAt = performance.now()
    try {
        const probe = await invoke<{ status: number; ok: boolean }>('http_probe', {
            url: target,
            timeoutMs: timeout,
        })
        serverLatency.value = Math.round(performance.now() - startedAt)
        // 只有 2xx 算在线：404/500 说明服务在但坏了，不该给绿灯。
        serverState.value = probe.ok ? 'online' : 'offline'
    } catch {
        serverLatency.value = null
        serverState.value = 'offline'
    }
    // 非 MC 目标没有"人数"概念，别拿 0 冒充。
    serverOnline.value = null
    serverMax.value = null
}

async function loadInstances(): Promise<void> {
    try {
        instances.value = (await listInstances()) as InstanceRow[]
    } catch {
        instances.value = []
    }
}

function startTimers(): void {
    timersSignature = timersKey()
    void pollStats()
    // 下载进度：面板打开才订阅，关掉就退订（磁悬条在每个页面都挂着，不能常驻监听）。
    if (settings.value.widgets.some((w) => w.id === 'downloads' && w.enabled)) {
        void install_job_listener((payload: unknown) => {
            const job = payload as {
                job_id?: string
                finished?: string | null
                progress?: { current?: number; total?: number } | null
                summary?: {
                    speed_bytes_per_second?: number | null
                    bytes_downloaded?: number
                    bytes_total?: number | null
                } | null
                display?: { name?: string | null } | null
            }
            const id = String(job?.job_id ?? '')
            if (!id) return

            // 结束的任务要从列表里拿掉，否则会停在一个 100% 的假进度上
            if (job.finished) {
                jobs.value = jobs.value.filter((item) => item.id !== id)
                return
            }

            const summary = job.summary ?? {}
            const current = job.progress?.current ?? 0
            const bytesTotal = summary.bytes_total ?? 0
            const total = job.progress?.total ?? bytesTotal
            let percent = 0
            if (total > 0) {
                percent = (current / total) * 100
            } else if (bytesTotal > 0) {
                percent = ((summary.bytes_downloaded ?? 0) / bytesTotal) * 100
            }
            percent = Math.min(100, Math.max(0, percent))

            const next: RunningJob = {
                id,
                percent,
                speed: summary.speed_bytes_per_second ?? null,
                name: job.display?.name ?? null,
            }
            const index = jobs.value.findIndex((item) => item.id === id)
            if (index >= 0) jobs.value.splice(index, 1, next)
            else jobs.value = [...jobs.value, next]
        })
            .then((unlisten) => {
                unlistenJobs = unlisten
            })
            .catch(() => {
                unlistenJobs = null
            })
    }
    clockTimer = window.setInterval(() => (now.value = new Date()), 1000)
    if (settings.value.widgets.some((w) => w.id === 'system' && w.enabled)) {
        statsTimer = window.setInterval(pollStats, Math.max(500, optsOf('system').statsInterval))
    }
    if (settings.value.widgets.some((w) => w.id === 'server' && w.enabled)) {
        void pollServer()
        serverTimer = window.setInterval(pollServer, 15000)
    }
}

function stopTimers(): void {
    unlistenJobs?.()
    unlistenJobs = null
    for (const timer of [statsTimer, clockTimer, serverTimer]) {
        if (timer !== null) window.clearInterval(timer)
    }
    statsTimer = null
    clockTimer = null
    serverTimer = null
}

async function launch(id: string): Promise<void> {
    launchingId.value = id
    try {
        await runInstance(id)
    } catch {
        // 启动失败由全局通知处理（runInstance 内部已经报过），这里只管收尾。
    } finally {
        launchingId.value = null
    }
}

/* ---------------- 派生显示值 ---------------- */

const clockText = computed(() => {
    const opts = optsOf('clock')
    return now.value.toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit',
        hour12: !opts.clock24h,
        ...(opts.clockSeconds ? { second: '2-digit' as const } : {}),
    })
})
const dateText = computed(() => {
    const opts = optsOf('date')
    return now.value.toLocaleDateString(
        [],
        opts.dateWeekday
            ? { month: 'long', day: 'numeric', weekday: 'long' }
            : { month: 'long', day: 'numeric' },
    )
})
const diskPercent = computed(() => {
    const total = stats.value.diskTotal ?? 0
    if (!total) return 0
    return Math.round((((total - (stats.value.diskFree ?? 0)) / total) * 100))
})

const memPercent = computed(() =>
    stats.value.memTotal ? Math.round((stats.value.memUsed / stats.value.memTotal) * 100) : 0,
)

/** 月历：6 行 × 7 列，周一开头。 */
/** ISO 8601 周数（周一算一周开始）。 */
function isoWeek(date: Date): number {
    const target = new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()))
    const dayNumber = target.getUTCDay() || 7
    target.setUTCDate(target.getUTCDate() + 4 - dayNumber)
    const yearStart = new Date(Date.UTC(target.getUTCFullYear(), 0, 1))
    return Math.ceil(((target.getTime() - yearStart.getTime()) / 86400000 + 1) / 7)
}

/**
 * 月历按**周**分组：这样才画得出「第几周」那一列。
 * `calendarWeekStart` 0 = 周日、1 = 周一（默认，符合中文习惯）。
 */
const calendarWeeks = computed(() => {
    const opts = optsOf('calendar')
    const startsMonday = opts.calendarWeekStart !== 0
    const today = now.value
    const year = today.getFullYear()
    const month = today.getMonth()
    const first = new Date(year, month, 1)
    const lead = startsMonday ? (first.getDay() + 6) % 7 : first.getDay()
    const daysInMonth = new Date(year, month + 1, 0).getDate()

    const cells: { day: number | null; today: boolean; date: Date | null }[] = []
    for (let i = 0; i < lead; i += 1) cells.push({ day: null, today: false, date: null })
    for (let day = 1; day <= daysInMonth; day += 1) {
        cells.push({ day, today: day === today.getDate(), date: new Date(year, month, day) })
    }
    while (cells.length % 7 !== 0) cells.push({ day: null, today: false, date: null })

    const weeks: { week: number; cells: typeof cells }[] = []
    for (let i = 0; i < cells.length; i += 7) {
        const row = cells.slice(i, i + 7)
        // 用这一周里第一个真实日期算周数（整周都是空白时沿用上一周）
        const anchor = row.find((cell) => cell.date)?.date ?? cells[i].date ?? first
        weeks.push({ week: isoWeek(anchor), cells: row })
    }
    return weeks
})

/** 星期表头，跟着「周起始」走。 */
const calendarHead = computed(() => {
    const sunday = ['日', '一', '二', '三', '四', '五', '六']
    return optsOf('calendar').calendarWeekStart === 0 ? sunday : [...sunday.slice(1), sunday[0]]
})

/** 版本实例卡片里真正要显示的那些（收藏过滤 + 条数上限）。 */
const visibleInstances = computed(() => {
    const opts = optsOf('versions')
    const rows = opts.instanceFavoritesOnly
        ? instances.value.filter((row) => Boolean(row.pinned_at))
        : instances.value
    return rows.slice(0, opts.instanceRows)
})

/** 「3 天前」这种相对时间交给 Intl 本地化，不手写中文。 */
const relativeFormat = new Intl.RelativeTimeFormat([], { numeric: 'auto' })

function lastPlayedText(value?: string | Date | null): string {
    if (!value) return ''
    const time = value instanceof Date ? value.getTime() : new Date(value).getTime()
    if (!Number.isFinite(time)) return ''
    const diff = Date.now() - time
    if (diff < 60_000) return relativeFormat.format(0, 'minute')
    if (diff < 3_600_000) return relativeFormat.format(-Math.round(diff / 60_000), 'minute')
    if (diff < 86_400_000) return relativeFormat.format(-Math.round(diff / 3_600_000), 'hour')
    return relativeFormat.format(-Math.round(diff / 86_400_000), 'day')
}

function formatBytes(value: number): string {
    if (!Number.isFinite(value) || value <= 0) return '0 B'
    const units = ['B', 'KB', 'MB', 'GB']
    let size = value
    let index = 0
    while (size >= 1024 && index < units.length - 1) {
        size /= 1024
        index += 1
    }
    return `${size.toFixed(size >= 10 || index === 0 ? 0 : 1)} ${units[index]}`
}

function go(to: string): void {
    emit('close')
    void router.push(to)
}

function refreshSettings(): void {
    settings.value = readMagneticSettings()
}

/**
 * 右键菜单/添加弹层是浮层，点到别处要收起。
 *
 * ⚠ 以前是 setup 顶层无条件挂的监听，任何一次按下都会关菜单 ——
 * 包括按在卡片上准备拖动，所以「一按组件菜单就消失」。
 */
function onDocumentPointerDown(event: PointerEvent): void {
    if (!menu.value && !addOpen.value) return
    const target = event.target as HTMLElement | null
    if (target?.closest('.mbp-menu')) return
    closeWidgetMenu()
}

if (typeof document !== 'undefined') {
    document.addEventListener('pointerdown', onDocumentPointerDown, true)
}

watch(
    () => props.open,
    (open) => {
        if (open) {
            refreshSettings()
            // 补齐「还没有坐标」的组件，之后一律按坐标渲染。
            ensureLayout()
            void loadInstances()
            startTimers()
            // 打开就聚焦搜索：键盘用户不用先 Tab 一轮
            void nextTick(() => searchEl.value?.focus())
        } else {
            stopTimers()
            // 关面板才清任务列表：否则下次打开会先显示上一次的"僵尸进度"。
            // （不能放在 stopTimers 里 —— 拖卡片也会触发一次 stop/start，会把正在进行
            //  的下载列表清空，看起来像"下载突然消失了"。）
            jobs.value = []
            query.value = ''
        }
    },
)

onBeforeUnmount(stopTimers)

/**
 * 「要不要重开定时器」的指纹：启用的组件集合 + 采样间隔。
 *
 * ⚠ 拖动卡片也会 `saveSettings()` → 广播变更 → 走到这里。以前无条件 stop/start，
 * 等于每拖一次就把采样与下载订阅全部重建一遍（进度会抖、订阅会重来）。
 */
function timersKey(): string {
    const ids = settings.value.widgets
        .filter((item) => item.enabled)
        .map((item) => item.id)
        .join(',')
    return `${ids}|${optsOf('system').statsInterval}`
}

let timersSignature = ''

// 设置页改了配置，面板要立刻反映（同一个页面里就能看到变化）。
function onSettingsChanged(): void {
    refreshSettings()
    if (!props.open) return
    // 只有「组件开关 / 采样间隔」真的变了才重建定时器，其余改动（位置、透明度）不碰。
    if (timersKey() === timersSignature) return
    stopTimers()
    startTimers()
}
window.addEventListener('hayenai:magnetic-changed', onSettingsChanged)
onBeforeUnmount(() => window.removeEventListener('hayenai:magnetic-changed', onSettingsChanged))
</script>

<template>
    <Teleport to="body">
        <Transition name="mbp-pop">
            <div
                v-if="open && anchor"
                class="mb-panel"
                data-magnetic-layer
                :style="{
                    '--mbp-alpha': settings.opacity / 100,
                    '--mbp-width': `${settings.panelWidth}px`,
                    left: `${anchor.left}px`,
                    bottom: `${anchor.bottom}px`,
                }"
                role="dialog"
                aria-modal="false"
                :aria-label="formatMessage(messages.dialogLabel)"
                @pointerdown.stop
            >
                <header class="mbp-head">
                    <input
                        ref="searchEl"
                        v-model="query"
                        class="mbp-search"
                        role="combobox"
                        aria-autocomplete="list"
                        :aria-expanded="paletteResults.length > 0"
                        :placeholder="formatMessage(messages.search)"
                        @keydown="onSearchKeydown"
                    />
                    <button
                        type="button"
                        class="mbp-icon-btn"
                        :aria-label="formatMessage(messages.addWidget)"
                        v-tooltip="formatMessage(messages.addWidget)"
                        @click.stop="addOpen = !addOpen"
                    >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M12 5v14M5 12h14" />
                        </svg>
                    </button>
                    <button
                        type="button"
                        class="mbp-icon-btn"
                        :aria-label="formatMessage(messages.settings)"
                        v-tooltip="formatMessage(messages.settings)"
                        @click="emit('open-settings')"
                    >
                        <SettingsIcon />
                    </button>

                    <!-- 命令面板：搜到就能跳（路由 / 实例 / 组件） -->
                    <div
                        v-if="query.trim()"
                        class="mbp-cmdk"
                        role="listbox"
                        @pointerdown.stop
                    >
                        <button
                            v-for="(item, index) in paletteResults"
                            :key="item.key"
                            type="button"
                            role="option"
                            :aria-selected="index === paletteIndex"
                            class="mbp-cmdk-row"
                            :class="{ 'is-active': index === paletteIndex }"
                            @mouseenter="paletteIndex = index"
                            @click.stop="runPaletteItem(item)"
                        >
                            <component :is="item.icon" class="mbp-cmdk-icon" />
                            <span class="mbp-cmdk-label">{{ item.label }}</span>
                            <span class="mbp-cmdk-hint">{{ item.hint }}</span>
                        </button>
                        <p v-if="!paletteResults.length" class="mbp-hint">
                            {{ formatMessage(messages.noResults) }}
                        </p>
                    </div>

                    <!-- 添加组件弹层 -->
                    <div v-if="addOpen" class="mbp-menu" @pointerdown.stop>
                        <span class="mbp-menu-title">{{ formatMessage(messages.addWidget) }}</span>
                        <button
                            v-for="id in availableWidgets"
                            :key="id"
                            type="button"
                            class="mbp-menu-item"
                            @click.stop="addWidget(id)"
                        >
                            {{ widgetLabel(id) }}
                        </button>
                        <p v-if="!availableWidgets.length" class="mbp-hint">
                            {{ formatMessage(messages.allAdded) }}
                        </p>
                    </div>
                </header>

                <div v-if="shortcutItems.length" class="mbp-shortcuts">
                    <button
                        v-for="item in shortcutItems"
                        :key="item.key"
                        type="button"
                        class="mbp-shortcut"
                        v-tooltip="
                            item.instanceId
                                ? `${item.label} · ${formatMessage(messages.dblLaunch)}`
                                : item.label
                        "
                        @click="onShortcut(item)"
                        @dblclick="item.instanceId && launch(item.instanceId)"
                    >
                        <component :is="item.icon" class="mbp-shortcut-icon" />
                        <span>{{ item.label }}</span>
                    </button>
                </div>

                <p v-if="!visibleWidgets.length" class="mbp-empty">
                    {{ formatMessage(messages.noWidgets) }}
                </p>

                <!--
                    自由画布：卡片是绝对定位的，位置来自配置里的 x/y（列 / px）。
                    `--mbp-col` 交给 CSS 折算列宽，面板宽度变了比例不变形。
                -->
                <div v-else class="mbp-scroll">
                    <div ref="canvasEl" class="mbp-canvas" :style="{ height: `${canvasHeight}px` }">
                        <!-- 版本实例 -->
                        <section
                            v-if="visibleWidgets.includes('versions')"
                            class="mbp-card is-wide"
                            v-bind="cardProps('versions')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('versions') }}</span>
                                <div class="mbp-rows">
                                    <div
                                        v-for="item in visibleInstances"
                                        :key="item.id"
                                        class="mbp-row"
                                    >
                                        <span class="mbp-row-name">{{ item.name }}</span>
                                        <span class="mbp-row-meta">{{ item.game_version }}</span>
                                        <span
                                            v-if="
                                                optsOf('versions').instanceLastPlayed &&
                                                item.last_played
                                            "
                                            class="mbp-row-when"
                                        >
                                            {{ lastPlayedText(item.last_played) }}
                                        </span>
                                        <button
                                            v-if="optsOf('versions').instanceLaunch"
                                            type="button"
                                            class="mbp-mini"
                                            :disabled="launchingId === item.id"
                                            @click="launch(item.id)"
                                        >
                                            {{ formatMessage(messages.launch) }}
                                        </button>
                                    </div>
                                    <p v-if="!visibleInstances.length" class="mbp-hint">
                                        {{
                                            optsOf('versions').instanceFavoritesOnly &&
                                            instances.length
                                                ? formatMessage(messages.noFavorites)
                                                : '—'
                                        }}
                                    </p>
                                </div>
                            </div>
                        </section>

                        <!-- 快捷跳转（面板顶部已有；这里是「单独放一块」的形态） -->
                        <section
                            v-if="visibleWidgets.includes('shortcuts')"
                            class="mbp-card"
                            v-bind="cardProps('shortcuts')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('shortcuts') }}</span>
                                <div class="mbp-icon-grid">
                                    <button
                                        v-for="item in shortcutItems.slice(0, 8)"
                                        :key="item.key"
                                        type="button"
                                        class="mbp-icon-cell"
                                        v-tooltip="
                                            item.instanceId
                                                ? `${item.label} · ${formatMessage(messages.dblLaunch)}`
                                                : item.label
                                        "
                                        @click="onShortcut(item)"
                                        @dblclick="item.instanceId && launch(item.instanceId)"
                                    >
                                        <component :is="item.icon" />
                                    </button>
                                </div>
                            </div>
                        </section>

                        <!-- 时间 -->
                        <section
                            v-if="visibleWidgets.includes('clock')"
                            class="mbp-card"
                            v-bind="cardProps('clock')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('clock') }}</span>
                                <strong class="mbp-clock">{{ clockText }}</strong>
                            </div>
                        </section>

                        <!-- 日期 -->
                        <section
                            v-if="visibleWidgets.includes('date')"
                            class="mbp-card"
                            v-bind="cardProps('date')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('date') }}</span>
                                <strong class="mbp-date">{{ dateText }}</strong>
                            </div>
                        </section>

                        <!-- 日历 -->
                        <section
                            v-if="visibleWidgets.includes('calendar')"
                            class="mbp-card is-square"
                            v-bind="cardProps('calendar')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('calendar') }}</span>
                                <div
                                    class="mbp-cal"
                                    :class="{ 'has-weeks': optsOf('calendar').calendarWeekNumbers }"
                                >
                                    <span
                                        v-if="optsOf('calendar').calendarWeekNumbers"
                                        class="mbp-cal-wk-head"
                                    />
                                    <span
                                        v-for="(label, index) in calendarHead"
                                        :key="`h-${index}`"
                                        class="mbp-cal-head"
                                    >
                                        {{ label }}
                                    </span>
                                    <template v-for="row in calendarWeeks" :key="row.week">
                                        <span
                                            v-if="optsOf('calendar').calendarWeekNumbers"
                                            class="mbp-cal-wk"
                                        >
                                            {{ row.week }}
                                        </span>
                                        <span
                                            v-for="(cell, index) in row.cells"
                                            :key="`${row.week}-${index}`"
                                            :class="{ today: cell.today }"
                                        >
                                            {{ cell.day ?? '' }}
                                        </span>
                                    </template>
                                </div>
                            </div>
                        </section>

                        <!-- 系统资源 -->
                        <section
                            v-if="visibleWidgets.includes('system')"
                            class="mbp-card is-wide"
                            v-bind="cardProps('system')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('system') }}</span>
                                <div class="mbp-gauges">
                                    <div v-if="optsOf('system').showCpu" class="mbp-gauge">
                                        <span>{{ formatMessage(messages.cpu) }}</span>
                                        <div class="mbp-meter">
                                            <i :style="{ width: `${stats.cpu}%` }" />
                                        </div>
                                        <b>{{ Math.round(stats.cpu) }}%</b>
                                    </div>
                                    <div v-if="optsOf('system').showMemory" class="mbp-gauge">
                                        <span>{{ formatMessage(messages.memory) }}</span>
                                        <div class="mbp-meter">
                                            <i :style="{ width: `${memPercent}%` }" />
                                        </div>
                                        <b>{{ memPercent }}%</b>
                                    </div>
                                    <div v-if="optsOf('system').showNetwork" class="mbp-gauge">
                                        <span>{{ formatMessage(messages.network) }}</span>
                                        <div class="mbp-net">
                                            ↑{{ formatBytes(stats.tx) }}/s · ↓{{
                                                formatBytes(stats.rx)
                                            }}/s
                                        </div>
                                    </div>
                                    <div v-if="optsOf('system').showDisk" class="mbp-gauge">
                                        <span>{{ formatMessage(messages.disk) }}</span>
                                        <div class="mbp-meter">
                                            <i :style="{ width: `${diskPercent}%` }" />
                                        </div>
                                        <b v-if="stats.diskTotal">
                                            {{ formatBytes(stats.diskFree ?? 0) }}
                                            {{ formatMessage(messages.free) }}
                                        </b>
                                        <b v-else>—</b>
                                    </div>
                                    <p
                                        v-if="
                                            !optsOf('system').showCpu &&
                                            !optsOf('system').showMemory &&
                                            !optsOf('system').showNetwork &&
                                            !optsOf('system').showDisk
                                        "
                                        class="mbp-hint"
                                    >
                                        {{ formatMessage(messages.noOptions) }}
                                    </p>
                                </div>
                            </div>
                        </section>

                        <!-- 正在播放 -->
                        <section
                            v-if="visibleWidgets.includes('lyrics')"
                            class="mbp-card is-wide"
                            v-bind="cardProps('lyrics')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('lyrics') }}</span>
                                <div class="mbp-lyric">
                                    <img
                                        v-if="song?.pic && optsOf('lyrics').lyricsCover"
                                        class="mbp-lyric-cover"
                                        :src="song.pic"
                                        alt=""
                                        loading="lazy"
                                    />
                                    <div class="mbp-lyric-text">
                                        <strong>{{
                                            song?.name ?? formatMessage(messages.noLyric)
                                        }}</strong>
                                        <span>{{ music.activeLyric || song?.artist || '' }}</span>
                                    </div>
                                </div>
                            </div>
                        </section>

                        <!-- 下载进度 -->
                        <section
                            v-if="visibleWidgets.includes('downloads')"
                            class="mbp-card"
                            v-bind="cardProps('downloads')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('downloads') }}</span>
                                <div v-if="visibleJobs.length" class="mbp-jobs">
                                    <div v-for="job in visibleJobs" :key="job.id" class="mbp-job">
                                        <div class="mbp-job-top">
                                            <span class="mbp-job-name">{{ job.name ?? '—' }}</span>
                                            <b>{{ Math.round(job.percent) }}%</b>
                                            <em
                                                v-if="
                                                    optsOf('downloads').downloadSpeed &&
                                                    job.speed !== null
                                                "
                                            >
                                                {{ formatBytes(job.speed) }}/s
                                            </em>
                                        </div>
                                        <div class="mbp-meter">
                                            <i :style="{ width: `${job.percent}%` }" />
                                        </div>
                                    </div>
                                </div>
                                <p v-else class="mbp-hint">{{ formatMessage(messages.noDownload) }}</p>
                            </div>
                        </section>

                        <!-- 服务器状态（手动地址） -->
                        <section
                            v-if="visibleWidgets.includes('server')"
                            class="mbp-card"
                            v-bind="cardProps('server')"
                        >
                            <div class="mbp-card-body">
                                <span class="mbp-label">{{ widgetLabel('server') }}</span>
                                <div class="mbp-server">
                                    <span class="mbp-server-name">{{ settings.server.name }}</span>
                                    <span class="mbp-server-state" :class="serverState">
                                        {{
                                            serverState === 'online'
                                                ? formatMessage(messages.serverOnline)
                                                : serverState === 'offline'
                                                  ? formatMessage(messages.serverOffline)
                                                  : formatMessage(messages.serverUnknown)
                                        }}
                                        <em
                                            v-if="
                                                optsOf('server').serverLatency && serverLatency !== null
                                            "
                                            class="mbp-latency"
                                        >
                                            {{ serverLatency }} ms
                                        </em>
                                        <em v-if="serverOnline !== null" class="mbp-latency">
                                            {{ serverOnline
                                            }}{{ serverMax !== null ? `/${serverMax}` : ''
                                            }}{{ ' ' }}{{ formatMessage(messages.players) }}
                                        </em>
                                    </span>
                                </div>
                            </div>
                        </section>
                        <!-- 缩放手柄：叠加在每张卡片右下角。放在画布层而不是卡片里，
                             这样 8 个卡片块不用各写一遍，位置也能跟着拖动实时走。 -->
                        <span
                            v-for="id in visibleWidgets"
                            :key="`rz-${id}`"
                            class="mbp-resize"
                            :style="resizeHandleStyle(id)"
                            @pointerdown.stop="onResizeStart($event, id)"
                        />
                    </div>
                </div>

                <!-- 右键菜单：删除 / 固定 / 组件设置（单独 Teleport，见 menuStyle 注释） -->
            </div>
        </Transition>
    </Teleport>

    <Teleport to="body">
        <div
            v-if="menu"
            class="mbp-menu is-context"
            data-magnetic-layer
            :style="menuStyle"
            @pointerdown.stop
        >
            <button type="button" class="mbp-menu-item" @click.stop="removeWidget(menu.id)">
                {{ formatMessage(messages.removeWidget) }}
            </button>
            <button type="button" class="mbp-menu-item" @click.stop="togglePin(menu.id)">
                {{
                    widgetPref(menu.id)?.pinned
                        ? formatMessage(messages.unpinWidget)
                        : formatMessage(messages.pinWidget)
                }}
            </button>
            <button type="button" class="mbp-menu-item" @click.stop="openSettingsFromMenu">
                {{ formatMessage(messages.widgetSettings) }}
            </button>
        </div>
    </Teleport>

    <!-- 组件设置弹窗：就地改，不跳页（主人明确要求） -->
    <Teleport to="body">
        <div
            v-if="settingsFor"
            class="mbp-sheet-mask"
            data-magnetic-layer
            @pointerdown.self="closeWidgetSettings"
        >
            <section
                class="mbp-sheet"
                role="dialog"
                :aria-label="formatMessage(messages.widgetPanelTitle)"
            >
                <header class="mbp-sheet-head">
                    <span class="mbp-sheet-title">
                        {{ widgetLabel(settingsFor) }}
                        <em>{{ formatMessage(messages.widgetPanelTitle) }}</em>
                    </span>
                    <button
                        type="button"
                        class="mbp-sheet-close"
                        :aria-label="formatMessage(messages.done)"
                        @click="closeWidgetSettings"
                    >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M6 6l12 12M18 6L6 18" />
                        </svg>
                    </button>
                </header>

                <div class="mbp-sheet-body">
                    <p class="mbp-sheet-label">{{ formatMessage(messages.commonTitle) }}</p>

                    <div class="mbp-sheet-row">
                        <span>{{ formatMessage(messages.widthLabel) }}</span>
                        <div class="mbp-seg">
                            <button
                                v-for="cols in WIDTH_OPTIONS"
                                :key="cols"
                                type="button"
                                :class="{
                                    active:
                                        (widgetPref(settingsFor)?.w ??
                                            MAGNETIC_WIDGET_DEFAULT_COLS[settingsFor]) === cols,
                                }"
                                @click="setWidgetWidth(settingsFor, cols)"
                            >
                                {{ cols }}
                            </button>
                        </div>
                    </div>

                    <button
                        type="button"
                        class="mbp-sheet-link"
                        @click="resetWidgetPosition(settingsFor)"
                    >
                        {{ formatMessage(messages.resetPosition) }}
                    </button>

                    <!-- 组件专属设置（数据驱动：见 OPTION_ROWS） -->
                    <template v-if="optionRows.length">
                        <p class="mbp-sheet-label">
                            {{ formatMessage(messages.widgetPanelTitle) }}
                        </p>
                        <template v-for="row in optionRows" :key="row.key">
                            <!-- 实例多选：单独一块，不能挤进一行 -->
                            <div v-if="row.kind === 'instances'" class="mbp-opt-group">
                                <p class="mbp-sheet-label">{{ formatMessage(row.label) }}</p>
                                <label
                                    v-for="instance in instances"
                                    :key="instance.id"
                                    class="mbp-opt"
                                >
                                    <span>{{ instance.name }}</span>
                                    <input
                                        type="checkbox"
                                        :checked="hasShortcutInstance(instance.id)"
                                        @change="toggleShortcutInstance(instance.id)"
                                    />
                                </label>
                                <p v-if="!instances.length" class="mbp-hint">—</p>
                            </div>

                            <label v-else class="mbp-opt">
                                <span>{{ formatMessage(row.label) }}</span>
                                <input
                                    v-if="row.kind === 'toggle'"
                                    type="checkbox"
                                    :checked="Boolean(optValue(row.key))"
                                    @change="toggleOption(row.key)"
                                />
                                <span v-else-if="row.kind === 'seg'" class="mbp-seg">
                                    <button
                                        v-for="option in row.options"
                                        :key="option.value"
                                        type="button"
                                        :class="{
                                            active: Number(optValue(row.key)) === option.value,
                                        }"
                                        @click="setOptionNumber(row.key, option.value)"
                                    >
                                        {{ segLabel(option.label) }}
                                    </button>
                                </span>
                            </label>
                        </template>
                    </template>

                    <template v-if="settingsFor === 'server'">
                        <p class="mbp-sheet-label">{{ widgetLabel('server') }}</p>
                        <label class="mbp-field">
                            <span>{{ formatMessage(messages.serverName) }}</span>
                            <input
                                :value="settings.server.name"
                                @input="
                                    setServerField(
                                        'name',
                                        ($event.target as HTMLInputElement).value,
                                    )
                                "
                            />
                        </label>
                        <label class="mbp-field">
                            <span>{{ formatMessage(messages.serverUrl) }}</span>
                            <input
                                :value="settings.server.url"
                                placeholder="play.example.com:25565"
                                @input="
                                    setServerField('url', ($event.target as HTMLInputElement).value)
                                "
                            />
                        </label>
                    </template>
                    <p v-if="!optionRows.length && settingsFor !== 'server'" class="mbp-hint">
                        {{ formatMessage(messages.noOptions) }}
                    </p>
                </div>

                <footer class="mbp-sheet-foot">
                    <button type="button" class="mbp-sheet-btn" @click="closeWidgetSettings">
                        {{ formatMessage(messages.done) }}
                    </button>
                </footer>
            </section>
        </div>
    </Teleport>
</template>

<style scoped>
.mb-panel {
    /* fixed + Teleport 到 body：避开条子的 transform，backdrop-filter 才真正生效 */
    position: fixed;
    /*
     * ⚠ 必须显式给 z-index。
     * 面板被 Teleport 到 body（z=auto），而整个应用内容区 `.app-contents` 是
     * `position:absolute; z-index:1` —— 不给 z-index 时面板**被压在页面内容下面**：
     * 视觉上还能透出来（页面是玻璃/透明的），但指针事件全被页面接走，
     * 于是「点卡片」被外层判成「点面板外面」，一按就收起，组件根本拖不动。
     * 取 400：高于内容区(1)，低于各种 Modal(10001+)。
     */
    z-index: 400;
    transform: translateX(-50%);
    width: min(var(--mbp-width, 720px), calc(100vw - 24px));
    max-height: min(58vh, 520px);
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.75rem;
    border-radius: 20px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-2) 100%),
        color-mix(in srgb, var(--surface-1) calc(var(--mbp-alpha, 0.55) * 100%), transparent);
    -webkit-backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(180%);
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(180%);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
    color: var(--text-0);
    overflow: hidden;
    /*
     * 面板自身作为容器：头部/快捷条的字号用 `cqw` 跟着面板宽度走。
     * ⚠ 容器单位不能在容器自身上用（会退回视口单位），所以这里只声明容器，
     * 真正的 `font-size: …cqw` 写在子孙（.mbp-head / .mbp-shortcuts）上。
     */
    container-type: inline-size;
    container-name: panel;
}

.mbp-head {
    display: flex;
    align-items: center;
    gap: 0.4em;
    /* 面板宽度 560→1080 时，头部字号 11→15px 连续变化（不再写死 0.8rem）。 */
    font-size: clamp(11px, 1.7cqw, 15px);
}

.mbp-search {
    flex: 1;
    height: 2.5em;
    padding: 0 0.85em;
    border-radius: 0.8em;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    color: var(--text-0);
    font: inherit;
    font-size: 0.92em;
    outline: none;
}

.mbp-search:focus-visible {
    border-color: var(--color-brand);
}

.mbp-icon-btn,
.mbp-mini,
.mbp-shortcut,
.mbp-icon-cell {
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    color: var(--text-0);
    cursor: pointer;
    transition:
        background var(--t-fast) var(--ease),
        border-color var(--t-fast) var(--ease);
}

.mbp-icon-btn:hover,
.mbp-mini:hover,
.mbp-shortcut:hover,
.mbp-icon-cell:hover {
    background: var(--glass-3);
    border-color: var(--color-brand);
}

.mbp-icon-btn {
    width: 2.5em;
    height: 2.5em;
    border-radius: 0.8em;
    display: grid;
    place-items: center;
    flex: none;
}

.mbp-icon-btn :deep(svg) {
    width: 1.25em;
    height: 1.25em;
    /* 这些是线框图标：没有 stroke 就什么都看不见（原来 + 按钮是空的）。 */
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
}

.mbp-shortcuts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35em;
    font-size: clamp(11px, 1.7cqw, 15px);
}

.mbp-shortcut {
    display: inline-flex;
    align-items: center;
    gap: 0.3em;
    height: 2.1em;
    padding: 0 0.6em;
    border-radius: 0.6em;
    font-size: 0.88em;
}

.mbp-shortcut-icon {
    width: 1.05em;
    height: 1.05em;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
}

/*
 * 自由画布：卡片绝对定位，坐标由 JS 算好塞进 inline style。
 * 高度由 canvasHeight 动态给；滚动在画布外层（.mbp-body）。
 */
/* 画布外层负责滚动：面板 max-height 有限，画布一旦比它高就必须能滚，
   否则拖到下面的卡片永远看不见（换成绝对定位时把原来的 overflow-y 弄丢了）。 */
.mbp-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-width: thin;
}

.mbp-canvas {
    position: relative;
    width: 100%;
    /* 拖动时不要选中文字（用户报「一滑就出现文字选择」） */
    user-select: none;
    -webkit-user-select: none;
}

.mbp-card {
    border-radius: 14px;
    border: 1px solid var(--stroke-1);
    /* 与全站一致：玻璃 + 模糊 + 发丝描边。不要用实心底去"买"可读性 —— 
       2026-11-14 主人明确要求遵循启动器风格统一原则。 */
    background: var(--glass-1);
    -webkit-backdrop-filter: blur(18px) saturate(160%);
    backdrop-filter: blur(18px) saturate(160%);
    /*
     * 自由摆放要求高度可算：画布多高、拖到哪算对齐都靠它。
     * 尺寸由 inline style 的 width/height 决定（拖右下角可改），内容自己滚动。
     */
    position: absolute;
    height: 148px;
    overflow: hidden;
    box-sizing: border-box;
    /*
     * 卡片是**尺寸容器**：内部的字号/间距/列数全部用 cqw・cqh + em 跟着实际大小走，
     * 这样 3 列的小卡和 12 列的大卡用的是同一套样式，不用为每个尺寸写死一份。
     * ⚠ 容器单位不能在容器自身上用，所以内边距与字号都写在 .mbp-card-body 里。
     */
    container-type: size;
    container-name: card;
    transition:
        border-color var(--t-fast) var(--ease),
        background var(--t-fast) var(--ease);
}

/*
 * 卡片内容层：真正消费容器单位的地方。
 * - 基准字号 = min(宽度 × 4.3%, 高度 × 8.4%)，夹在 9–15px：
 *   348×148 的常规卡≈12.4px（与改造前 0.66~0.86rem 的观感一致），
 *   172px 的窄卡自动落到 9px，12 列的大卡最高 15px 不会变成巨型字。
 * - 内边距同理走 cqw，窄卡收紧、大卡放宽。
 * - 其余内部尺寸一律写成 em，于是全部跟着这个基准字号联动。
 */
.mbp-card-body {
    display: flex;
    flex-direction: column;
    gap: 0.35em;
    height: 100%;
    box-sizing: border-box;
    padding: clamp(6px, 2.4cqw, 12px) clamp(7px, 2.8cqw, 13px);
    /*
     * 基准字号：宽 4.3% 与「宽 3.2% 与高 8.4% 中取大者」取小 ——
     * 高度只在卡片很矮时才真正压字号（110px 的矮卡也不会掉到 9px），
     * 夹在 9–15px 之间：348×148 的常规卡≈12.4px，和改造前的观感一致。
     */
    font-size: clamp(9px, min(4.3cqw, max(3.2cqw, 8.4cqh)), 15px);
    line-height: 1.25;
    /* 内容少时垂直居中，内容超出时安全退回顶部（safe：不裁掉第一行） */
    justify-content: safe center;
    overflow: auto;
    scrollbar-width: none;
    border-radius: inherit;
}

.mbp-card {
    cursor: grab;
}

.mbp-card.is-dragging {
    opacity: 0.92;
    z-index: 5;
    cursor: grabbing;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28);
    transform: scale(1.015);
    /* 拖动时不跟手抖：别给 left/top 加过渡 */
    transition: none;
}

.mbp-card.is-pinned {
    border-color: color-mix(in srgb, var(--color-brand) 35%, var(--stroke-1));
}

/* 悬停 2 秒后会吸附到这张卡上 */
.mbp-card.is-snap-target {
    border-color: var(--color-brand);
    background: color-mix(in srgb, var(--color-brand) 10%, transparent);
}

/* 自由摆放后不再有列跨度，宽度由 inline style 的 width 决定。 */

.mbp-label {
    font-size: 0.85em;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-text-tertiary);
}

.mbp-clock {
    font-size: 1.95em;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
}

.mbp-date {
    font-size: 1.1em;
    font-weight: 600;
}

/* 行列表吃掉剩余高度：卡片被拖高就多显示几行，不再写死 max-height: 120px */
.mbp-rows {
    display: flex;
    flex-direction: column;
    gap: 0.22em;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: none;
}

.mbp-row {
    display: flex;
    align-items: center;
    gap: 0.45em;
    font-size: 0.95em;
}

.mbp-row-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mbp-row-meta {
    color: var(--color-text-tertiary);
    font-size: 0.85em;
}

.mbp-mini {
    height: 1.9em;
    padding: 0 0.5em;
    border-radius: 0.6em;
    font-size: 0.86em;
    flex: none;
}

/* 列数不写死：宽度够就多摆几颗，窄了自动换行 */
.mbp-icon-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(2.4em, 1fr));
    gap: 0.3em;
    overflow-y: auto;
    min-height: 0;
}

.mbp-icon-cell {
    height: 2.5em;
    border-radius: 0.7em;
    display: grid;
    place-items: center;
}

.mbp-icon-cell :deep(svg) {
    width: 1.25em;
    height: 1.25em;
    /* @modrinth/assets 的图标是**线框**：填色会把形状糊成一团（看起来"图标不对"）。 */
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
}

.mbp-cal {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 0.12em;
    font-size: 0.85em;
    font-variant-numeric: tabular-nums;
    text-align: center;
}

/* 开「周数」时多一列（ISO 周号），整行一起排 */
.mbp-cal.has-weeks {
    grid-template-columns: 1.35em repeat(7, 1fr);
}

.mbp-cal-head {
    opacity: 0.55;
    font-size: 0.8em;
}

.mbp-cal-wk {
    opacity: 0.4;
    font-size: 0.76em;
}

.mbp-cal span {
    padding: 0.14em 0;
    border-radius: 5px;
    color: var(--color-text-secondary);
}

.mbp-cal span.today {
    background: color-mix(in srgb, var(--color-brand) 22%, transparent);
    color: var(--text-0);
    font-weight: 700;
}

.mbp-gauges {
    display: flex;
    flex-direction: column;
    gap: 0.3em;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: none;
    justify-content: safe center;
}

.mbp-gauge {
    display: grid;
    grid-template-columns: 3.1em 1fr 2.6em;
    align-items: center;
    gap: 0.45em;
    font-size: 0.9em;
}

.mbp-gauge b {
    text-align: right;
    font-variant-numeric: tabular-nums;
}

.mbp-meter {
    height: 0.42em;
    min-height: 3px;
    border-radius: 0.22em;
    background: var(--glass-2);
    overflow: hidden;
}

.mbp-meter i {
    display: block;
    height: 100%;
    border-radius: 0.22em;
    background: var(--color-brand);
    transition: width 320ms var(--ease-out);
}

.mbp-net {
    grid-column: span 2;
    font-size: 0.9em;
    color: var(--color-text-secondary);
    font-variant-numeric: tabular-nums;
}

.mbp-quota {
    display: flex;
    flex-direction: column;
    gap: 0.3em;
}

.mbp-quota-num {
    display: flex;
    align-items: baseline;
    gap: 0.3em;
}

.mbp-quota-num b {
    font-size: 1.7em;
    font-variant-numeric: tabular-nums;
}

.mbp-quota-num span {
    font-size: 0.88em;
    color: var(--color-text-tertiary);
}

.mbp-server {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.45em;
    font-size: 1em;
}

.mbp-server-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mbp-server-state {
    font-size: 0.86em;
    padding: 0.08em 0.5em;
    border-radius: 0.5em;
    border: 1px solid var(--stroke-1);
}

.mbp-server-state.online {
    color: var(--color-brand);
    border-color: color-mix(in srgb, var(--color-brand) 45%, transparent);
}

.mbp-server-state.offline {
    color: var(--color-red, #e05a5a);
}

.mbp-menu {
    position: absolute;
    right: 0.75rem;
    top: 3rem;
    min-width: 152px;
    max-height: 260px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 0.3rem;
    border-radius: 12px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-2) 100%),
        color-mix(in srgb, var(--surface-1) 72%, transparent);
    -webkit-backdrop-filter: blur(24px) saturate(180%);
    backdrop-filter: blur(24px) saturate(180%);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
    z-index: 5;
}

/* 右键菜单用视口坐标，所以要脱离面板的定位上下文 */
.mbp-menu.is-context {
    position: fixed;
    right: auto;
    top: auto;
    /* 单独 Teleport 到 body：要压过面板(z=400) */
    z-index: 500;
}

.mbp-menu-title {
    padding: 0.25rem 0.45rem 0.3rem;
    font-size: 0.66rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-text-tertiary);
}

.mbp-menu-item {
    text-align: left;
    padding: 0.35rem 0.45rem;
    border-radius: 8px;
    border: 0;
    background: transparent;
    color: var(--text-0);
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
}

.mbp-menu-item:hover {
    background: var(--glass-3);
}

.mbp-lyric {
    display: flex;
    align-items: center;
    gap: 0.5em;
    min-height: 0;
    flex: 1 1 auto;
    min-width: 0;
}

/* 文字列：原来 .mbp-lyric 自己就是纵向列，加了封面之后拆成两层 */
.mbp-lyric-text {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
}

.mbp-lyric strong {
    font-size: 1.1em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mbp-lyric span {
    font-size: 0.92em;
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mbp-hint,
.mbp-empty {
    margin: 0;
    font-size: 0.88em;
    color: var(--color-text-tertiary);
}

.mbp-empty {
    padding: 0.6em 0.2em;
}

.mbp-pop-enter-active,
.mbp-pop-leave-active {
    transition:
        opacity 180ms var(--ease-out),
        transform 200ms var(--ease-out);
}

.mbp-pop-enter-from,
.mbp-pop-leave-to {
    opacity: 0;
    transform: translateX(-50%) translateY(8px) scale(0.98);
}

/* ---------------- 组件设置弹窗 ---------------- */

/*
 * 弹窗遮罩：**不压暗，只模糊**（主人 2026-11-14 明确要求，与项目 NewModal
 * `.modal-overlay.standard` 同一口径：`background: transparent` + backdrop-filter）。
 * 之前那层 rgba(8,10,18,.28) 会把整屏压黑，和「自定义启动画面」弹窗完全不是一种观感。
 */
.mbp-sheet-mask {
    position: fixed;
    inset: 0;
    z-index: 600;
    display: grid;
    place-items: center;
    background: transparent;
    -webkit-backdrop-filter: blur(12px) saturate(150%);
    backdrop-filter: blur(12px) saturate(150%);
}

.mbp-sheet {
    width: min(340px, calc(100vw - 48px));
    max-height: min(70vh, 480px);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-2) 100%),
        color-mix(in srgb, var(--surface-1) 60%, transparent);
    -webkit-backdrop-filter: blur(28px) saturate(180%);
    backdrop-filter: blur(28px) saturate(180%);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.2);
    color: var(--text-0);
}

.mbp-sheet-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.7rem 0.75rem 0.5rem;
    border-bottom: 1px solid var(--stroke-1);
}

.mbp-sheet-title {
    display: flex;
    flex-direction: column;
    gap: 0.05rem;
    font-size: 0.82rem;
    font-weight: 600;
}

.mbp-sheet-title em {
    font-style: normal;
    font-size: 0.66rem;
    font-weight: 400;
    color: var(--color-text-tertiary);
}

.mbp-sheet-close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.mbp-sheet-close:hover {
    background: var(--glass-2);
    color: var(--text-0);
}

.mbp-sheet-close svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
}

.mbp-sheet-body {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.65rem 0.75rem;
    overflow-y: auto;
}

.mbp-sheet-label {
    margin: 0;
    font-size: 0.66rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-text-tertiary);
}

.mbp-sheet-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.76rem;
}

.mbp-seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 9px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
}

.mbp-seg button {
    min-width: 30px;
    padding: 0.18rem 0.32rem;
    border: none;
    border-radius: 7px;
    background: transparent;
    color: var(--color-text-secondary);
    font-size: 0.72rem;
    cursor: pointer;
}

.mbp-seg button:hover {
    background: var(--glass-2);
    color: var(--text-0);
}

.mbp-seg button.active {
    background: color-mix(in srgb, var(--color-brand) 18%, transparent);
    color: var(--text-0);
}

.mbp-sheet-link {
    align-self: flex-start;
    padding: 0.2rem 0;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--color-brand) 90%, white);
    font-size: 0.72rem;
    cursor: pointer;
}

.mbp-sheet-link:hover {
    text-decoration: underline;
}

.mbp-field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.74rem;
    color: var(--color-text-secondary);
}

.mbp-field input {
    width: 100%;
    padding: 0.32rem 0.45rem;
    border-radius: 8px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    color: var(--text-0);
    font-size: 0.74rem;
    user-select: text;
}

.mbp-sheet-foot {
    display: flex;
    justify-content: flex-end;
    padding: 0.55rem 0.75rem 0.7rem;
    border-top: 1px solid var(--stroke-1);
}

.mbp-sheet-btn {
    padding: 0.3rem 0.85rem;
    border-radius: 9px;
    border: 1px solid color-mix(in srgb, var(--color-brand) 45%, transparent);
    background: color-mix(in srgb, var(--color-brand) 10%, transparent);
    color: color-mix(in srgb, var(--color-brand) 95%, white);
    font-size: 0.76rem;
    cursor: pointer;
}

.mbp-sheet-btn:hover {
    background: color-mix(in srgb, var(--color-brand) 18%, transparent);
}

/* 输入框要能选中文字：画布的 user-select:none 不能传染到这里 */
.mbp-search,
.mbp-field input {
    user-select: text;
    -webkit-user-select: text;
}

/* 缩放手柄：平时很淡，鼠标进画布才明显；拖动时是 nwse 光标 */
.mbp-resize {
    position: absolute;
    z-index: 6;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
    opacity: 0.12;
    transition: opacity var(--t-fast) var(--ease);
}

.mbp-canvas:hover .mbp-resize {
    opacity: 0.55;
}

.mbp-resize:hover {
    opacity: 1;
}

.mbp-resize::after {
    content: '';
    position: absolute;
    right: 1px;
    bottom: 1px;
    width: 8px;
    height: 8px;
    border-right: 1.5px solid color-mix(in srgb, var(--color-brand) 85%, white);
    border-bottom: 1.5px solid color-mix(in srgb, var(--color-brand) 85%, white);
    border-bottom-right-radius: 3px;
}

/* 组件设置弹窗里的一行开关 */
.mbp-opt {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.76rem;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.mbp-opt input[type='checkbox'] {
    width: 15px;
    height: 15px;
    accent-color: var(--color-brand);
    cursor: pointer;
}

.mbp-lyric-cover {
    width: 3em;
    height: 3em;
    border-radius: 0.7em;
    object-fit: cover;
    flex-shrink: 0;
    background: var(--glass-2);
}

.mbp-latency {
    margin-left: 0.35em;
    font-size: 0.8em;
    font-style: normal;
    opacity: 0.72;
}

/* 下载卡片：多条任务 */
.mbp-jobs {
    display: flex;
    flex-direction: column;
    gap: 0.35em;
    overflow-y: auto;
    min-height: 0;
    flex: 1 1 auto;
}

.mbp-job {
    display: flex;
    flex-direction: column;
    gap: 0.18em;
}

.mbp-job-top {
    display: flex;
    align-items: baseline;
    gap: 0.4em;
    font-size: 0.84em;
    color: var(--color-text-secondary);
}

.mbp-job-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mbp-job-top b {
    color: var(--text-0);
    font-size: 0.9em;
}

.mbp-job-top em {
    font-style: normal;
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
}

.mbp-row-when {
    font-size: 0.78em;
    opacity: 0.6;
    white-space: nowrap;
}

/* 实例多选那一块 */
.mbp-opt-group {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    max-height: 168px;
    overflow-y: auto;
}

/* 命令面板：搜索框下方浮层 */
.mbp-cmdk {
    position: absolute;
    left: 0.75rem;
    right: 0.75rem;
    top: calc(100% - 0.15rem);
    z-index: 6;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 0.25rem;
    max-height: 260px;
    overflow-y: auto;
    border-radius: 12px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-2) 100%),
        color-mix(in srgb, var(--surface-1) 72%, transparent);
    -webkit-backdrop-filter: blur(24px) saturate(180%);
    backdrop-filter: blur(24px) saturate(180%);
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.18);
}

.mbp-cmdk-row {
    display: flex;
    align-items: center;
    gap: 0.5em;
    padding: 0.4em 0.55em;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--text-0);
    font: inherit;
    font-size: 0.92em;
    text-align: left;
    cursor: pointer;
}

.mbp-cmdk-row.is-active {
    background: color-mix(in srgb, var(--color-brand) 16%, transparent);
}

.mbp-cmdk-icon {
    width: 1em;
    height: 1em;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
}

.mbp-cmdk-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.mbp-cmdk-hint {
    font-size: 0.8em;
    color: var(--color-text-tertiary);
    flex: none;
}

/* 面板窄（分屏 / 小窗）时：快捷条只留图标，把宽度让给搜索框 */
@container panel (max-width: 620px) {
    .mbp-shortcut span {
        display: none;
    }

    .mbp-shortcut {
        padding: 0 0.5em;
    }
}

/* 键盘聚焦卡片时要看得见 */
.mbp-card:focus-visible {
    outline: 2px solid var(--color-brand);
    outline-offset: -2px;
}

/* ============================================================
 * 卡片自适应分档（容器查询，作用域 = 每张卡片自己的实际宽高）
 *
 * 这里只处理「布局怎么变」，字号/间距的连续缩放已经在 .mbp-card-body
 * 用 cqw・cqh 做掉了。分档只在光靠缩放不够的时候介入：
 * 窄到一定程度必须让次要信息退场，而不是把它压成一条省略号。
 * ============================================================ */

/* 极窄（≤150px，用户把卡拖到 1~2 列）：连标签都不要了，保住主内容 */
@container card (max-width: 150px) {
    .mbp-label {
        display: none;
    }

    .mbp-icon-grid {
        grid-template-columns: repeat(2, 1fr);
    }
}

/* 窄卡（≤230px）：版本号/上次游玩/周数这些次要列让位 */
@container card (max-width: 230px) {
    .mbp-row-meta,
    .mbp-row-when,
    .mbp-cal-wk,
    .mbp-cal-wk-head {
        display: none;
    }

    .mbp-cal.has-weeks {
        grid-template-columns: repeat(7, 1fr);
    }

    .mbp-icon-grid {
        grid-template-columns: repeat(3, 1fr);
    }

    /* 仪表盘在窄卡里改成「标签自适应宽度」，别用固定 3.1em 挤掉进度条 */
    .mbp-gauge {
        grid-template-columns: auto 1fr auto;
        gap: 0.35em;
    }
}

/* 矮卡（≤132px）：把上下内边距和间距收紧，高度全留给内容 */
@container card (max-height: 132px) {
    .mbp-card-body {
        gap: 0.22em;
        padding-top: 0.4em;
        padding-bottom: 0.4em;
    }
}

/* 宽卡（≥380px）：仪表盘两列并排，图标格自动多摆 */
@container card (min-width: 380px) {
    .mbp-gauges {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(10em, 1fr));
        align-content: center;
    }

    .mbp-icon-grid {
        grid-template-columns: repeat(auto-fit, minmax(2.6em, 1fr));
    }
}

/* 日历卡固定 6 行 × 7 列，比别的卡更吃高度：矮卡把行内边距再收一点，
   实测 348×148 时会溢出 4px，收掉这 4px 就不会出现滚动条。 */
@container card (max-height: 170px) {
    .mbp-cal span {
        padding: 0.06em 0;
    }
}

/* 高卡（≥220px，用户拖高）：行距与日历格一起放大，内容垂直居中 */
@container card (min-height: 220px) {
    .mbp-rows {
        gap: 0.4em;
    }

    .mbp-cal {
        align-content: center;
        height: 100%;
    }

    .mbp-cal span {
        padding: 0.3em 0;
    }
}</style>
