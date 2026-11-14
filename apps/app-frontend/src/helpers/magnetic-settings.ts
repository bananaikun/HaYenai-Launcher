/**
 * 磁悬条的配置：外观（不透明度 / 大小）+ 组件清单（含自由摆放坐标）+ 组件自己的小配置。
 *
 * 为什么存 localStorage 而不是这些的设置表：磁悬条在**每个页面**都会挂，
 * 读设置表要等 State 就绪（几百毫秒），期间条子只能先画默认值再跳变。
 * 这里的数据量很小、不需要跨设备同步，localStorage 同步读最省事。
 *
 * 布局口径（2026-11-14 主人拍板「完全自由摆放」）：
 * - `x` / `w` 用**列**表示（画布固定 12 列，渲染时按百分比折算）——
 *   这样面板宽度改了（560–1080）卡片不会变形。
 * - `y` 用**像素**（相对画布顶），自由摆放就是要能任意纵向错开。
 * - 三个字段都可缺省：缺省时由面板按「流式排布」补上并写回，之后一律按坐标走。
 */

/**
 * 组件 id 的**唯一真源**。
 *
 * 组件名（界面文案）不放在这里 —— 它要参与 i18n，见 `helpers/magnetic/labels.ts`。
 * 这里只留 id，供 `mergeWidgets` 判断「这个 id 还认识吗」。
 */
export const MAGNETIC_WIDGET_IDS = [
    'versions',
    'shortcuts',
    'clock',
    'date',
    'calendar',
    'system',
    'server',
    'lyrics',
    'downloads',
] as const

export type MagneticWidgetId = (typeof MAGNETIC_WIDGET_IDS)[number]

/** 磁悬条**条子上**可以轮播的页面（不是展开菜单里的卡片）。 */
export type MagneticBarPage = 'lyrics' | 'launch' | 'system'

export interface MagneticBarPagePref {
    id: MagneticBarPage
    enabled: boolean
}

/**
 * 各组件的**专属设置**（可选字段，用不到就留 undefined）。
 *
 * 口径：只放「一眼能看出区别」的开关，不做复杂配置面板。
 * 新增组件要加设置时，在这里加字段 + 在面板弹窗里加一行即可。
 */
export interface MagneticWidgetOptions {
    /** 时间：24 小时制（默认 true）。 */
    clock24h?: boolean
    /** 时间：显示秒。 */
    clockSeconds?: boolean
    /** 日期：显示星期。 */
    dateWeekday?: boolean
    /** 系统资源：显示 CPU。 */
    showCpu?: boolean
    /** 系统资源：显示内存。 */
    showMemory?: boolean
    /** 系统资源：显示网络。 */
    showNetwork?: boolean
    /** 版本实例：最多显示几条。 */
    instanceRows?: number
    /** 版本实例：显示「启动」按钮。 */
    instanceLaunch?: boolean
    /** 服务器状态：显示延迟（ms）。 */
    serverLatency?: boolean
    /** 正在播放：显示封面。 */
    lyricsCover?: boolean
    /** 快捷跳转：附加的实例 id（加进跳转格子里，双击启动）。 */
    shortcutInstances?: string[]
    /** 日历：每周从周一还是周日开始（1 = 周一）。 */
    calendarWeekStart?: number
    /** 日历：显示 ISO 周数。 */
    calendarWeekNumbers?: boolean
    /** 下载：最多显示几条任务。 */
    downloadRows?: number
    /** 下载：显示速度。 */
    downloadSpeed?: boolean
    /** 版本实例：显示上次游玩时间。 */
    instanceLastPlayed?: boolean
    /** 版本实例：只显示收藏的实例。 */
    instanceFavoritesOnly?: boolean
    /** 系统资源：刷新间隔（ms）。 */
    statsInterval?: number
    /** 系统资源：显示磁盘占用。 */
    showDisk?: boolean
    /** 服务器：探测超时（ms）。 */
    serverTimeout?: number
}

/** 默认值集中在这里，组件里不用到处写 `?? true`。 */
export const MAGNETIC_WIDGET_OPTION_DEFAULTS: Required<MagneticWidgetOptions> = {
    clock24h: true,
    clockSeconds: false,
    dateWeekday: true,
    showCpu: true,
    showMemory: true,
    showNetwork: true,
    instanceRows: 4,
    instanceLaunch: true,
    serverLatency: false,
    lyricsCover: true,
    shortcutInstances: [],
    calendarWeekStart: 1,
    calendarWeekNumbers: false,
    downloadRows: 2,
    downloadSpeed: true,
    instanceLastPlayed: false,
    instanceFavoritesOnly: false,
    statsInterval: 1000,
    showDisk: false,
    serverTimeout: 4000,
}

export interface MagneticWidgetPref {
    id: MagneticWidgetId
    enabled: boolean
    /** 固定后不能被拖动（右键菜单里切换）。 */
    pinned?: boolean
    /** 列起点 0–11。缺省 = 让面板按流式排布。 */
    x?: number
    /** 纵向位置（px，相对画布顶）。 */
    y?: number
    /** 占几列（1–12）。缺省 = 按组件类型的默认宽度。 */
    w?: number
    /** 高度（px）。缺省 = 默认 148。拖卡片右下角可改。 */
    h?: number
    /** 这张卡片自己的设置项（见 MagneticWidgetOptions）。 */
    opts?: MagneticWidgetOptions
}

export interface MagneticSettings {
    /** 整条磁悬条的开关。关掉 = 完全不渲染（不占位、不轮询）。 */
    barEnabled: boolean
    /** 背景不透明度 0–100。0 = 只剩模糊与描边。 */
    opacity: number
    /** 整体大小 80–120（百分比）。 */
    scale: number
    /** 面板宽度（px）。 */
    panelWidth: number
    /** 条子上页面的顺序与开关。 */
    barPages: MagneticBarPagePref[]
    widgets: MagneticWidgetPref[]
    /** 服务器状态：手动填地址，探 `GET <url>` 是否 200。 */
    server: { name: string; url: string; playersUrl?: string }
}

const STORAGE_KEY = 'hayenai.magnetic'
const CHANGE_EVENT = 'hayenai:magnetic-changed'

/** 画布固定 12 列。 */
export const MAGNETIC_COLUMNS = 12

/** 每个组件第一次摆上画布时占几列。 */
/**
 * 每个组件第一次摆上画布时占几列。
 *
 * 默认统一 6 列（一行两张）：3/4 列的小卡片并排放完会剩半行空档，
 * 而默认布局应该一眼看去是整齐的。想变小可以在「组件设置」里改成 3/4 列。
 */
export const MAGNETIC_WIDGET_DEFAULT_COLS: Record<MagneticWidgetId, number> = {
    versions: 12,
    shortcuts: 6,
    clock: 6,
    date: 6,
    calendar: 6,
    system: 6,
    server: 6,
    lyrics: 6,
    downloads: 6,
}

const DEFAULT_SETTINGS: MagneticSettings = {
    barEnabled: true,
    opacity: 55,
    scale: 100,
    panelWidth: 720,
    barPages: [
        { id: 'lyrics', enabled: true },
        { id: 'launch', enabled: true },
        { id: 'system', enabled: true },
    ],
    widgets: [
        { id: 'versions', enabled: true },
        { id: 'shortcuts', enabled: true },
        { id: 'clock', enabled: true },
        { id: 'date', enabled: true },
        { id: 'calendar', enabled: false },
        { id: 'system', enabled: true },
        { id: 'server', enabled: false },
        { id: 'lyrics', enabled: false },
        { id: 'downloads', enabled: false },
    ],
    server: { name: 'HaYenai', url: 'http://192.168.10.149:23443/api/health', playersUrl: '' },
}

export function readMagneticSettings(): MagneticSettings {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return structuredClone(DEFAULT_SETTINGS)
        const parsed = JSON.parse(raw) as Partial<MagneticSettings>
        const merged: MagneticSettings = {
            barEnabled: parsed.barEnabled !== false,
            opacity: clamp(Number(parsed.opacity ?? DEFAULT_SETTINGS.opacity), 0, 100),
            scale: clamp(Number(parsed.scale ?? DEFAULT_SETTINGS.scale), 80, 120),
            panelWidth: clamp(Number(parsed.panelWidth ?? DEFAULT_SETTINGS.panelWidth), 560, 1080),
            barPages: mergeBarPages(parsed.barPages),
            widgets: mergeWidgets(parsed.widgets),
            server: {
                name: parsed.server?.name ?? DEFAULT_SETTINGS.server.name,
                url: parsed.server?.url ?? DEFAULT_SETTINGS.server.url,
                playersUrl: parsed.server?.playersUrl ?? '',
            },
        }
        // 补齐过（老配置缺新组件）就写回一次 —— 否则「添加组件」里永远看不到新类型，
        // 因为 availableWidgets 是从内存里的 widgets 算的，而下次读盘又回到旧清单。
        if (
            (parsed.widgets?.length ?? 0) !== merged.widgets.length ||
            (parsed.barPages?.length ?? 0) !== merged.barPages.length
        ) {
            try {
                localStorage.setItem(STORAGE_KEY, JSON.stringify(merged))
            } catch {
                // 写不进去也不影响本次使用。
            }
        }
        return merged
    } catch {
        return structuredClone(DEFAULT_SETTINGS)
    }
}

export function writeMagneticSettings(settings: MagneticSettings): void {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(settings))
    } catch {
        // 配额满 / 隐私模式：宁可这条设置不生效，也不要抛错把界面打挂。
    }
    window.dispatchEvent(new CustomEvent(CHANGE_EVENT))
}

/** 订阅变更（同窗口的自定义事件 + 跨窗口的 storage）。返回取消订阅函数。 */
export function onMagneticSettingsChange(handler: () => void): () => void {
    window.addEventListener(CHANGE_EVENT, handler)
    window.addEventListener('storage', handler)
    return () => {
        window.removeEventListener(CHANGE_EVENT, handler)
        window.removeEventListener('storage', handler)
    }
}

/** 清掉所有坐标，让面板重新按流式排布（「恢复默认布局」按钮用）。 */
export function clearWidgetPositions(settings: MagneticSettings): void {
    for (const item of settings.widgets) {
        delete item.x
        delete item.y
        delete item.w
    }
}

export function moveWidget(
    widgets: MagneticWidgetPref[],
    from: number,
    to: number,
): MagneticWidgetPref[] {
    if (to < 0 || to >= widgets.length || from === to) return widgets
    const next = widgets.slice()
    const [item] = next.splice(from, 1)
    next.splice(to, 0, item)
    return next
}

const BAR_PAGE_IDS: MagneticBarPage[] = ['lyrics', 'launch', 'system']

function mergeBarPages(saved?: MagneticBarPagePref[]): MagneticBarPagePref[] {
    const seen = new Set<MagneticBarPage>()
    const result: MagneticBarPagePref[] = []
    for (const item of saved ?? []) {
        if (!BAR_PAGE_IDS.includes(item.id) || seen.has(item.id)) continue
        seen.add(item.id)
        result.push({ id: item.id, enabled: Boolean(item.enabled) })
    }
    for (const id of BAR_PAGE_IDS) {
        if (!seen.has(id)) result.push({ id, enabled: true })
    }
    // 全关掉时至少保留一页，否则条子上什么都不会显示。
    if (!result.some((item) => item.enabled)) result[0].enabled = true
    return result
}

function mergeWidgets(saved?: MagneticWidgetPref[]): MagneticWidgetPref[] {
    const known = MAGNETIC_WIDGET_IDS as readonly MagneticWidgetId[]
    if (!saved?.length) return structuredClone(DEFAULT_SETTINGS.widgets)

    const seen = new Set<MagneticWidgetId>()
    const result: MagneticWidgetPref[] = []
    for (const item of saved) {
        // 老配置里可能还有已经下线的组件（比如 DeepSeek 额度），这里直接丢掉。
        if (!known.includes(item.id) || seen.has(item.id)) continue
        seen.add(item.id)
        result.push({
            id: item.id,
            enabled: Boolean(item.enabled),
            pinned: Boolean(item.pinned),
            x: optionalNumber(item.x),
            y: optionalNumber(item.y),
            w: optionalNumber(item.w),
            h: optionalNumber(item.h),
            opts: item.opts && typeof item.opts === 'object' ? { ...item.opts } : undefined,
        })
    }
    // 老配置里没有的新组件，默认关闭地追加到末尾 —— 不偷偷打开用户没选过的卡片。
    for (const id of known) {
        if (!seen.has(id)) result.push({ id, enabled: false })
    }
    return result
}

function optionalNumber(value: unknown): number | undefined {
    const num = Number(value)
    return Number.isFinite(num) ? num : undefined
}

function clamp(value: number, min: number, max: number): number {
    if (!Number.isFinite(value)) return min
    return Math.min(max, Math.max(min, value))
}
