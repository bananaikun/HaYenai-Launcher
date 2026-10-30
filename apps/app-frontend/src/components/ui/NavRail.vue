<!--
  NavRail · 左侧主导航
  ------------------------------------------------------------
  结构与交互逐条对齐后台管理 `Sidebar.tsx`：
    .side > .side-brand + .side-nav(.nav-orb, .nav-group > .nav-group-ttl + .nav-item) + .side-foot
  样式全部来自 `assets/stylesheets/admin-verbatim.scss`（后台 shell.css 的逐字副本），
  本组件不自带一套外观，只负责三件事：
    1. 导航树（分组 + 两级：父项可展开出子项）
    2. 邻近感应 —— 与后台同一条 rAF 时间轴，把每一项的 `--effect` 缓动到目标值
    3. 选中指示器 `.nav-orb` 在项之间滑动

  两级结构是启动器唯一超出后台的地方：后台 12 个页面平铺够用，启动器的页面多一层。
-->
<script setup lang="ts">
import {
    ArrowDownIcon,
    ChangeSkinIcon,
    CompassIcon,
    FlaskConicalIcon,
    HomeIcon,
    ImagesIcon,
    LibraryIcon,
    SettingsIcon,
    UsersIcon,
    WorldIcon,
} from '@modrinth/assets'
import { commonMessages, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { Component } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { h } from 'vue'

import NavButton from '@/components/ui/NavButton.vue'

/** @modrinth/assets 没有 music 图标，这里内联一个音符（与其余图标同为 1.8 线宽） */
const MusicNoteIcon = () =>
    h(
        'svg',
        {
            viewBox: '0 0 24 24',
            width: 16,
            height: 16,
            fill: 'none',
            stroke: 'currentColor',
            'stroke-width': 1.8,
            'stroke-linecap': 'round',
            'stroke-linejoin': 'round',
        },
        [
            h('path', { d: 'M9 18V6l10-2v12' }),
            h('circle', { cx: 6.5, cy: 18, r: 2.5 }),
            h('circle', { cx: 16.5, cy: 16, r: 2.5 }),
        ],
    )

const props = withDefaults(
    defineProps<{
        /** 不需要出现在侧栏里的项（设置里被勾掉 + 未开放的功能） */
        hidden?: string[]
    }>(),
    { hidden: () => [] },
)

const emit = defineEmits<{ 'update:collapsed': [collapsed: boolean] }>()

const route = useRoute()
const router = useRouter()
const { formatMessage } = useVIntl()

const messages = defineMessages({
    groupGames: { id: 'app.navigation.group.games', defaultMessage: 'Games' },
    groupContent: { id: 'app.navigation.group.content', defaultMessage: 'Content' },
    groupOnline: { id: 'app.navigation.group.online', defaultMessage: 'Online' },
    groupAppearance: { id: 'app.navigation.group.appearance', defaultMessage: 'Appearance' },
    groupTools: { id: 'app.navigation.group.tools', defaultMessage: 'Tools' },
    groupSystem: { id: 'app.navigation.group.system', defaultMessage: 'System' },
    groupMedia: { id: 'app.navigation.group.media', defaultMessage: 'Media' },
    music: { id: 'app.navigation.music', defaultMessage: 'Music' },
    recentInstances: { id: 'app.navigation.recent-instances', defaultMessage: 'Recent instances' },
    newInstance: { id: 'app.navigation.create-instance', defaultMessage: 'Create new instance' },
    collapse: { id: 'app.sidebar.collapse', defaultMessage: 'Collapse sidebar' },
    expand: { id: 'app.sidebar.expand', defaultMessage: 'Expand sidebar' },
    home: { id: 'app.navigation.home', defaultMessage: 'Home' },
    library: { id: 'app.navigation.library', defaultMessage: 'Library' },
    downloads: { id: 'app.navigation.downloads', defaultMessage: 'Downloads' },
    worlds: { id: 'app.navigation.worlds', defaultMessage: 'Worlds' },
    screenshots: { id: 'app.navigation.screenshots', defaultMessage: 'Screenshots' },
    discover: { id: 'app.navigation.discover-content', defaultMessage: 'Discover content' },
    multiplayer: { id: 'app.navigation.multiplayer', defaultMessage: 'Multiplayer' },
    skins: { id: 'app.navigation.skin-selector', defaultMessage: 'Skin selector' },
    lab: { id: 'app.navigation.lab', defaultMessage: 'Lab' },
    modpacks: { id: 'app.browse.project-type.modpacks', defaultMessage: 'Modpacks' },
    mods: { id: 'app.browse.project-type.mods', defaultMessage: 'Mods' },
    resourcepacks: { id: 'app.browse.project-type.resourcepacks', defaultMessage: 'Resource packs' },
    shaders: { id: 'app.browse.project-type.shaders', defaultMessage: 'Shaders' },
    datapacks: { id: 'app.browse.project-type.datapacks', defaultMessage: 'Data packs' },
    favorites: { id: 'app.browse.project-type.favorites', defaultMessage: 'Favorites' },
    servers: { id: 'app.multiplayer.tab.servers', defaultMessage: 'Servers' },
    rooms: { id: 'app.multiplayer.tab.rooms', defaultMessage: 'Rooms' },
    skinEditor: { id: 'app.lab.skin-editor.title', defaultMessage: 'Skin editor' },
    recipeGenerator: { id: 'app.lab.recipe-generator.title', defaultMessage: 'Recipe generator' },
    seedMap: { id: 'app.lab.seed-map.title', defaultMessage: 'Seed map' },
    schematicPreview: {
        id: 'app.lab.schematic-preview.title',
        defaultMessage: 'Schematic workshop',
    },
    modTranslation: { id: 'app.lab.mod-translation.title', defaultMessage: 'Mod translation' },
    gradientText: { id: 'app.lab.gradient-text.title', defaultMessage: 'Gradient text generator' },
})

interface NavLeaf {
    id: string
    label: string
    en: string
    to: string
}
interface NavNode extends NavLeaf {
    icon: Component
    children?: NavLeaf[]
}
interface NavGroup {
    id: string
    label: string
    nodes: NavNode[]
}

/**
 * 导航顺序是产品决定，不是排序结果：分组按用途归拢，组内按「天天用 → 偶尔用」排。
 * `en` 是设计稿里的全大写副行（后台同样硬编码在 NAV_GROUPS 里），不走 i18n。
 */
const allGroups = computed<NavGroup[]>(() => [
    {
        id: 'games',
        label: formatMessage(messages.groupGames),
        nodes: [
            {
                id: 'home',
                label: formatMessage(messages.home),
                en: 'HOME',
                to: '/',
                icon: HomeIcon,
            },
            {
                id: 'library',
                label: formatMessage(messages.library),
                en: 'LIBRARY',
                to: '/library',
                icon: LibraryIcon,
            },
            {
                id: 'downloads',
                label: formatMessage(messages.downloads),
                en: 'DOWNLOADS',
                to: '/downloads',
                icon: ArrowDownIcon,
            },
            {
                id: 'worlds',
                label: formatMessage(messages.worlds),
                en: 'WORLDS',
                to: '/worlds',
                icon: WorldIcon,
            },
            {
                id: 'screenshots',
                label: formatMessage(messages.screenshots),
                en: 'SCREENSHOTS',
                to: '/screenshots',
                icon: ImagesIcon,
            },
        ],
    },
    {
        id: 'content',
        label: formatMessage(messages.groupContent),
        nodes: [
            {
                id: 'discover',
                label: formatMessage(messages.discover),
                en: 'DISCOVER',
                to: '/browse/modpack',
                icon: CompassIcon,
                children: [
                    {
                        id: 'discover-modpacks',
                        label: formatMessage(messages.modpacks),
                        en: 'MODPACKS',
                        to: '/browse/modpack',
                    },
                    {
                        id: 'discover-mods',
                        label: formatMessage(messages.mods),
                        en: 'MODS',
                        to: '/browse/mod',
                    },
                    {
                        id: 'discover-resourcepacks',
                        label: formatMessage(messages.resourcepacks),
                        en: 'RESOURCE PACKS',
                        to: '/browse/resourcepack',
                    },
                    {
                        id: 'discover-shaders',
                        label: formatMessage(messages.shaders),
                        en: 'SHADERS',
                        to: '/browse/shader',
                    },
                    {
                        id: 'discover-datapacks',
                        label: formatMessage(messages.datapacks),
                        en: 'DATA PACKS',
                        to: '/browse/datapack',
                    },
                    {
                        id: 'discover-favorites',
                        label: formatMessage(messages.favorites),
                        en: 'FAVORITES',
                        to: '/browse/favorites',
                    },
                ],
            },
        ],
    },
    {
        id: 'online',
        label: formatMessage(messages.groupOnline),
        nodes: [
            {
                id: 'multiplayer',
                label: formatMessage(messages.multiplayer),
                en: 'MULTIPLAYER',
                to: '/multiplayer/servers',
                icon: UsersIcon,
                children: [
                    {
                        id: 'multiplayer-servers',
                        label: formatMessage(messages.servers),
                        en: 'SERVERS',
                        to: '/multiplayer/servers',
                    },
                    {
                        id: 'multiplayer-rooms',
                        label: formatMessage(messages.rooms),
                        en: 'ROOMS',
                        to: '/multiplayer/rooms',
                    },
                ],
            },
        ],
    },
    {
        id: 'appearance',
        label: formatMessage(messages.groupAppearance),
        nodes: [
            {
                id: 'skins',
                label: formatMessage(messages.skins),
                en: 'SKINS',
                to: '/skins',
                icon: ChangeSkinIcon,
            },
        ],
    },
    {
        id: 'media',
        label: formatMessage(messages.groupMedia),
        nodes: [
            {
                id: 'music',
                label: formatMessage(messages.music),
                en: 'MUSIC',
                to: '/music',
                icon: MusicNoteIcon,
            },
        ],
    },
    {
        id: 'tools',
        label: formatMessage(messages.groupTools),
        nodes: [
            {
                id: 'lab',
                label: formatMessage(messages.lab),
                en: 'LAB',
                to: '/lab',
                icon: FlaskConicalIcon,
                children: [
                    {
                        id: 'lab-home',
                        label: formatMessage(messages.lab),
                        en: 'OVERVIEW',
                        to: '/lab',
                    },
                    {
                        id: 'lab-skin-editor',
                        label: formatMessage(messages.skinEditor),
                        en: 'SKIN EDITOR',
                        to: '/lab/skin-editor',
                    },
                    {
                        id: 'lab-recipe',
                        label: formatMessage(messages.recipeGenerator),
                        en: 'RECIPES',
                        to: '/lab/recipe-generator',
                    },
                    {
                        id: 'lab-seed-map',
                        label: formatMessage(messages.seedMap),
                        en: 'SEED MAP',
                        to: '/lab/seed-map',
                    },
                    {
                        id: 'lab-schematic',
                        label: formatMessage(messages.schematicPreview),
                        en: 'SCHEMATICS',
                        to: '/lab/schematic-preview',
                    },
                    {
                        id: 'lab-translation',
                        label: formatMessage(messages.modTranslation),
                        en: 'TRANSLATION',
                        to: '/lab/mod-translation',
                    },
                    {
                        id: 'lab-gradient',
                        label: formatMessage(messages.gradientText),
                        en: 'GRADIENT TEXT',
                        to: '/lab/gradient-text',
                    },
                ],
            },
        ],
    },
    {
        id: 'system',
        label: formatMessage(messages.groupSystem),
        nodes: [
            {
                id: 'settings',
                label: formatMessage(commonMessages.settingsLabel),
                en: 'SETTINGS',
                to: '/settings',
                icon: SettingsIcon,
            },
        ],
    },
])

/** 过滤掉被设置隐藏的项；整组空掉时连标题一起不画 */
const groups = computed<NavGroup[]>(() =>
    allGroups.value
        .map((group) => ({
            ...group,
            nodes: group.nodes.filter((node) => !props.hidden.includes(node.id)),
        }))
        .filter((group) => group.nodes.length > 0),
)

const COLLAPSED_KEY = 'hayenai:side-collapsed'
const OPEN_KEY = 'hayenai:side-open'

function readCollapsed(): boolean {
    try {
        return localStorage.getItem(COLLAPSED_KEY) === '1'
    } catch {
        return false
    }
}
function readOpen(): string[] {
    try {
        const raw = localStorage.getItem(OPEN_KEY)
        const parsed = raw ? (JSON.parse(raw) as unknown) : []
        return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === 'string') : []
    } catch {
        return []
    }
}
function persist(key: string, value: string) {
    try {
        localStorage.setItem(key, value)
    } catch {
        // 隐私模式下 localStorage 会抛；展开/收起退化成「当次有效」，不该打断导航
    }
}

const collapsed = ref(readCollapsed())
const openIds = ref<string[]>(readOpen())

const navEl = ref<HTMLElement | null>(null)
const orbEl = ref<HTMLElement | null>(null)

watch(collapsed, (value) => {
    persist(COLLAPSED_KEY, value ? '1' : '0')
    emit('update:collapsed', value)
})

/** `/` 只吃精确匹配，其余按路径段前缀 —— 否则 `/lab` 会把 `/lab/seed-map` 也点亮 */
function isAt(to: string): boolean {
    if (to === '/') return route.path === '/'
    return route.path === to || route.path.startsWith(`${to}/`)
}

function isNodeActive(node: NavNode): boolean {
    if (isAt(node.to)) return true
    return Boolean(node.children?.some((child) => isAt(child.to)))
}

/**
 * 当前项的 id：父子同时命中时取**子项**（路径更长者优先，等长时子项胜）。
 * 这样落在 `/browse/modpack` 时亮的是「整合包」，父项靠展开箭头表示"我在这儿"。
 */
const activeId = computed(() => {
    let best = ''
    let bestLen = -1
    for (const group of groups.value) {
        for (const node of group.nodes) {
            // 先父后子、等长时后者胜：`/browse/modpack` 同时命中父子，亮子项
            const candidates = node.children ? [node, ...node.children] : [node]
            for (const leaf of candidates) {
                if (!isAt(leaf.to)) continue
                if (leaf.to.length >= bestLen) {
                    best = leaf.id
                    bestLen = leaf.to.length
                }
            }
        }
    }
    return best
})

function isOpen(node: NavNode): boolean {
    return !collapsed.value && openIds.value.includes(node.id)
}

function toggleOpen(node: NavNode) {
    if (collapsed.value || !node.children?.length) return
    openIds.value = openIds.value.includes(node.id)
        ? openIds.value.filter((id) => id !== node.id)
        : [...openIds.value, node.id]
    persist(OPEN_KEY, JSON.stringify(openIds.value))
    // 子项进来后总高变了，指示器要重新落位
    void nextTick(() => syncOrb(true))
}

/** 父项：导航 + 展开。只展开不导航会像"点了没反应"，只导航不展开还得再点一次 */
function onNodeClick(node: NavNode) {
    if (node.children?.length) toggleOpen(node)
}

/* ---------------- 选中指示器 ---------------- */
/**
 * 元素在 `.side-nav` 滚动内容坐标系里的位置。
 * 不能用 `offsetTop`：子项挂在 `.nav-sub`（position:relative）下，offsetTop 是相对它的，
 * 指示器会跳到错误高度。用 rect 差值 + scrollTop 才对所有层级都成立。
 */
function offsetInNav(nav: HTMLElement, el: HTMLElement) {
    const navRect = nav.getBoundingClientRect()
    const rect = el.getBoundingClientRect()
    return { top: rect.top - navRect.top + nav.scrollTop, height: rect.height }
}

function syncOrb(instant = false) {
    const orb = orbEl.value
    const nav = navEl.value
    if (!orb || !nav) return
    const item = nav.querySelector<HTMLElement>('.nav-item.on')
    if (!item) {
        orb.style.opacity = '0'
        return
    }
    const { top, height } = offsetInNav(nav, item)
    /* 当前项可能在折叠区外（侧栏内容比窗口高）：把它滚进可视区，
       否则用户看不到"我在哪"。block:'nearest' 只滚这个容器，不动页面 */
    if (top < nav.scrollTop || top + height > nav.scrollTop + nav.clientHeight) {
        item.scrollIntoView({ block: 'nearest' })
    }
    if (instant) orb.style.transition = 'none'
    orb.style.top = `${top}px`
    orb.style.height = `${height}px`
    orb.style.opacity = '1'
    if (instant) {
        // 强制一次布局，再恢复过渡，避免"瞬移"被下一帧的 transition 吃掉
        void orb.offsetHeight
        orb.style.transition = ''
    }
}

watch(
    () => route.fullPath,
    () => {
        const node = groups.value
            .flatMap((group) => group.nodes)
            .find((candidate) => candidate.children?.length && isNodeActive(candidate))
        if (node && !openIds.value.includes(node.id) && !collapsed.value) {
            openIds.value = [...openIds.value, node.id]
            persist(OPEN_KEY, JSON.stringify(openIds.value))
        }
        void nextTick(() => syncOrb())
    },
)

/* ---------------- 邻近感应 ----------------
   与后台 Sidebar 同一套机制：单个 rAF 循环把每一项的 --effect 缓动到目标值，
   所有项共用一条时间轴，而不是各挂一条 transition 各走各的。
   半径 110px ≈ 两个导航项高；τ=100ms 的指数缓动与帧率无关，掉帧也不变速。 */
const NEAR = 110
const TAU = 0.1

let targets: number[] = []
let values: number[] = []
let raf = 0
let lastTs = 0

function prefersReducedMotion(): boolean {
    return (
        typeof window !== 'undefined' &&
        window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true
    )
}

function itemsIn(nav: HTMLElement): HTMLElement[] {
    return Array.from(nav.querySelectorAll<HTMLElement>('.nav-item'))
}

function frame(now: number) {
    const dt = Math.min((now - lastTs) / 1000, 0.05)
    lastTs = now
    const k = 1 - Math.exp(-dt / TAU)
    const nav = navEl.value
    if (!nav) {
        raf = 0
        return
    }
    const items = itemsIn(nav)
    let moving = false
    items.forEach((el, i) => {
        const target = targets[i] ?? 0
        const current = values[i] ?? 0
        const next = current + (target - current) * k
        const settled = Math.abs(target - next) < 0.0015
        values[i] = settled ? target : next
        el.style.setProperty('--effect', values[i].toFixed(4))
        if (!settled) moving = true
    })
    raf = moving ? requestAnimationFrame(frame) : 0
}

function kick() {
    if (raf) return
    lastTs = performance.now()
    raf = requestAnimationFrame(frame)
}

function onMove(event: PointerEvent) {
    const nav = navEl.value
    if (!nav || collapsed.value || prefersReducedMotion()) return
    const py = event.clientY - nav.getBoundingClientRect().top + nav.scrollTop
    itemsIn(nav).forEach((el, i) => {
        const { top, height } = offsetInNav(nav, el)
        const distance = Math.abs(py - (top + height / 2))
        const p = Math.max(0, 1 - distance / NEAR)
        // smoothstep：边缘更软，比线性更跟手
        targets[i] = p * p * (3 - 2 * p)
    })
    kick()
}

function onLeave() {
    targets = targets.map(() => 0)
    kick()
}

onMounted(() => {
    emit('update:collapsed', collapsed.value)
    void nextTick(() => syncOrb(true))
})

onBeforeUnmount(() => {
    if (raf) cancelAnimationFrame(raf)
    raf = 0
})

function goCreate() {
    void router.push('/create')
}
</script>

<template>
    <aside class="side" :class="{ min: collapsed }">
        <!-- 品牌块（H 光球 + 名称）已按主人要求移除：标题栏在侧栏之上，
             品牌由标题栏左侧承担，侧栏从第一组导航开始 -->
        <nav ref="navEl" class="side-nav" @pointermove="onMove" @pointerleave="onLeave">
            <i ref="orbEl" class="nav-orb" aria-hidden="true"><i /></i>

            <div v-for="group in groups" :key="group.id" class="nav-group">
                <!-- 收起态只留图标列，分组标题会把这一列撑歪 —— 直接不出 -->
                <div v-if="!collapsed" class="nav-group-ttl">
                    <span>{{ group.label }}</span>
                    <span class="line" />
                </div>

                <template v-for="node in group.nodes" :key="node.id">
                    <NavButton
                        :to="node.to"
                        :label="collapsed ? undefined : node.label"
                        :en="collapsed ? undefined : node.en"
                        :title="collapsed ? node.label : undefined"
                        :is-primary="() => node.id === activeId"
                        :class="{ 'is-open': isOpen(node), 'has-sub': node.children?.length }"
                        @click="onNodeClick(node)"
                    >
                        <component :is="node.icon" />
                        <template v-if="node.children?.length && !collapsed" #trail>
                            <svg class="nav-chev" viewBox="0 0 16 16" aria-hidden="true">
                                <path
                                    d="M6 3.2 10.8 8 6 12.8"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="1.5"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                />
                            </svg>
                        </template>
                    </NavButton>

                    <div v-if="node.children?.length && isOpen(node)" class="nav-sub">
                        <NavButton
                            v-for="child in node.children"
                            :key="child.id"
                            :to="child.to"
                            :label="child.label"
                            :en="child.en"
                            :is-primary="() => child.id === activeId"
                            class="is-sub"
                        />
                    </div>
                </template>
            </div>

            <div class="nav-group">
                <div v-if="!collapsed" class="nav-group-ttl">
                    <span>{{ formatMessage(messages.recentInstances) }}</span>
                    <span class="line" />
                    <button
                        type="button"
                        class="nav-add"
                        :title="formatMessage(messages.newInstance)"
                        :aria-label="formatMessage(messages.newInstance)"
                        @click="goCreate"
                    >
                        +
                    </button>
                </div>
                <slot name="instances" />
            </div>
        </nav>

        <div class="side-foot">
            <slot name="foot" />
            <div class="side-toggle-row">
                <button
                    type="button"
                    class="icon-btn side-toggle"
                    :title="
                        collapsed ? formatMessage(messages.expand) : formatMessage(messages.collapse)
                    "
                    :aria-label="
                        collapsed ? formatMessage(messages.expand) : formatMessage(messages.collapse)
                    "
                    :aria-expanded="!collapsed"
                    @click="collapsed = !collapsed"
                >
                    <svg
                        class="toggle-chev"
                        :class="{ flipped: !collapsed }"
                        viewBox="0 0 16 16"
                        aria-hidden="true"
                    >
                        <path
                            d="M6 3.2 10.8 8 6 12.8"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="1.4"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                        />
                    </svg>
                </button>
            </div>
        </div>
    </aside>
</template>

<style scoped lang="scss">
/* 指示器滑动：略带回弹的临界阻尼曲线，切换页面时条子"跟过去"而不是瞬移。
   展开/收起是布局突变，由 syncOrb(true) 临时关掉过渡 */
.nav-orb {
    transition:
        top 420ms cubic-bezier(0.32, 1.28, 0.5, 1),
        height 420ms cubic-bezier(0.32, 1.28, 0.5, 1),
        opacity 200ms var(--ease);
}

/* 导航列会溢出（分组 + 两级子项）。滚动条：
   2026-10-30 23:20 起全站改由 `OverlayScrollbar.vue` 自绘（原生条被整体隐藏），
   这里原本那套 `.side-nav::-webkit-scrollbar` 已成为死样式，已删。 */
.side-nav {
    /* 指示器 .nav-orb 是绝对定位，坐标系必须是这层滚动容器，
       否则它会被 .side（position:relative）接管，滚动时不跟内容走 */
    position: relative;
}

/* 父项右侧的展开箭头 */
.nav-chev {
    flex: none;
    width: 12px;
    height: 12px;
    margin-left: auto;
    color: var(--text-3);
    transition:
        transform var(--t-base) var(--ease),
        color var(--t-base) var(--ease);
}
.nav-item:hover .nav-chev {
    color: var(--text-1);
}
.nav-item.is-open .nav-chev {
    transform: rotate(90deg);
    color: var(--text-1);
}

/* 子项：与父项同一套 .nav-item，只是矮一档、去掉图标位、缩进对齐父项文字 */
.nav-sub {
    display: grid;
    gap: 2px;
    margin: 2px 0 6px;
    padding-left: 53px;
    position: relative;
}
.nav-sub::before {
    content: '';
    position: absolute;
    left: 27px;
    top: 6px;
    bottom: 6px;
    width: 1px;
    background: linear-gradient(180deg, var(--stroke-1), transparent);
    pointer-events: none;
}
.nav-sub :deep(.nav-item.is-sub) {
    height: 34px;
    padding: 0 10px;
    border-radius: var(--r-sm);
    gap: 0;
}
/* 子项没有图标位：留着 30px 的 .nav-ico 会画出一个小方块，还把文字推到右边 */
.nav-sub :deep(.nav-item.is-sub .nav-ico) {
    display: none;
}
.nav-sub :deep(.nav-item.is-sub .nav-cn) {
    font-size: 12.2px;
}
.nav-sub :deep(.nav-item.is-sub .nav-en) {
    display: none;
}

/* 分组标题右侧的"新建实例"小按钮 */
.nav-add {
    flex: none;
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: var(--glass-2);
    color: var(--text-2);
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
    transition:
        background var(--t-fast) var(--ease),
        color var(--t-fast) var(--ease);
}
.nav-add:hover {
    background: var(--glass-4);
    color: var(--text-0);
}

.side-toggle-row {
    display: flex;
    justify-content: flex-end;
}
.side.min .side-toggle-row {
    justify-content: center;
}
.toggle-chev {
    width: 13px;
    height: 13px;
    transition: transform var(--t-base) var(--ease);
}
.toggle-chev.flipped {
    transform: rotate(180deg);
}
</style>
