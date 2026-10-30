<script setup lang="ts">
/**
 * 「设置 → 外观 → 壁纸」这一块。
 *
 * 页面由三段组成，全部围绕**本地**壁纸包（不做服务端拉取）：
 *
 * 1. 当前壁纸卡片 —— 缩略图 + 名称 + 类型/体积徽标 + 「停用」；
 * 2. 导入（`.mpkg` / `.pkg` 文件，或松散工程目录 / 工坊目录）；
 * 3. 两个缩略图网格：`已导入` 与 `Steam 库`。
 *
 * 网格的观感照抄后台管理（`HaYenai-服务器端/src/styles/modules.css` 的
 * `.wall-grid` / `.wall-cell`）：4 列、10px 间距、16:10、选中 2px 边框 + 3px 光环、
 * hover 上浮 3px、底部黑渐变上 10px 白字标题。
 *
 * **与后台唯一的实质差别**是缩略图的加载策略。后台一次性把 65 张图的
 * `background-image` 全塞进 DOM；我们这 65 个包一共 1.8 GB，`preview.gif` 有的
 * 就 1 MB，全量拉会让设置页卡死。所以这里：
 * - 缩略图走 `<img loading="lazy">`，且 `src` **只在进入视口时才写**（IntersectionObserver）；
 * - 未进入视口时是 `--surface-3` 纯色（外加标题），看起来像「还没加载的卡片」；
 * - 每页最多渲染 60 项，底部「加载更多」——DOM 节点数不会随库增长而线性膨胀。
 *
 * 选中即生效 —— 配置写进 `wallpapers/config.json`（**不进 settings 表**），
 * 背景层读的是同一份反应式状态，所以不用重启、不用事件总线。
 *
 * 关于 ETC2：贴着「当前设备可能无法渲染」的徽标，但**允许导入、允许选中**。
 * 转码是二期的事（架构文档 §11.4），一期只做「告诉用户」。
 */
import { invoke } from '@tauri-apps/api/core'
import { TrashIcon } from '@modrinth/assets'
import {
    Button,
    ConfirmModal,
    defineMessages,
    injectNotificationManager,
    useVIntl,
} from '@modrinth/ui'
import { open } from '@tauri-apps/plugin-dialog'
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { OverflowMenu } from '@modrinth/ui'

import {
    canImportSteamWallpaper,
    formatWallpaperBytes,
    importSteamWallpaper,
    importWallpaperAndRefresh,
    probeWallpaperGpu,
    refreshWallpapers,
    removeWallpaperAndRefresh,
    scanSteamWallpapers,
    selectWallpaperAndRefresh,
    steamWallpaperPreviewUrl,
    steamWorkshopPageUrl,
    wallpaperAssetUrl,
    wallpaperErrorMessage,
    type WallpaperKind,
    type WallpaperMeta,
    wallpaperRenderability,
    wallpaperState,
    WALLPAPER_QUALITY_PRESETS,
    normalizeWallpaperQuality,
    setWallpaperConfig,
    type WallpaperAntiAliasing,
    type WallpaperParticles,
    type WallpaperPostProcessing,
    type WallpaperQualityPreset,
    type WallpaperSteamItem,
} from '@/helpers/wallpaper'
import {
    isRotationId,
    rotationState,
    setRotation,
    toggleRotationId,
} from '@/helpers/wallpaper-rotation'
import WallpaperPropertiesPanel from './WallpaperPropertiesPanel.vue'
import WallpaperRotationPicker from './WallpaperRotationPicker.vue'

import SettingsSection from './SettingsSection.vue'

const { formatMessage } = useVIntl()
const { addNotification } = injectNotificationManager()

/** 每页渲染多少张。60 正好是 4 列 × 15 行，翻页时不留半行。 */
const PAGE_SIZE = 60

const messages = defineMessages({
    title: { id: 'app.settings.wallpaper.title', defaultMessage: 'Wallpaper' },
    description: {
        id: 'app.settings.wallpaper.description',
        defaultMessage:
            'Play a Wallpaper Engine scene or video behind the launcher. Packages are read from this computer only.',
    },
    currentTitle: {
        id: 'app.settings.wallpaper.current-title',
        defaultMessage: 'Current wallpaper',
    },
    none: {
        id: 'app.settings.wallpaper.none',
        defaultMessage: 'No wallpaper in use — the launcher background stays as configured above.',
    },
    importFile: {
        id: 'app.settings.wallpaper.import-file',
        defaultMessage: 'Import package',
    },
    importDirectory: {
        id: 'app.settings.wallpaper.import-directory',
        defaultMessage: 'Import folder',
    },
    importing: {
        id: 'app.settings.wallpaper.importing',
        defaultMessage: 'Importing…',
    },
    importHint: {
        id: 'app.settings.wallpaper.import-hint',
        defaultMessage:
            'A .mpkg or .pkg package, an unpacked Wallpaper Engine project folder, or a Workshop folder all work.',
    },
    libraryTitle: {
        id: 'app.settings.wallpaper.library-title',
        defaultMessage: 'Imported wallpapers',
    },
    empty: {
        id: 'app.settings.wallpaper.empty',
        defaultMessage: 'Nothing imported yet.',
    },
    loading: {
        id: 'app.settings.wallpaper.loading',
        defaultMessage: 'Reading wallpapers…',
    },
    showing: {
        id: 'app.settings.wallpaper.showing',
        defaultMessage: 'Showing {shown} of {total}',
    },
    loadMore: {
        id: 'app.settings.wallpaper.load-more',
        defaultMessage: 'Load more',
    },
    use: { id: 'app.settings.wallpaper.use', defaultMessage: 'Use' },
    inUse: { id: 'app.settings.wallpaper.in-use', defaultMessage: 'In use' },
    disable: {
        id: 'app.settings.wallpaper.disable',
        defaultMessage: 'Stop using wallpaper',
    },
    delete: { id: 'app.settings.wallpaper.delete', defaultMessage: 'Delete' },
    deleteTitle: {
        id: 'app.settings.wallpaper.delete-title',
        defaultMessage: 'Delete this wallpaper?',
    },
    deleteDescription: {
        id: 'app.settings.wallpaper.delete-description',
        defaultMessage:
            'The package is removed from disk. You can import it again at any time, but anything stored in it (including its settings) is lost.',
    },
    unsupportedHint: {
        id: 'app.settings.wallpaper.unsupported-hint',
        defaultMessage: 'This wallpaper type is not supported yet.',
    },
    etc2Warning: {
        id: 'app.settings.wallpaper.etc2-warning',
        defaultMessage: 'May not render on this device',
    },
    etc2Info: {
        id: 'app.settings.wallpaper.etc2-info',
        defaultMessage: 'Uses ETC2 textures',
    },
    repairFailedTitle: {
        id: 'app.settings.wallpaper.repair-failed-title',
        defaultMessage: 'This package cannot be repaired',
    },
    repairFailedText: {
        id: 'app.settings.wallpaper.repair-failed-text',
        defaultMessage:
            '“{title}” has neither a loose scene nor an expandable scene.pkg — try importing it again.',
    },
    damaged: {
        id: 'app.settings.wallpaper.damaged',
        defaultMessage: 'Package damaged',
    },
    typeScene: { id: 'app.settings.wallpaper.type-scene', defaultMessage: 'Scene' },
    typeVideo: { id: 'app.settings.wallpaper.type-video', defaultMessage: 'Video' },
    typeWeb: { id: 'app.settings.wallpaper.type-web', defaultMessage: 'Web' },
    typeApplication: {
        id: 'app.settings.wallpaper.type-application',
        defaultMessage: 'Application',
    },
    typeOther: { id: 'app.settings.wallpaper.type-other', defaultMessage: 'Other' },
    actionFailedTitle: {
        id: 'app.settings.wallpaper.action-failed-title',
        defaultMessage: 'Wallpaper action failed',
    },
    actionFailedText: {
        id: 'app.settings.wallpaper.action-failed-text',
        defaultMessage: 'The launcher could not finish that wallpaper action.',
    },
    importedTitle: {
        id: 'app.settings.wallpaper.imported-title',
        defaultMessage: 'Wallpaper imported',
    },
    alreadyInstalled: {
        id: 'app.settings.wallpaper.already-installed',
        defaultMessage: 'That package is already in the library.',
    },
    removedTitle: {
        id: 'app.settings.wallpaper.removed-title',
        defaultMessage: 'Wallpaper deleted',
    },

    /* Steam 库 */
    steamTitle: {
        id: 'app.settings.wallpaper.steam.title',
        defaultMessage: 'Steam library',
    },
    steamDescription: {
        id: 'app.settings.wallpaper.steam.description',
        defaultMessage:
            'Lists the Wallpaper Engine wallpapers your Steam library already has on disk. Nothing is downloaded — importing copies a wallpaper into the library above.',
    },
    steamScan: {
        id: 'app.settings.wallpaper.steam.scan',
        defaultMessage: 'Scan Steam library',
    },
    steamScanning: {
        id: 'app.settings.wallpaper.steam.scanning',
        defaultMessage: 'Scanning…',
    },
    steamIdle: {
        id: 'app.settings.wallpaper.steam.idle',
        defaultMessage: 'Scan to list what this computer already has.',
    },
    steamSummary: {
        id: 'app.settings.wallpaper.steam.summary',
        defaultMessage: 'Found {total}, skipped {skipped}.',
    },
    steamSummaryClean: {
        id: 'app.settings.wallpaper.steam.summary-clean',
        defaultMessage: 'Found {total}.',
    },
    steamUnavailable: {
        id: 'app.settings.wallpaper.steam.unavailable',
        defaultMessage: 'Wallpaper Engine was not detected on this computer.',
    },
    steamEmptyTitle: {
        id: 'app.settings.wallpaper.steam.empty-title',
        defaultMessage: 'Nothing to import',
    },
    steamEmptyHint: {
        id: 'app.settings.wallpaper.steam.empty-hint',
        defaultMessage:
            'Subscribe to a wallpaper on Steam, let Wallpaper Engine finish downloading it, then scan again here.',
    },
    steamSkippedNotice: {
        id: 'app.settings.wallpaper.steam.skipped-notice',
        defaultMessage:
            '{skipped} folder(s) look like wallpapers but could not be read, so they are not listed.',
    },
    steamImport: {
        id: 'app.settings.wallpaper.steam.import',
        defaultMessage: 'Import',
    },
    steamImported: {
        id: 'app.settings.wallpaper.steam.imported',
        defaultMessage: 'Added',
    },
    steamImporting: {
        id: 'app.settings.wallpaper.steam.importing',
        defaultMessage: 'Importing…',
    },
    steamImportAll: {
        id: 'app.settings.wallpaper.steam.import-all',
        defaultMessage: 'Import all ({total})',
    },
    steamProgress: {
        id: 'app.settings.wallpaper.steam.progress',
        defaultMessage: '{done} of {total}',
    },
    steamCancel: {
        id: 'app.settings.wallpaper.steam.cancel',
        defaultMessage: 'Stop',
    },
    steamOpenInSteam: {
        id: 'app.settings.wallpaper.steam.open-in-steam',
        defaultMessage: 'Open in Steam',
    },
    steamStateOrphan: {
        id: 'app.settings.wallpaper.steam.state-orphan',
        defaultMessage: 'Folder is missing its project data',
    },
    steamStateMissing: {
        id: 'app.settings.wallpaper.steam.state-missing',
        defaultMessage: 'Not downloaded yet',
    },
    steamStateUnknown: {
        id: 'app.settings.wallpaper.steam.state-unknown',
        defaultMessage: "Steam's install record could not be read",
    },
    steamBatchTitle: {
        id: 'app.settings.wallpaper.steam.batch-title',
        defaultMessage: 'Steam import finished',
    },
    steamBatchSummary: {
        id: 'app.settings.wallpaper.steam.batch-summary',
        defaultMessage: 'Imported {done}, failed {failed}.',
    },
})

/** 设备能力探一次：徽标要区分「包用了 ETC2」与「这台机器画不出 ETC2」。 */
const caps = probeWallpaperGpu()

const rotationPicker = ref<InstanceType<typeof WallpaperRotationPicker>>()

const items = computed(() => wallpaperState.items)
const loaded = computed(() => wallpaperState.loaded)
const current = computed(
    () => items.value.find((item) => item.id === wallpaperState.config.selected) ?? null,
)

/** 正在导入 / 正在切换的包 id。用来禁用按钮，避免连点。 */
const importing = ref(false)
const switching = ref<string | null>(null)
const pendingDelete = ref<WallpaperMeta | null>(null)
const confirmDeleteModal = ref<InstanceType<typeof ConfirmModal> | null>(null)

const typeLabels: Record<WallpaperKind, string> = {
    scene: formatMessage(messages.typeScene),
    video: formatMessage(messages.typeVideo),
    web: formatMessage(messages.typeWeb),
    application: formatMessage(messages.typeApplication),
    other: formatMessage(messages.typeOther),
}

/**
 * 类型标签。Rust 侧的 `kind` 是**归一化后的字符串**而不是枚举，理论上可能冒出
 * 表里没有的值，所以这里按 `Record<string, string>` 查，查不到退回「其他」。
 */
function kindLabel(kind: string): string {
    const labels: Record<string, string> = typeLabels
    return labels[kind] ?? typeLabels.other
}

/* ------------------------------------------------------------------ *
 * 缩略图懒加载
 * ------------------------------------------------------------------ */

/**
 * 已经进入过视口的缩略图地址。
 *
 * 存的是 **URL** 而不是元素：分页追加 / 导入后刷新列表都会换掉 DOM 节点，
 * 但「这张图已经可以显示了」这件事按 URL 记才不会被反复撤销。
 */
const loadedThumbs = ref<Set<string>>(new Set())

/** 一个观察器管整页（两个网格共用）。`null` 表示这层 WebView 不支持。 */
let thumbObserver: IntersectionObserver | null = null
/** URL → 当前挂着的元素，用来在卸载时 unobserve。 */
const thumbNodes = new Map<string, HTMLElement>()

function markThumbLoaded(url: string): void {
    if (loadedThumbs.value.has(url)) return
    const next = new Set(loadedThumbs.value)
    next.add(url)
    loadedThumbs.value = next
}

function ensureThumbObserver(): IntersectionObserver | null {
    if (thumbObserver) return thumbObserver
    if (typeof IntersectionObserver === 'undefined') return null
    thumbObserver = new IntersectionObserver(
        (entries) => {
            for (const entry of entries) {
                if (!entry.isIntersecting) continue
                // 元素 → URL 的反查靠 dataset：`setRef` 只给元素，拿不到闭包里的那份。
                const url = (entry.target as HTMLElement).dataset.thumb
                if (url) {
                    thumbNodes.delete(url)
                    markThumbLoaded(url)
                }
                thumbObserver?.unobserve(entry.target)
            }
        },
        // 提前 240px 开始取，滚动时不会看到一片灰。
        { root: null, rootMargin: '240px 0px', threshold: 0.01 },
    )
    return thumbObserver
}

/** 每个 URL 一份稳定的 ref 回调 —— 函数身份不变，Vue 就不会反复卸载/重挂。 */
const thumbRefCache = new Map<string, (element: unknown) => void>()

/** 没有预览图时占位用的空回调。 `:ref` 只要拿到一个函数就够。 */
const noopThumbRef = (_element: unknown): void => {
    // 没有预览图 ⇒ 没有要观察的节点。
}

function makeThumbRef(url: string): (element: unknown) => void {
    return (element: unknown): void => {
        const node = element instanceof HTMLElement ? element : null
        if (!node) {
            const previous = thumbNodes.get(url)
            if (previous) {
                thumbObserver?.unobserve(previous)
                thumbNodes.delete(url)
            }
            return
        }
        if (loadedThumbs.value.has(url)) return
        const observer = ensureThumbObserver()
        if (!observer) {
            // 老 WebView（没有 IntersectionObserver）退回「直接全加载」：
            // 宁可慢一点，也不要整页空白。
            markThumbLoaded(url)
            return
        }
        thumbNodes.set(url, node)
        observer.observe(node)
    }
}

/** 模板里的 `:ref` 入口。 */
function thumbRef(url: string | null): (element: unknown) => void {
    if (!url) return noopThumbRef
    const cached = thumbRefCache.get(url)
    if (cached) return cached
    const created = makeThumbRef(url)
    thumbRefCache.set(url, created)
    return created
}

/** 只有进了视口才给 `src`；否则 `undefined`（`<img>` 不发起请求）。 */
function thumbSrc(url: string | null): string | undefined {
    if (!url) return undefined
    return loadedThumbs.value.has(url) ? url : undefined
}

/* ------------------------------------------------------------------ *
 * 已导入库：分页 + 展示工具
 * ------------------------------------------------------------------ */

const libraryLimit = ref(PAGE_SIZE)
const libraryVisible = computed(() => items.value.slice(0, libraryLimit.value))
const libraryRemaining = computed(() => Math.max(0, items.value.length - libraryLimit.value))

function loadMoreLibrary(): void {
    libraryLimit.value += PAGE_SIZE
}

/** 缩略图地址：包内 preview 走 `hayenai-we` 协议，不走 asset 协议（scope 到不了子目录）。 */
function previewUrl(item: WallpaperMeta): string | null {
    if (!item.preview) return null
    return wallpaperAssetUrl(item.id, item.preview)
}

/** 名字首字母，没有预览图时当占位。 */
function initialOf(title: string): string {
    return (title.trim()[0] ?? '?').toUpperCase()
}

function isSelected(item: WallpaperMeta): boolean {
    return wallpaperState.config.selected === item.id
}

/* ---------------- 壁纸画质（2026-10-30 20:20） ----------------
   WE 客户端「品质」面板里的项：帧率 / 抗锯齿 / 后处理 / 纹理分辨率是 webwallgl
   真能消费的；**阴影 / 体积照明 / 反射**是 WE 的 3D 场景特性，2D 图层合成做不到，
   所以这里不摆假开关。 */
const quality = computed(() => wallpaperState.config.quality)

const qualityPresets: { id: WallpaperQualityPreset; label: string }[] = [
    { id: 'low', label: '低' },
    { id: 'medium', label: '中' },
    { id: 'high', label: '高' },
    { id: 'ultra', label: '超高' },
]

function onFpsInput(event: Event): void {
    applyQuality({ fps: Number((event.target as HTMLInputElement).value) })
}

function onScaleInput(event: Event): void {
    applyQuality({ textureScale: Number((event.target as HTMLInputElement).value) })
}

function applyQuality(patch: Partial<typeof quality.value>): void {
    wallpaperState.config = {
        ...wallpaperState.config,
        quality: normalizeWallpaperQuality({ ...quality.value, ...patch }),
    }
    void setWallpaperConfig(wallpaperState.config).catch((error) => {
        console.warn('[wallpaper] 保存画质失败', error)
    })
}

function applyPreset(preset: WallpaperQualityPreset): void {
    wallpaperState.config = {
        ...wallpaperState.config,
        quality: { ...WALLPAPER_QUALITY_PRESETS[preset] },
    }
    void setWallpaperConfig(wallpaperState.config).catch(() => undefined)
}

/** 当前最接近哪个预设（用于高亮）。 */
const activePreset = computed<WallpaperQualityPreset | null>(() => {
    const q = quality.value
    for (const key of Object.keys(WALLPAPER_QUALITY_PRESETS) as WallpaperQualityPreset[]) {
        const p = WALLPAPER_QUALITY_PRESETS[key]
        if (
            p.fps === q.fps &&
            p.postProcessing === q.postProcessing &&
            p.antiAliasing === q.antiAliasing &&
            p.particles === q.particles &&
            p.textureScale === q.textureScale
        ) {
            return key
        }
    }
    return null
})

const AA_OPTIONS: { id: WallpaperAntiAliasing; label: string }[] = [
    { id: 'off', label: '关闭' },
    { id: 'fxaa', label: 'FXAA' },
    { id: 'msaa2', label: 'MSAA ×2' },
    { id: 'msaa4', label: 'MSAA ×4' },
]

const POST_OPTIONS: { id: WallpaperPostProcessing; label: string }[] = [
    { id: 'off', label: '关闭' },
    { id: 'low', label: '低' },
    { id: 'medium', label: '中' },
    { id: 'high', label: '启用（高）' },
]

const PARTICLE_OPTIONS: { id: WallpaperParticles; label: string }[] = [
    { id: 'off', label: '关闭' },
    { id: 'low', label: '低' },
    { id: 'medium', label: '中' },
    { id: 'high', label: '高' },
]

const aaLabel = computed(() => AA_OPTIONS.find((o) => o.id === quality.value.antiAliasing)?.label ?? '关闭')
const postLabel = computed(() => POST_OPTIONS.find((o) => o.id === quality.value.postProcessing)?.label ?? '启用（高）')
const particleLabel = computed(() => PARTICLE_OPTIONS.find((o) => o.id === quality.value.particles)?.label ?? '高')

function aaMenu() {
    return AA_OPTIONS.map((o) => ({
        id: o.id,
        label: o.label,
        disabled: quality.value.antiAliasing === o.id,
        action: () => applyQuality({ antiAliasing: o.id }),
    }))
}
function postMenu() {
    return POST_OPTIONS.map((o) => ({
        id: o.id,
        label: o.label,
        disabled: quality.value.postProcessing === o.id,
        action: () => applyQuality({ postProcessing: o.id }),
    }))
}
function particleMenu() {
    return PARTICLE_OPTIONS.map((o) => ({
        id: o.id,
        label: o.label,
        disabled: quality.value.particles === o.id,
        action: () => applyQuality({ particles: o.id }),
    }))
}

/** 轮播模式的下拉（用启动器自己的 OverflowMenu，和首页小组件设置一致）。 */
const rotationModeOptions = computed(() => [
    { id: 'off', label: '关闭', disabled: rotationState.mode === 'off', action: () => setRotation({ mode: 'off' as const }) },
    { id: 'sequential', label: '按顺序', disabled: rotationState.mode === 'sequential', action: () => setRotation({ mode: 'sequential' as const }) },
    { id: 'random', label: '随机', disabled: rotationState.mode === 'random', action: () => setRotation({ mode: 'random' as const }) },
])

const rotationModeLabel = computed(() =>
    rotationState.mode === 'sequential' ? '按顺序' : rotationState.mode === 'random' ? '随机' : '关闭',
)

/** 当前正在使用的那张（可调项面板只针对它）。 */
const currentWallpaper = computed<WallpaperMeta | null>(() => {
    const id = wallpaperState.config.selected
    if (!id) return null
    return items.value.find((item) => item.id === id) ?? null
})

/** 第一期只画 scene / video；损坏的包不提供「使用」。 */
function canUse(item: WallpaperMeta): boolean {
    if (item.damaged) return false
    return wallpaperRenderability(item, caps) !== 'unsupported'
}

/** 卡片上的警示/信息徽标。`null` = 不显示。 */
function auditBadge(item: WallpaperMeta): { text: string; serious: boolean } | null {
    const audit = item.texAudit
    if (!audit.etc2 && !audit.bptc) return null
    const needsGpu = (audit.etc2 && !caps.etc2) || (audit.bptc && !caps.bptc)
    if (needsGpu) return { text: formatMessage(messages.etc2Warning), serious: true }
    return { text: formatMessage(messages.etc2Info), serious: false }
}

type TileTone = 'danger' | 'warn' | 'info'

interface TileFlag {
    /** 完整说明，只出现在 `title` 里 —— 缩略图上放不下那么多字。 */
    text: string
    tone: TileTone
}

/**
 * 缩略图左上角的小圆点。
 *
 * 后台管理没有这套东西（那边不显示包的健康状况），但我们需要「坏了 / 画不出 /
 * 贴图特殊」这三个信息。用 6px 圆点 + `title` 而不是文字徽标，是为了不把 16:10
 * 的缩略图挤成一张贴满标签的卡片；完整文案在悬停提示与「当前壁纸」卡片里。
 */
function tileFlags(item: WallpaperMeta): TileFlag[] {
    const flags: TileFlag[] = []
    if (item.damaged) {
        flags.push({ text: formatMessage(messages.damaged), tone: 'danger' })
    } else if (!canUse(item)) {
        flags.push({ text: formatMessage(messages.unsupportedHint), tone: 'warn' })
    }
    const audit = auditBadge(item)
    if (audit) flags.push({ text: audit.text, tone: audit.serious ? 'warn' : 'info' })
    return flags
}

/** 悬停提示：标题 + 类型 + 体积 + 所有徽标文案（后台管理也这么拼）。 */
function tileTooltip(item: WallpaperMeta): string {
    const parts = [item.title, kindLabel(item.type), formatWallpaperBytes(item.bytes)]
    for (const flag of tileFlags(item)) parts.push(flag.text)
    return parts.join(' · ')
}

/* ------------------------------------------------------------------ *
 * Steam 库
 * ------------------------------------------------------------------ */

/** 页面区块的四种样子。`unavailable` 既包括「没装 WE」也包括「命令报错」。 */
type SteamPhase = 'idle' | 'scanning' | 'ready' | 'empty' | 'unavailable'

const steamPhase = ref<SteamPhase>('idle')
const steamItems = ref<WallpaperSteamItem[]>([])
const steamSkipped = ref(0)
/** 被跳过的原因（Rust 侧的中文单句），只用来做那条提示的 tooltip。 */
const steamSkippedReasons = ref<string[]>([])
/** Rust 侧的原话（中文单句）；有就优先显示，没有才用 i18n 兜底。 */
const steamErrorText = ref<string | null>(null)
const steamLimit = ref(PAGE_SIZE)

const steamVisible = computed(() => steamItems.value.slice(0, steamLimit.value))
const steamRemaining = computed(() => Math.max(0, steamItems.value.length - steamLimit.value))
const steamImportable = computed(() =>
    steamItems.value.filter((item) => canImportSteamWallpaper(item)),
)

/** 单个导入中 / 批量导入中，两个互斥的忙碌态。 */
const steamBusyId = ref<string | null>(null)
const batch = reactive({ running: false, done: 0, failed: 0, total: 0 })
/** 「停止」按的是**队列**：当前这一项会做完，避免半截文件留在磁盘上。 */
let batchCancelled = false

const batchFinished = computed(() => batch.done + batch.failed)
const batchPercent = computed(() => {
    if (batch.total <= 0) return '0%'
    const ratio = Math.min(1, batchFinished.value / batch.total)
    return `${Math.round(ratio * 100)}%`
})
const steamBusy = computed(() => batch.running || steamBusyId.value !== null)

const steamStatusText = computed<string>(() => {
    switch (steamPhase.value) {
        case 'scanning':
            return formatMessage(messages.steamScanning)
        case 'unavailable':
            return steamErrorText.value ?? formatMessage(messages.steamUnavailable)
        case 'ready':
            return steamSkipped.value > 0
                ? formatMessage(messages.steamSummary, {
                      total: steamItems.value.length,
                      skipped: steamSkipped.value,
                  })
                : formatMessage(messages.steamSummaryClean, { total: steamItems.value.length })
        default:
            return formatMessage(messages.steamIdle)
    }
})

/** 空态标题：「没东西可导」与「扫不动」是两句不同的话。 */
const steamEmptyHeading = computed<string>(() =>
    steamPhase.value === 'unavailable'
        ? (steamErrorText.value ?? formatMessage(messages.steamUnavailable))
        : formatMessage(messages.steamEmptyTitle),
)

function steamPreview(item: WallpaperSteamItem): string | null {
    return steamWallpaperPreviewUrl(item.workshopId, item.preview)
}

/** 「目录不完整」的说明；完整的条目返回 `null`。 */
function steamStateLabel(item: WallpaperSteamItem): string | null {
    if (item.itemState === 'orphan') return formatMessage(messages.steamStateOrphan)
    if (item.itemState === 'missing') return formatMessage(messages.steamStateMissing)
    // Rust 侧的第三种取值：`appworkshop_431960.acf` 读不到，装没装说不准。
    if (item.itemState === 'unknown') return formatMessage(messages.steamStateUnknown)
    return null
}

/** 跳过提示的 tooltip：把 Rust 给的原因串起来（可能为空 ⇒ 不给 title）。 */
const steamSkippedTooltip = computed<string | undefined>(() =>
    steamSkippedReasons.value.length > 0 ? steamSkippedReasons.value.join('\n') : undefined,
)

function steamTooltip(item: WallpaperSteamItem): string {
    const parts = [item.title, kindLabel(item.kind)]
    if (item.bytes > 0) parts.push(formatWallpaperBytes(item.bytes))
    const state = steamStateLabel(item)
    if (state) parts.push(state)
    if (item.importedId !== null) parts.push(formatMessage(messages.steamImported))
    return parts.join(' · ')
}

/** 已导入的排后面（没什么可做的），其余按标题，保证顺序稳定。 */
function compareSteamItems(left: WallpaperSteamItem, right: WallpaperSteamItem): number {
    const leftDone = left.importedId !== null ? 1 : 0
    const rightDone = right.importedId !== null ? 1 : 0
    if (leftDone !== rightDone) return leftDone - rightDone
    return left.title.localeCompare(right.title)
}

function loadMoreSteam(): void {
    steamLimit.value += PAGE_SIZE
}

/** 扫一遍工坊目录。失败时把 Rust 的原话留在状态行里，不当错误弹窗。 */
async function scanSteam(): Promise<void> {
    if (steamBusy.value) return
    steamPhase.value = 'scanning'
    steamErrorText.value = null
    try {
        const result = await scanSteamWallpapers()
        steamItems.value = [...result.items].sort(compareSteamItems)
        steamSkipped.value = result.skipped
        steamSkippedReasons.value = result.skippedReasons
        steamLimit.value = PAGE_SIZE
        steamPhase.value = result.items.length > 0 ? 'ready' : 'empty'
    } catch (error) {
        console.warn('[wallpaper] Steam 扫描失败', error)
        steamItems.value = []
        steamSkipped.value = 0
        steamSkippedReasons.value = []
        steamErrorText.value = wallpaperErrorMessage(error)
        steamPhase.value = 'unavailable'
    }
}

/** 整块缩略图即「在 Steam 中打开」—— 订了但没下载完的也能点回去处理。 */
async function openSteamPage(item: WallpaperSteamItem): Promise<void> {
    try {
        await openUrl(steamWorkshopPageUrl(item.workshopId))
    } catch (error) {
        console.warn('[wallpaper] 打开 Steam 失败', error)
        notifyFailure(error)
    }
}

/**
 * 把「这个工坊条目已经进了库」写回内存里的扫描结果。
 *
 * 不重新扫一遍工坊目录：那是 65 个目录 + 1.8 GB 的元数据读取。真正需要重读的
 * 是**已导入库**，批量结束时统一刷。
 */
function applySteamImport(workshopId: string, importedId: string): void {
    steamItems.value = steamItems.value.map((entry) =>
        entry.workshopId === workshopId ? { ...entry, importedId } : entry,
    )
}

async function importOneSteam(item: WallpaperSteamItem): Promise<void> {
    if (steamBusy.value || !canImportSteamWallpaper(item)) return
    steamBusyId.value = item.workshopId
    try {
        const outcome = await importSteamWallpaper(item.workshopId)
        applySteamImport(item.workshopId, outcome.meta.id)
        addNotification({
            title: formatMessage(messages.importedTitle),
            text: outcome.alreadyInstalled
                ? formatMessage(messages.alreadyInstalled)
                : outcome.meta.title,
            type: 'success',
        })
        for (const warning of outcome.warnings) {
            addNotification({ title: outcome.meta.title, text: warning, type: 'warning' })
        }
        await refreshWallpapers()
    } catch (error) {
        console.warn('[wallpaper] Steam 导入失败', error)
        notifyFailure(error)
    } finally {
        steamBusyId.value = null
    }
}

/**
 * 批量导入。
 *
 * **串行**是刻意的：65 个包要逐个解包 + 拷文件，并发只会让磁盘寻道互相打断
 * （架构文档 §11.2 里记过一次实测）。每完成一项更新一次进度，界面才有 `3/65`。
 */
async function importAllSteam(): Promise<void> {
    if (steamBusy.value) return
    const queue = [...steamImportable.value]
    if (queue.length === 0) return

    batchCancelled = false
    batch.running = true
    batch.done = 0
    batch.failed = 0
    batch.total = queue.length

    for (const item of queue) {
        if (batchCancelled) break
        try {
            const outcome = await importSteamWallpaper(item.workshopId)
            applySteamImport(item.workshopId, outcome.meta.id)
            batch.done += 1
        } catch (error) {
            console.warn('[wallpaper] Steam 批量导入失败', item.workshopId, error)
            batch.failed += 1
        }
    }

    batch.running = false
    await refreshWallpapers()
    addNotification({
        title: formatMessage(messages.steamBatchTitle),
        text: formatMessage(messages.steamBatchSummary, {
            done: batch.done,
            failed: batch.failed,
        }),
        type: batch.failed > 0 ? 'warning' : 'success',
    })
}

/** 只停队列，不打断手里这一项。 */
function cancelSteamBatch(): void {
    batchCancelled = true
}

/* ------------------------------------------------------------------ *
 * 手动导入 / 选中 / 删除
 * ------------------------------------------------------------------ */

/** 修复失败（包里连 scene.pkg 都没有）：说清楚原因，别让按钮静默失效。 */
function notifyRepairFailed(item: WallpaperMeta): void {
    addNotification({
        title: formatMessage(messages.repairFailedTitle),
        text: formatMessage(messages.repairFailedText, { title: item.title }),
        type: 'error',
    })
}

function notifyFailure(error: unknown): void {
    addNotification({
        title: formatMessage(messages.actionFailedTitle),
        text: wallpaperErrorMessage(error) ?? formatMessage(messages.actionFailedText),
        type: 'error',
    })
}

/** 导入一个路径。失败只提示一句，不打断页面。 */
async function runImport(path: string): Promise<void> {
    if (importing.value) return
    importing.value = true
    try {
        const outcome = await importWallpaperAndRefresh(path)
        addNotification({
            title: formatMessage(messages.importedTitle),
            text: outcome.alreadyInstalled
                ? formatMessage(messages.alreadyInstalled)
                : outcome.meta.title,
            type: 'success',
        })
        // Rust 侧给的中文单句：ETC2 贴图 / 暂不支持 / 入口文件缺失。
        for (const warning of outcome.warnings) {
            addNotification({ title: outcome.meta.title, text: warning, type: 'warning' })
        }
    } catch (error) {
        console.warn('[wallpaper] 导入失败', error)
        notifyFailure(error)
    } finally {
        importing.value = false
    }
}

/** `.mpkg` / `.pkg` 文件。 */
async function pickFile(): Promise<void> {
    try {
        const picked = await open({
            multiple: false,
            directory: false,
            filters: [{ name: 'Wallpaper Engine', extensions: ['mpkg', 'pkg'] }],
        })
        if (typeof picked === 'string') await runImport(picked)
    } catch (error) {
        notifyFailure(error)
    }
}

/** 目录：松散工程（有 project.json）或工坊目录（带 scene.pkg）。 */
async function pickDirectory(): Promise<void> {
    try {
        const picked = await open({ multiple: false, directory: true })
        if (typeof picked === 'string') await runImport(picked)
    } catch (error) {
        notifyFailure(error)
    }
}

/** 选中即生效：写 config.json → 共享状态变化 → 背景层立刻换（不重启）。 */
async function use(item: WallpaperMeta): Promise<void> {
    if (isSelected(item) || switching.value) return
    if (!canUse(item) && !item.damaged) return
    switching.value = item.id
    try {
        // 早期版本导入的工坊目录是 `scene.pkg` 容器形态：`project.json` 指向的 scene.json
        // 并不存在，于是被判 damaged、按钮禁用、点了也没反应。这里先就地展开再选中。
        if (item.damaged) {
            const repaired = await invoke<boolean>('plugin:wallpaper|wallpaper_repair', {
                id: item.id,
            })
            if (!repaired) {
                notifyRepairFailed(item)
                return
            }
            // 不重读一次列表，卡片上的红点会留到下次进页面 —— 看着像「没修好」。
            await refreshWallpapers()
        }
        await selectWallpaperAndRefresh(item.id)
    } catch (error) {
        console.warn('[wallpaper] 切换失败', error)
        notifyFailure(error)
    } finally {
        switching.value = null
    }
}

async function disable(): Promise<void> {
    if (switching.value) return
    switching.value = 'none'
    try {
        await selectWallpaperAndRefresh(null)
    } catch (error) {
        notifyFailure(error)
    } finally {
        switching.value = null
    }
}

function askDelete(item: WallpaperMeta): void {
    pendingDelete.value = item
    confirmDeleteModal.value?.show()
}

/** 用户取消 / 关掉了确认框：把待删项丢掉，免得下一次误删。 */
function onDeleteDialogHidden(): void {
    pendingDelete.value = null
}

async function confirmDelete(): Promise<void> {
    const target = pendingDelete.value
    pendingDelete.value = null
    if (!target) return
    try {
        await removeWallpaperAndRefresh(target.id)
        addNotification({
            title: formatMessage(messages.removedTitle),
            text: target.title,
            type: 'success',
        })
    } catch (error) {
        console.warn('[wallpaper] 删除失败', error)
        notifyFailure(error)
    }
}

onMounted(() => {
    // 设置页可能比背景层先进来（直接深链到 /settings）：这里再拉一次，
    // `refreshWallpapers` 会把并发调用合并成一次。
    void refreshWallpapers()
})

onBeforeUnmount(() => {
    thumbObserver?.disconnect()
    thumbObserver = null
    thumbNodes.clear()
    thumbRefCache.clear()
})
</script>

<template>
    <SettingsSection
        :title="formatMessage(messages.title)"
        :description="formatMessage(messages.description)"
        title-id="settings-target-appearance-wallpaper"
    >
        <div class="flex flex-col gap-6 p-4">
            <!-- 当前壁纸 -->
            <div class="flex flex-col gap-2">
                <h3 class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                    {{ formatMessage(messages.currentTitle) }}
                </h3>
                <div
                    v-if="current"
                    class="flex items-center gap-3 rounded-lg border border-solid border-surface-4 bg-surface-2 p-3"
                >
                    <div
                        class="flex size-16 shrink-0 items-center justify-center overflow-hidden rounded-md bg-surface-3"
                    >
                        <img
                            v-if="previewUrl(current)"
                            :src="previewUrl(current) ?? undefined"
                            alt=""
                            class="size-full object-cover"
                            draggable="false"
                        />
                        <span
                            v-else
                            class="text-lg font-semibold text-[var(--color-text-tertiary)]"
                        >
                            {{ initialOf(current.title) }}
                        </span>
                    </div>
                    <div class="min-w-0 flex-1">
                        <p
                            class="m-0 truncate text-sm font-semibold text-[var(--color-text-primary)]"
                            :title="current.title"
                        >
                            {{ current.title }}
                        </p>
                        <div class="mt-1 flex flex-wrap items-center gap-1.5">
                            <span class="wp-badge">{{ kindLabel(current.type) }}</span>
                            <span class="wp-badge">{{ formatWallpaperBytes(current.bytes) }}</span>
                            <span
                                v-if="auditBadge(current)"
                                class="wp-badge"
                                :class="{ 'is-serious': auditBadge(current)?.serious }"
                            >
                                {{ auditBadge(current)?.text }}
                            </span>
                            <span v-if="current.damaged" class="wp-badge is-serious">
                                {{ formatMessage(messages.damaged) }}
                            </span>
                        </div>
                    </div>
                    <Button :disabled="switching !== null" class="!shadow-none" @click="disable">
                        {{ formatMessage(messages.disable) }}
                    </Button>
                </div>
                <p v-else class="m-0 text-sm text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.none) }}
                </p>
            </div>

            <!-- 导入 -->
            <div class="flex flex-col gap-2">
                <div class="flex flex-wrap items-center gap-2">
                    <Button :disabled="importing" @click="pickFile">
                        {{ formatMessage(messages.importFile) }}
                    </Button>
                    <Button :disabled="importing" @click="pickDirectory">
                        {{ formatMessage(messages.importDirectory) }}
                    </Button>
                    <span v-if="importing" class="text-sm text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.importing) }}
                    </span>
                </div>
                <p class="m-0 text-xs leading-relaxed text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.importHint) }}
                </p>
            </div>

            <!-- 已导入：缩略图网格 -->
            <div class="flex flex-col gap-2">
                <div class="flex flex-wrap items-baseline justify-between gap-2">
                    <h3 class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                        {{ formatMessage(messages.libraryTitle) }}
                    </h3>
                    <span
                        v-if="loaded && items.length > 0"
                        class="text-xs tabular-nums text-[var(--color-text-tertiary)]"
                    >
                        {{
                            formatMessage(messages.showing, {
                                shown: libraryVisible.length,
                                total: items.length,
                            })
                        }}
                    </span>
                </div>

                <p v-if="!loaded" class="m-0 text-sm text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.loading) }}
                </p>
                <p
                    v-else-if="items.length === 0"
                    class="m-0 text-sm text-[var(--color-text-tertiary)]"
                >
                    {{ formatMessage(messages.empty) }}
                </p>
                <template v-else>
                    <!--
                        列数照抄后台管理：≥1280 四列、≥768 三列（1024–1279 也落在这一档）、
                        更窄两列；间距 10px —— `gap-2.5` 就是 0.625rem。
                    -->
                    <div class="wp-grid">
                        <!-- 轮播控制条（独占一整行）：模式 + 间隔 + 已选张数 -->
                        <div class="wp-rotate-bar">
                            <span class="wp-rotate-bar__label">壁纸轮播</span>
                            <!-- 2026-10-25 13:00：换成启动器同款下拉（首页小组件设置就是 OverflowMenu），
                                 不再用原生 <select> —— 它的展开面板是系统白列表，CSS 管不到。 -->
                            <OverflowMenu :options="rotationModeOptions" placement="bottom-start">
                                {{ rotationModeLabel }} ▾
                            </OverflowMenu>
                            <!-- 2026-10-25 12:30 主人要求：间隔改成滑块拉动（原来是下拉档位） -->
                            <span class="wp-rotate-bar__slider">
                                <input
                                    type="range"
                                    min="5"
                                    max="90"
                                    step="5"
                                    :value="rotationState.intervalMinutes"
                                    @input="
                                        setRotation({
                                            intervalSeconds: Number(
                                                ($event.target as HTMLInputElement).value,
                                            ),
                                        })
                                    "
                                />
                                <!-- 2026-10-25 12:35 主人：间隔改 5~90 **秒**，步进 5 秒 -->
                                <b>{{ rotationState.intervalSeconds }} 秒</b>
                            </span>
                            <!-- 2026-10-25 11:50：主人要「弹出和后台管理选择壁纸一样的形式」
                                 ⇒ 换成按钮，点开是缩略图网格多选（WallpaperRotationPicker）。 -->
                            <button
                                type="button"
                                class="wp-rotate-bar__count is-button"
                                @click="rotationPicker?.show()"
                            >
                                已选 {{ rotationState.ids.length }} 张 ›
                            </button>
                        </div>
                        <WallpaperRotationPicker ref="rotationPicker" />
                        <!-- 壁纸画质（2026-10-30 20:30）：对应 WE 客户端「品质」面板里
                             webwallgl 真能消费的部分（帧率/抗锯齿/后处理/粒子/纹理）。 -->
                        <details class="wp-props-block">
                            <summary>壁纸画质</summary>
                            <div class="wp-quality">
                                <div class="wp-quality-presets" role="radiogroup">
                                    <button
                                        v-for="preset in qualityPresets"
                                        :key="preset.id"
                                        type="button"
                                        :class="{ 'is-active': activePreset === preset.id }"
                                        @click="applyPreset(preset.id)"
                                    >
                                        {{ preset.label }}
                                    </button>
                                </div>

                                <label class="wp-quality-row">
                                    <span>帧率</span>
                                    <input
                                        type="range"
                                        min="30"
                                        max="120"
                                        step="30"
                                        :value="quality.fps"
                                        @input="onFpsInput($event)"
                                    />
                                    <b>{{ quality.fps }}</b>
                                </label>

                                <div class="wp-quality-row">
                                    <span>抗锯齿</span>
                                    <OverflowMenu :options="aaMenu()" placement="bottom-end">
                                        {{ aaLabel }} ▾
                                    </OverflowMenu>
                                </div>

                                <div class="wp-quality-row">
                                    <span>后处理</span>
                                    <OverflowMenu :options="postMenu()" placement="bottom-end">
                                        {{ postLabel }} ▾
                                    </OverflowMenu>
                                </div>

                                <div class="wp-quality-row">
                                    <span>粒子</span>
                                    <OverflowMenu :options="particleMenu()" placement="bottom-end">
                                        {{ particleLabel }} ▾
                                    </OverflowMenu>
                                </div>

                                <label class="wp-quality-row">
                                    <span>纹理分辨率</span>
                                    <input
                                        type="range"
                                        min="0.25"
                                        max="1"
                                        step="0.25"
                                        :value="quality.textureScale"
                                        @input="onScaleInput($event)"
                                    />
                                    <b>{{ Math.round(quality.textureScale * 100) }}%</b>
                                </label>

                                <p class="wp-quality-note">
                                    阴影 / 体积照明 / 反射 属于 WE 的 3D 场景特性，当前 2D 图层合成无法实现。
                                    纹理分辨率改动需重新加载壁纸才完全生效。
                                </p>
                            </div>
                        </details>

                        <!-- 当前壁纸的可调项（WE 的 general.properties）。没有就不显示整块。 -->
                        <details
                            v-if="currentWallpaper && currentWallpaper.hasProperties"
                            class="wp-props-block"
                        >
                            <summary>壁纸可调项 · {{ currentWallpaper.title }}</summary>
                            <WallpaperPropertiesPanel :wallpaper-id="currentWallpaper.id" />
                        </details>
                        <div v-for="item in libraryVisible" :key="item.id" class="group relative">
                            <!-- 轮播多选（主人 06:44）：勾选只进轮播池，不改变当前使用的壁纸 -->
                            <label
                                class="wp-rotate-pick"
                                :title="isRotationId(item.id) ? '从轮播中移除' : '加入轮播'"
                                @click.stop
                            >
                                <input
                                    type="checkbox"
                                    :checked="isRotationId(item.id)"
                                    @change.stop="toggleRotationId(item.id)"
                                />
                            </label>
                            <button
                                type="button"
                                class="wp-tile"
                                :class="{ 'is-selected': isSelected(item) }"
                                :title="tileTooltip(item)"
                                :aria-label="`${formatMessage(messages.use)}: ${item.title}`"
                                :aria-pressed="isSelected(item)"
                                :disabled="(!canUse(item) && !item.damaged) || switching !== null"
                                @click="use(item)"
                            >
                                <img
                                    v-if="previewUrl(item)"
                                    :ref="thumbRef(previewUrl(item))"
                                    :src="thumbSrc(previewUrl(item))"
                                    :data-thumb="previewUrl(item) ?? undefined"
                                    alt=""
                                    loading="lazy"
                                    decoding="async"
                                    draggable="false"
                                    class="wp-tile__img"
                                />
                                <span v-else class="wp-tile__initial">
                                    {{ initialOf(item.title) }}
                                </span>

                                <span class="wp-tile__cap">{{ item.title }}</span>

                                <span
                                    v-if="isSelected(item) || tileFlags(item).length > 0"
                                    class="wp-tile__marks"
                                >
                                    <span v-if="isSelected(item)" class="wp-chip">
                                        {{ formatMessage(messages.inUse) }}
                                    </span>
                                    <span
                                        v-for="flag in tileFlags(item)"
                                        :key="flag.text"
                                        class="wp-dot"
                                        :class="`wp-dot--${flag.tone}`"
                                        :title="flag.text"
                                    />
                                </span>
                            </button>

                            <button
                                type="button"
                                class="wp-tile__delete"
                                :aria-label="`${formatMessage(messages.delete)}: ${item.title}`"
                                :title="formatMessage(messages.delete)"
                                @click.stop="askDelete(item)"
                            >
                                <TrashIcon class="size-4" />
                            </button>
                        </div>
                    </div>

                    <div v-if="libraryRemaining > 0" class="flex justify-center pt-1">
                        <Button type="outlined" size="sm" @click="loadMoreLibrary">
                            {{ formatMessage(messages.loadMore) }}
                            <span class="tabular-nums opacity-70">+{{ libraryRemaining }}</span>
                        </Button>
                    </div>
                </template>
            </div>

            <!-- Steam 库 -->
            <div class="wp-steam">
                <div class="flex flex-wrap items-start justify-between gap-2">
                    <div class="min-w-0">
                        <h3 class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                            {{ formatMessage(messages.steamTitle) }}
                        </h3>
                        <p
                            class="m-0 mt-1 text-xs leading-relaxed text-[var(--color-text-tertiary)]"
                        >
                            {{ formatMessage(messages.steamDescription) }}
                        </p>
                    </div>
                    <div class="flex shrink-0 flex-wrap items-center gap-2">
                        <Button
                            v-if="steamImportable.length > 0 && !batch.running"
                            type="outlined"
                            :disabled="steamBusy"
                            @click="importAllSteam"
                        >
                            {{
                                formatMessage(messages.steamImportAll, {
                                    total: steamImportable.length,
                                })
                            }}
                        </Button>
                        <Button
                            :loading="steamPhase === 'scanning'"
                            :disabled="steamBusy"
                            @click="scanSteam"
                        >
                            {{ formatMessage(messages.steamScan) }}
                        </Button>
                    </div>
                </div>

                <!-- 状态行：一句人话，永远只占一行 -->
                <p class="m-0 text-xs leading-relaxed text-[var(--color-text-tertiary)]">
                    {{ steamStatusText }}
                </p>

                <!-- 批量进度：串行导入，进度是「已完成 / 总数」 -->
                <div v-if="batch.running" class="mt-1 flex flex-col gap-1.5">
                    <div class="flex items-center justify-between gap-2">
                        <span class="text-xs tabular-nums text-[var(--color-text-default)]">
                            {{
                                formatMessage(messages.steamProgress, {
                                    done: batchFinished,
                                    total: batch.total,
                                })
                            }}
                        </span>
                        <Button type="quiet" size="xs" @click="cancelSteamBatch">
                            {{ formatMessage(messages.steamCancel) }}
                        </Button>
                    </div>
                    <div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-4">
                        <div class="wp-progress-fill" :style="{ width: batchPercent }" />
                    </div>
                    <p v-if="batch.failed > 0" class="m-0 text-xs text-orange">
                        {{
                            formatMessage(messages.steamBatchSummary, {
                                done: batch.done,
                                failed: batch.failed,
                            })
                        }}
                    </p>
                </div>

                <!-- 跳过提示：非阻断，只是一句话；具体原因在 tooltip 里 -->
                <p
                    v-if="steamSkipped > 0 && (steamPhase === 'ready' || steamPhase === 'empty')"
                    class="m-0 text-xs leading-relaxed text-orange"
                    :title="steamSkippedTooltip"
                >
                    {{ formatMessage(messages.steamSkippedNotice, { skipped: steamSkipped }) }}
                </p>

                <!-- 空态：没装 WE / 没订阅，两条路径都导向「回到这里点扫描」 -->
                <div
                    v-if="steamPhase === 'empty' || steamPhase === 'unavailable'"
                    class="mt-1 rounded-lg border border-solid border-surface-4 bg-surface-2 p-3"
                >
                    <p class="m-0 text-sm font-semibold text-[var(--color-text-primary)]">
                        {{ steamEmptyHeading }}
                    </p>
                    <p class="m-0 mt-1 text-xs leading-relaxed text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.steamEmptyHint) }}
                    </p>
                </div>

                <!-- 工坊网格 -->
                <template v-if="steamPhase === 'ready'">
                    <div class="wp-grid mt-1">
                        <div
                            v-for="item in steamVisible"
                            :key="item.workshopId"
                            class="group relative"
                            :title="steamTooltip(item)"
                        >
                            <button
                                type="button"
                                class="wp-tile"
                                :class="{ 'is-added': item.importedId !== null }"
                                :aria-label="`${formatMessage(messages.steamOpenInSteam)}: ${item.title}`"
                                @click="openSteamPage(item)"
                            >
                                <img
                                    v-if="steamPreview(item)"
                                    :ref="thumbRef(steamPreview(item))"
                                    :src="thumbSrc(steamPreview(item))"
                                    :data-thumb="steamPreview(item) ?? undefined"
                                    alt=""
                                    loading="lazy"
                                    decoding="async"
                                    draggable="false"
                                    class="wp-tile__img"
                                />
                                <span v-else class="wp-tile__initial">
                                    {{ initialOf(item.title) }}
                                </span>

                                <span class="wp-tile__cap">{{ item.title }}</span>

                                <!-- 悬停提示：整块缩略图就是「在 Steam 中打开」 -->
                                <span class="wp-tile__hint">
                                    {{ formatMessage(messages.steamOpenInSteam) }}
                                </span>

                                <span v-if="steamStateLabel(item)" class="wp-tile__marks">
                                    <span
                                        class="wp-dot wp-dot--warn"
                                        :title="steamStateLabel(item) ?? undefined"
                                    />
                                </span>
                            </button>

                            <div class="absolute top-1.5 right-1.5 z-[1]">
                                <Button
                                    v-if="item.importedId !== null"
                                    type="base"
                                    size="2xs"
                                    disabled
                                >
                                    {{ formatMessage(messages.steamImported) }}
                                </Button>
                                <Button
                                    v-else
                                    type="colored"
                                    size="2xs"
                                    :disabled="steamBusy || !canImportSteamWallpaper(item)"
                                    :loading="steamBusyId === item.workshopId"
                                    @click.stop="importOneSteam(item)"
                                >
                                    {{
                                        steamBusyId === item.workshopId
                                            ? formatMessage(messages.steamImporting)
                                            : formatMessage(messages.steamImport)
                                    }}
                                </Button>
                            </div>
                        </div>
                    </div>

                    <div v-if="steamRemaining > 0" class="flex justify-center pt-1">
                        <Button type="outlined" size="sm" @click="loadMoreSteam">
                            {{ formatMessage(messages.loadMore) }}
                            <span class="tabular-nums opacity-70">+{{ steamRemaining }}</span>
                        </Button>
                    </div>
                </template>
            </div>
        </div>

        <ConfirmModal
            ref="confirmDeleteModal"
            :title="formatMessage(messages.deleteTitle)"
            :description="formatMessage(messages.deleteDescription)"
            :proceed-label="formatMessage(messages.delete)"
            :proceed-icon="TrashIcon"
            :on-hide="onDeleteDialogHidden"
            @proceed="confirmDelete"
        />
    </SettingsSection>
</template>

<style scoped>
.wp-badge {
    display: inline-flex;
    align-items: center;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-sm);
    background: var(--surface-4);
    color: var(--color-text-tertiary);
    font-size: 0.6875rem;
    line-height: 1.4;
    white-space: nowrap;
}

.wp-badge.is-serious {
    color: var(--color-orange, #e08325);
}

/* ============================================================
   缩略图网格
   ------------------------------------------------------------
   列数与间距照抄后台 `.wall-grid`：≥1280 四列、768–1279 三列、<768 两列，
   gap 固定 10px。写在这里而不是用 Tailwind 的 `md:` / `xl:`，是因为要跟
   后台的数值一一对应（默认断点正好是 768 / 1280，但把数字写出来才查得动）。
   ============================================================ */
.wp-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    padding: 4px 0 2px;
}

@media (min-width: 768px) {
    .wp-grid {
        grid-template-columns: repeat(3, minmax(0, 1fr));
    }
}

@media (min-width: 1280px) {
    .wp-grid {
        grid-template-columns: repeat(4, minmax(0, 1fr));
    }
}

/* ============================================================
   缩略图卡片（照抄后台 `.wall-cell`）
   ------------------------------------------------------------
   16:10、圆角、`border: 2px solid transparent` **常驻** —— 选中时才换颜色，
   否则整行会跟着下移 2px。上浮 3px 与 200ms 缓动也一并照抄。
   子元素全部 absolute：比例完全由 `aspect-ratio` 决定，不受内容撑高。
   ============================================================ */
.wp-quality {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 2px;
}

.wp-quality-presets {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;

    button {
        padding: 6px 0;
        border-radius: 8px;
        border: 1px solid var(--stroke-1);
        background: color-mix(in srgb, #0a0b10 28%, transparent);
        color: var(--text-1);
        font-size: 12px;
        cursor: pointer;

        &.is-active {
            border-color: var(--c-a);
            background: color-mix(in srgb, var(--c-a) 24%, transparent);
            color: var(--text-0);
            font-weight: 600;
        }
    }
}

.wp-quality-row {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr) 56px;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-1);

    input[type='range'] {
        width: 100%;
        accent-color: var(--c-a);
        cursor: pointer;
    }

    b {
        text-align: right;
        color: var(--text-0);
        font-variant-numeric: tabular-nums;
    }
}

.wp-quality-note {
    margin: 2px 0 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-3);
}

.wp-props-block {
    grid-column: 1 / -1;
    border-radius: var(--radius-md);
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    padding: 8px 10px;
    font-size: 12px;

    summary {
        cursor: pointer;
        font-weight: 600;
        color: var(--text-0);
        list-style: none;

        &::before {
            content: '▸';
            display: inline-block;
            margin-right: 6px;
            transition: transform 140ms ease;
        }
    }

    &[open] summary::before {
        transform: rotate(90deg);
    }

    &[open] summary {
        margin-bottom: 8px;
    }
}

.wp-rotate-bar {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--glass-1);
    border: 1px solid var(--stroke-1);
    font-size: 12px;
    color: var(--text-1);

    /* 2026-10-25 12:15：原生 <select> 在深色玻璃上会露出**系统白底 + 白下拉面板**
       （主人截图里那个白方块）。`appearance: none` 去掉系统皮肤，自己画深色玻璃 + 箭头。 */
    select {
        appearance: none;
        -webkit-appearance: none;
        padding: 4px 22px 4px 8px;
        border-radius: 7px;
        border: 1px solid var(--stroke-1);
        background-color: color-mix(in srgb, #0a0b10 32%, transparent);
        background-image: linear-gradient(180deg, var(--glass-2), var(--glass-1));
        color: var(--text-0);
        font-size: 12px;
        line-height: 1.3;
        cursor: pointer;
        background-repeat: no-repeat;
        background-position: right 6px center;
        background-size: 8px 5px;
        /* 小三角箭头（内联 SVG，避免额外图片请求） */
        background-image: linear-gradient(180deg, var(--glass-2), var(--glass-1)),
            url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' fill='none' stroke='%23ffffff' stroke-opacity='.7' stroke-width='1.4' stroke-linecap='round'/%3E%3C/svg%3E");
        background-position: right 6px center, right 6px center;
        background-size: auto, 10px 6px;

        option {
            background-color: #12151c;
            color: #fff;
        }
    }
}

.wp-rotate-bar__slider {
    display: inline-flex;
    align-items: center;
    gap: 8px;

    input[type='range'] {
        width: 110px;
        accent-color: var(--c-a);
        cursor: pointer;
    }

    b {
        min-width: 62px;
        color: var(--text-0);
        font-weight: 600;
        font-variant-numeric: tabular-nums;
    }
}

.wp-rotate-bar__label {
    font-weight: 600;
    color: var(--text-0);
}

.wp-rotate-bar__count {
    margin-left: auto;
    color: var(--text-3);
    font-size: 11px;

    &.is-button {
        padding: 4px 9px;
        border-radius: 7px;
        border: 1px solid var(--stroke-1);
        background: color-mix(in srgb, #0a0b10 32%, transparent);
        color: var(--text-1);
        cursor: pointer;
        transition: background-color 140ms ease;

        &:hover {
            background: var(--glass-3);
            color: var(--text-0);
        }
    }
}

/* 壁纸卡右上角的轮播勾选 */
.wp-rotate-pick {
    position: absolute;
    top: 6px;
    right: 6px;
    z-index: 2;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 999px;
    background: color-mix(in srgb, #0a0b10 55%, transparent);
    border: 1px solid var(--stroke-1);
    cursor: pointer;

    input {
        margin: 0;
        width: 12px;
        height: 12px;
        accent-color: var(--c-a);
        cursor: pointer;
    }
}

.wp-tile {
    position: relative;
    display: block;
    width: 100%;
    aspect-ratio: 16 / 10;
    padding: 0;
    overflow: hidden;
    border: 2px solid transparent;
    border-radius: var(--radius-md, 0.5rem);
    background: var(--surface-3);
    text-align: left;
    cursor: pointer;
    transition:
        transform 200ms cubic-bezier(0.16, 1, 0.3, 1),
        border-color 200ms ease,
        box-shadow 200ms ease;
}

.wp-tile:hover {
    transform: translateY(-3px);
}

/* 选中：2px 强调边框 + 3px 光环（光环用项目的 `--color-brand-shadow` 令牌，
   与 `HomeDashboard` / `global.scss` 里的焦点环同一口径）。 */
.wp-tile.is-selected {
    border-color: var(--color-brand);
    box-shadow: 0 0 0 3px var(--color-brand-shadow);
}

/* Steam 那边「已经进过库」的条目：淡一点的边框，表示无操作可做 */
.wp-tile.is-added {
    border-color: color-mix(in srgb, var(--color-brand) 55%, transparent);
}

.wp-tile:disabled {
    cursor: not-allowed;
    transform: none;
}

.wp-tile:focus-visible {
    outline: 2px solid var(--color-brand);
    outline-offset: 2px;
}

@media (prefers-reduced-motion: reduce) {
    .wp-tile,
    .wp-tile:hover {
        transform: none;
        transition: none;
    }
}

.wp-tile__img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

/* 没有预览图（或还没进视口）时的占位：名字首字母 */
.wp-tile__initial {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--color-text-tertiary);
}

/* 底部黑渐变 + 10px 白字标题 */
.wp-tile__cap {
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    padding: 12px 8px 5px;
    overflow: hidden;
    font-size: 10px;
    line-height: 1.25;
    color: #fff;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.7));
}

/* 左上角：使用中角标 + 警示圆点 */
.wp-tile__marks {
    position: absolute;
    top: 6px;
    left: 6px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
}

.wp-chip {
    display: inline-flex;
    align-items: center;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-sm);
    background: var(--color-brand);
    color: #fff;
    font-size: 10px;
    line-height: 1.4;
    white-space: nowrap;
}

.wp-dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    box-shadow: 0 0 0 1.5px rgba(0, 0, 0, 0.45);
}

.wp-dot--danger {
    background: var(--color-red, #d33);
}

.wp-dot--warn {
    background: var(--color-orange, #e08325);
}

.wp-dot--info {
    background: var(--color-text-tertiary);
}

/* 悬停时浮在缩略图中间的「在 Steam 中打开」 */
.wp-tile__hint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 8px;
    font-size: 10px;
    font-weight: 600;
    color: #fff;
    text-align: center;
    background: rgba(0, 0, 0, 0.42);
    opacity: 0;
    transition: opacity 200ms ease;
}

.wp-tile:hover .wp-tile__hint,
.wp-tile:focus-visible .wp-tile__hint {
    opacity: 1;
}

/* 删除按钮：默认隐身，悬停/聚焦才出来 */
.wp-tile__delete {
    position: absolute;
    top: 0.25rem;
    right: 0.25rem;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--surface-1) 78%, transparent);
    color: var(--color-text-tertiary);
    cursor: pointer;
    opacity: 0;
    transition: opacity 150ms ease;
}

.group:hover .wp-tile__delete,
.wp-tile__delete:focus-visible {
    opacity: 1;
}

.wp-tile__delete:hover {
    color: var(--color-red, #d33);
}

/* Steam 区块：用一条分隔线跟「已导入」拉开层次 */
.wp-steam {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding-top: 1.25rem;
    border-top: 1px solid var(--surface-4);
}

.wp-progress-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--color-brand);
    transition: width 200ms ease;
}
</style>
