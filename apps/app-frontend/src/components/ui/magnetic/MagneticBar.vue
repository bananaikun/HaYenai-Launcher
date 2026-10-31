<!--
  磁悬条（MagneticBar）
  ------------------------------------------------------------
  底部一条玻璃条，鼠标停在上面滚轮切换三个面（顺序按主人指定）：

    ① 歌词   与后台管理 LyricBar 同一套表现：封面 + 打字机歌词 + 声波 + 上一首/播放/下一首
    ② 启动   当前实例 + 启动游戏 / 选择版本 / 版本设置
    ③ 系统   CPU / 内存 / 网络上下行，各一条历史曲线

  另有两块不随滚轮切换的信息：安装/下载任务走底部进度带；好友消息到达时由后续接口填入。

  材质与外壳一致（`--ui-*` + 玻璃渐变 + 发丝描边）。阴影刻意收得很小 ——
  早先那层 30px 投影在浅色壁纸下会让底部像被压暗了一道。
-->
<script setup lang="ts">
import StrokeText from '@/components/ui/fx/StrokeText.vue'
import InstanceIcon from '@/components/ui/InstanceIcon.vue'
import { ChevronLeftIcon, ChevronRightIcon, PauseIcon, PlayIcon } from '@modrinth/assets'
import { defineMessages, injectNotificationManager, useVIntl } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { install_job_listener } from '@/helpers/events'
import { list as listInstances, run as runInstance } from '@/helpers/instance'
import { BAND_COUNT, readBands } from '@/helpers/music/music-analyser'
import {
    currentSong,
    loadPlaylist,
    music,
    next as musicNext,
    prev as musicPrev,
    toggle as musicToggle,
} from '@/store/music'

const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const router = useRouter()

const messages = defineMessages({
    lyrics: { id: 'app.magnetic.lyrics', defaultMessage: 'Lyrics' },
    launch: { id: 'app.magnetic.launch', defaultMessage: 'Launch' },
    system: { id: 'app.magnetic.system', defaultMessage: 'System' },
    play: { id: 'app.magnetic.play', defaultMessage: 'Play' },
    pause: { id: 'app.magnetic.pause', defaultMessage: 'Pause' },
    previous: { id: 'app.magnetic.previous', defaultMessage: 'Previous' },
    nextTrack: { id: 'app.magnetic.next', defaultMessage: 'Next' },
    launchGame: { id: 'app.magnetic.launch-game', defaultMessage: 'Launch game' },
    pickVersion: { id: 'app.magnetic.pick-version', defaultMessage: 'Choose version' },
    versionSettings: { id: 'app.magnetic.version-settings', defaultMessage: 'Version settings' },
    noInstance: { id: 'app.magnetic.no-instance', defaultMessage: 'No instance' },
    noLyrics: { id: 'app.magnetic.no-lyrics', defaultMessage: 'Nothing playing' },
    cpu: { id: 'app.magnetic.cpu', defaultMessage: 'CPU' },
    memory: { id: 'app.magnetic.memory', defaultMessage: 'Memory' },
    network: { id: 'app.magnetic.network', defaultMessage: 'Network' },
    launching: { id: 'app.magnetic.launching', defaultMessage: 'Launching…' },
})

type Mode = 'lyrics' | 'launch' | 'system'
/** 顺序即主人指定的顺序：歌词 → 启动/版本 → 系统 */
const MODES: Mode[] = ['lyrics', 'launch', 'system']

const mode = ref<Mode>('lyrics')
const modeIndex = computed(() => MODES.indexOf(mode.value))

interface InstanceRow {
    id: string
    name: string
    loader?: string
    game_version?: string
    install_stage?: string
    icon_path?: string | null
}

/** 选过的版本存住：不然每次开应用都退回"第一个已安装"，得重新点切换。 */
const INSTANCE_KEY = 'hayenai:magnetic-instance'

function readStoredInstance(): string | null {
    try {
        return localStorage.getItem(INSTANCE_KEY)
    } catch {
        return null
    }
}

const instance = ref<InstanceRow | null>(null)
const instanceList = ref<InstanceRow[]>([])
const launching = ref(false)

const song = computed(() => currentSong())
const instanceLabel = computed(() => instance.value?.name ?? formatMessage(messages.noInstance))

/* ---------------- 长句歌词横向滚动 ----------------
   2026-10-31（主人：① 长句要横向滚动 ② 每句要有逐字书写/高亮动画）
   ① `.mb-title` 原来是 `text-overflow: ellipsis` —— 超过一行的歌词直接被省略号吃掉。
      这里改成量出「文字宽度 − 可见宽度」，超了才跑跑马灯：
        先停 0.6s（等逐字书写跑完）→ 匀速左移 → 末尾停 0.8s → 回到开头。
      速度按距离算（28px/s），所以短句不动、长句不会一闪而过。
   ② 逐字书写由 `StrokeText` 自己负责；关键是要**每句重新挂载**
      （`:key="music.activeLyric"`）—— 之前组件实例被复用，CSS 动画只在第一句
      播过一次，后面每句都是"已经写完"的状态，看起来就是歌词不会动。 */
const lyricTrackRef = ref<HTMLElement | null>(null)
const lyricTitleRef = ref<HTMLElement | null>(null)
const marquee = ref({ active: false, shift: 0, duration: 0 })

const marqueeStyle = computed(() => {
    if (!marquee.value.active) return undefined
    return {
        '--mb-marquee-shift': `${marquee.value.shift}px`,
        '--mb-marquee-dur': `${marquee.value.duration}ms`,
    }
})

let marqueeTimer: number | null = null

function measureMarquee(): void {
    const track = lyricTrackRef.value
    const box = lyricTitleRef.value
    if (!track || !box) {
        marquee.value = { active: false, shift: 0, duration: 0 }
        return
    }
    const overflow = Math.ceil(track.scrollWidth - box.clientWidth)
    if (overflow <= 4) {
        marquee.value = { active: false, shift: 0, duration: 0 }
        return
    }
    // 往返一次的距离 ≈ overflow；28px/s 是读长中文句子不费劲的下限
    const distance = overflow + 8
    marquee.value = {
        active: true,
        shift: -distance,
        duration: Math.round(1400 + (distance / 28) * 1000),
    }
}

function scheduleMarqueeMeasure(): void {
    if (marqueeTimer !== null) window.clearTimeout(marqueeTimer)
    marquee.value = { active: false, shift: 0, duration: 0 }
    // StrokeText 要先量字宽、还可能等字体加载，所以延后两帧再测
    marqueeTimer = window.setTimeout(() => {
        marqueeTimer = null
        requestAnimationFrame(() => requestAnimationFrame(measureMarquee))
    }, 80)
}

watch(
    () => music.activeLyric,
    () => scheduleMarqueeMeasure(),
)
watch(
    () => music.settings.lyrics,
    () => scheduleMarqueeMeasure(),
)

const versionLabel = computed(() => {
    const row = instance.value
    if (!row) return ''
    const loader = row.loader ? row.loader[0].toUpperCase() + row.loader.slice(1) : 'Vanilla'
    return `${loader} ${row.game_version ?? ''}`.trim()
})


/**
 * 声波：每帧把频段平均响度写进 `--a`，高度由样式换算。
 * 直接写 DOM 不走响应式 —— 60Hz 的 setState 会把整条歌词重渲染一遍。
 */
const waveRef = ref<HTMLElement | null>(null)
const bandRef = ref<HTMLElement | null>(null)
const bands = new Float32Array(BAND_COUNT)
let waveRaf = 0
let waveLevel = ''

function pulse() {
    waveRaf = 0
    // 没在放音乐就别占一帧：原来无论放不放都每帧读一次频谱、写两次 CSS 变量，
    // 等于给全站多加一条常驻 rAF（滚动时和列表渲染抢同一帧）。
    if (!music.playing) return
    readBands(bands)
    let sum = 0
    for (let i = 0; i < BAND_COUNT; i++) sum += bands[i]
    const level = (sum / BAND_COUNT).toFixed(3)
    // 只在肉眼能看出差别时才写 DOM（千分位抖动没必要触发样式重算）
    if (level !== waveLevel) {
        waveLevel = level
        waveRef.value?.style.setProperty('--a', level)
        // 底边那条振幅带（后台 LyricBar 的 .lyric-bg 同款）：用 scaleX 而不是宽度，避免每帧重排
        bandRef.value?.style.setProperty('--a', level)
    }
    waveRaf = requestAnimationFrame(pulse)
}

/** 播放态驱动循环开关：播放开始起表、暂停停表（幂等）。 */
function syncWaveLoop(): void {
    if (music.playing) {
        if (!waveRaf) waveRaf = requestAnimationFrame(pulse)
    } else if (waveRaf) {
        cancelAnimationFrame(waveRaf)
        waveRaf = 0
        waveLevel = ''
        waveRef.value?.style.setProperty('--a', '0')
        bandRef.value?.style.setProperty('--a', '0')
    }
}

watch(() => music.playing, syncWaveLoop)

/* ---------------- 系统资源：定长环形缓冲 + 曲线 ---------------- */
const SAMPLES = 48
type Stats = { cpu: number; memUsed: number; memTotal: number; rx: number; tx: number }
const stats = ref<Stats>({ cpu: 0, memUsed: 0, memTotal: 0, rx: 0, tx: 0 })
const history = ref<{ cpu: number[]; mem: number[]; net: number[] }>({ cpu: [], mem: [], net: [] })
let statsTimer: number | null = null
let netCeiling = 64 * 1024

function pushSample(arr: number[], value: number): number[] {
    const next = [...arr, value]
    return next.length > SAMPLES ? next.slice(next.length - SAMPLES) : next
}

async function pollStats() {
    try {
        const s = await invoke<{
            cpu: number
            memory_used: number
            memory_total: number
            rx_bytes_per_sec: number
            tx_bytes_per_sec: number
        }>('system_stats')
        stats.value = {
            cpu: s.cpu,
            memUsed: s.memory_used,
            memTotal: s.memory_total,
            rx: s.rx_bytes_per_sec,
            tx: s.tx_bytes_per_sec,
        }
        const memPct = s.memory_total ? (s.memory_used / s.memory_total) * 100 : 0
        const net = s.rx_bytes_per_sec + s.tx_bytes_per_sec
        // 网络没有固定上限：用滑动峰值当分母，曲线才不会被一次尖峰压成直线
        netCeiling = Math.max(netCeiling * 0.98, net, 64 * 1024)
        history.value = {
            cpu: pushSample(history.value.cpu, Math.min(100, Math.max(0, s.cpu))),
            mem: pushSample(history.value.mem, Math.min(100, Math.max(0, memPct))),
            net: pushSample(history.value.net, Math.min(100, (net / netCeiling) * 100)),
        }
    } catch {
        // 命令缺失（旧版本）时这一面显示 0，不弹错误
    }
}

function startStats() {
    void pollStats()
    if (statsTimer === null) statsTimer = window.setInterval(pollStats, 1500)
}
function stopStats() {
    if (statsTimer !== null) {
        window.clearInterval(statsTimer)
        statsTimer = null
    }
}

/** 0-100 序列 → SVG 折线（viewBox 0 0 100 30） */
function sparkPath(values: number[]): string {
    if (values.length < 2) return ''
    const step = 100 / (SAMPLES - 1)
    const offset = SAMPLES - values.length
    return values
        .map((value, index) => {
            const x = (offset + index) * step
            const y = 30 - (Math.max(0, Math.min(100, value)) / 100) * 30
            return `${index === 0 ? 'M' : 'L'}${x.toFixed(2)},${y.toFixed(2)}`
        })
        .join(' ')
}

const memPercent = computed(() =>
    stats.value.memTotal ? Math.round((stats.value.memUsed / stats.value.memTotal) * 100) : 0,
)

function formatBytes(value: number): string {
    if (!Number.isFinite(value) || value <= 0) return '0 B'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    let v = value
    let unit = 0
    while (v >= 1024 && unit < units.length - 1) {
        v /= 1024
        unit += 1
    }
    return `${v.toFixed(v >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`
}

/* ---------------- 下载进度 ---------------- */
interface InstallProgress {
    id: string
    percent: number
}
const downloads = ref<InstallProgress[]>([])
let unlistenJobs: (() => void) | null = null
const activeDownload = computed(() => downloads.value[0] ?? null)

/* ---------------- 交互 ---------------- */
/* ---------------- 拖动 / 3 秒回位 ----------------
   磁悬条是浮层，会挡东西。给两种出路：拖动临时挪开，或松手 3 秒后自己回到原位。
   位置不持久化 —— "临时避让"语义，下次打开仍在默认位（否则用户忘了它去哪了）。 */
const offset = ref({ x: 0, y: 0 })
const dragging = ref(false)
let dragStart = { x: 0, y: 0, ox: 0, oy: 0 }
let returnTimer: number | null = null

function clearReturnTimer() {
    if (returnTimer !== null) {
        window.clearTimeout(returnTimer)
        returnTimer = null
    }
}

function scheduleReturn() {
    clearReturnTimer()
    returnTimer = window.setTimeout(() => {
        offset.value = { x: 0, y: 0 }
        returnTimer = null
    }, 3000)
}

function onDragStart(event: PointerEvent) {
    // 从按钮上按下时不拖动，否则点播放会连带把条子拖走
    if ((event.target as HTMLElement).closest('button')) return
    dragging.value = true
    clearReturnTimer()
    dragStart = { x: event.clientX, y: event.clientY, ox: offset.value.x, oy: offset.value.y }
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}

function onDragMove(event: PointerEvent) {
    if (!dragging.value) return
    offset.value = {
        x: dragStart.ox + (event.clientX - dragStart.x),
        y: dragStart.oy + (event.clientY - dragStart.y),
    }
}

function onDragEnd() {
    if (!dragging.value) return
    dragging.value = false
    scheduleReturn()
}

function onWheel(event: WheelEvent) {
    if (Math.abs(event.deltaY) < 2) return
    const step = event.deltaY > 0 ? 1 : -1
    mode.value = MODES[(modeIndex.value + step + MODES.length) % MODES.length]
}

async function loadInstances() {
    try {
        const rows = (await listInstances()) as InstanceRow[]
        instanceList.value = rows
        const saved = readStoredInstance()
        instance.value =
            rows.find((row) => row.id === saved) ??
            rows.find((row) => row.install_stage === 'installed') ??
            rows[0] ??
            null
    } catch {
        instance.value = null
    }
}

async function launchGame() {
    const row = instance.value
    if (!row || launching.value) return
    launching.value = true
    try {
        await runInstance(row.id)
    } catch (error) {
        handleError(error)
    } finally {
        launching.value = false
    }
}

/* 选择版本：直接展开实例下拉，选中就换"当前要启动的实例"。
   原实现跳去 /instance/:id/upgrade（那是"升级/换加载器"流程），跟"选一个已有实例启动"是两件事。 */
const pickerOpen = ref(false)

function togglePicker() {
    pickerOpen.value = !pickerOpen.value
    if (pickerOpen.value) instanceList.value.length || void loadInstances()
}

function chooseInstance(row: InstanceRow) {
    instance.value = row
    try {
        localStorage.setItem(INSTANCE_KEY, row.id)
    } catch {
        // 隐私模式写不进去：本次会话内仍然生效
    }
    pickerOpen.value = false
}

function openVersionSettings() {
    if (!instance.value) return
    void router.push(`/instance/${encodeURIComponent(instance.value.id)}`)
}

watch(mode, (value) => {
    if (value === 'system') startStats()
    else stopStats()
})

onMounted(async () => {
    // pointer capture 在某些情况下会丢（拖到窗口外松手），此时元素收不到 pointerup，
    // dragging 会永远为 true、条子卡在拖动位置。补一层全局兜底。
    window.addEventListener('pointerup', onDragEnd)
    window.addEventListener('pointercancel', onDragEnd)
    void loadInstances()
    void loadPlaylist()
    syncWaveLoop()
    // 窗口宽度变了，可见宽度跟着变 ⇒ 跑马灯要重新判定（短句变长句、长句变短句）
    window.addEventListener('resize', scheduleMarqueeMeasure)
    scheduleMarqueeMeasure()
    try {
        unlistenJobs = await install_job_listener((payload: unknown) => {
            const job = payload as {
                job_id?: string
                progress?: { current?: number; total?: number } | null
            }
            if (!job?.job_id) return
            const current = job.progress?.current ?? 0
            const total = job.progress?.total ?? 0
            if (!total) return
            const percent = Math.max(0, Math.min(100, Math.round((current / total) * 100)))
            downloads.value = [
                ...downloads.value.filter((item) => item.id !== job.job_id),
                { id: job.job_id, percent },
            ]
        })
    } catch {
        unlistenJobs = null
    }
})

onBeforeUnmount(() => {
    window.removeEventListener('pointerup', onDragEnd)
    window.removeEventListener('pointercancel', onDragEnd)
    window.removeEventListener('resize', scheduleMarqueeMeasure)
    if (marqueeTimer !== null) window.clearTimeout(marqueeTimer)
    stopStats()
    if (typeTimer !== null) window.clearInterval(typeTimer)
    if (waveRaf) cancelAnimationFrame(waveRaf)
    waveRaf = 0
    unlistenJobs?.()
})
</script>

<template>
    <div
        class="magnetic-bar"
        :class="`is-${mode}`, { 'is-dragging': dragging }"
        :style="{
            transform: `translate(calc(-50% + ${offset.x}px), ${offset.y}px)`,
            transition: dragging ? 'none' : undefined,
        }"
        @pointerdown="onDragStart"
        @pointermove="onDragMove"
        @pointerup="onDragEnd"
        @pointercancel="onDragEnd"
    >
        <!-- 振幅带：后台 LyricBar 的 .lyric-bg，贴着底边随响度伸缩 -->
        <span ref="bandRef" class="mb-band" aria-hidden="true" />
        <div class="mb-pages" @wheel.prevent="onWheel">
            <Transition name="mb-swap" mode="out-in">
                <section v-if="mode === 'lyrics'" key="lyrics" class="mb-face mb-lyric">
                    <span v-if="song?.pic" class="mb-cover">
                        <img :src="song.pic" alt="" loading="lazy" decoding="async" />
                    </span>
                    <div class="mb-lyric-text">
                        <span class="mb-kicker">
                            {{ song?.name ?? formatMessage(messages.noLyrics) }}
                        </span>
                        <strong
                            ref="lyricTitleRef"
                            class="mb-title"
                            :class="{ 'is-marquee': marquee.active }"
                        >
                            <template v-if="!music.settings.lyrics">纯音乐</template>
                            <template v-else-if="music.activeLyric">
                                <span ref="lyricTrackRef" class="mb-title-track" :style="marqueeStyle">
                                    <StrokeText
                                        :key="music.activeLyric"
                                        class="mb-title-stx"
                                        :text="music.activeLyric"
                                    /><i class="mb-caret" />
                                </span>
                            </template>
                            <template v-else-if="music.playing">♪ ♪</template>
                            <template v-else-if="music.error">音乐源不可用</template>
                            <template v-else>点此播放</template>
                        </strong>
                        <span class="mb-sub">{{ song?.artist ?? music.error }}</span>
                    </div>
                    <div ref="waveRef" class="mb-wave" :class="{ on: music.playing }">
                        <i v-for="i in 5" :key="i" />
                    </div>
                </section>

                <section v-else-if="mode === 'launch'" key="launch" class="mb-face">
                    <span class="mb-kicker mb-kicker--with-icon">
                        <InstanceIcon
                            v-if="instance"
                            class="mb-ver-icon"
                            :icon-path="instance.icon_path"
                            :instance-id="instance.id"
                            :loader="instance.loader"
                        />
                        {{ instanceLabel }}
                    </span>
                    <div class="mb-actions-row">
                        <button
                            type="button"
                            class="mb-cta"
                            :disabled="!instance || launching"
                            @click="launchGame"
                        >
                            {{
                                launching
                                    ? formatMessage(messages.launching)
                                    : formatMessage(messages.launchGame)
                            }}
                        </button>
                        <button
                            type="button"
                            class="mb-chip"
                            :disabled="!instanceList.length"
                            :aria-expanded="pickerOpen"
                            @click="togglePicker"
                        >
                            {{ formatMessage(messages.pickVersion) }}
                        </button>
                        <button
                            type="button"
                            class="mb-chip"
                            :disabled="!instance"
                            @click="openVersionSettings"
                        >
                            {{ formatMessage(messages.versionSettings) }}
                        </button>
                    </div>
                </section>

                <section v-else key="system" class="mb-face mb-system">
                    <div class="mb-graph">
                        <span class="mb-graph-label">
                            {{ formatMessage(messages.cpu) }}
                            <b>{{ Math.round(stats.cpu) }}%</b>
                        </span>
                        <svg viewBox="0 0 100 30" preserveAspectRatio="none" aria-hidden="true">
                            <path :d="sparkPath(history.cpu)" class="mb-line is-cpu" />
                        </svg>
                    </div>
                    <div class="mb-graph">
                        <span class="mb-graph-label">
                            {{ formatMessage(messages.memory) }}
                            <b>{{ memPercent }}%</b>
                        </span>
                        <svg viewBox="0 0 100 30" preserveAspectRatio="none" aria-hidden="true">
                            <path :d="sparkPath(history.mem)" class="mb-line is-mem" />
                        </svg>
                    </div>
                    <div class="mb-graph">
                        <span class="mb-graph-label">
                            ↑{{ formatBytes(stats.tx) }}/s · ↓{{ formatBytes(stats.rx) }}/s
                        </span>
                        <svg viewBox="0 0 100 30" preserveAspectRatio="none" aria-hidden="true">
                            <path :d="sparkPath(history.net)" class="mb-line is-net" />
                        </svg>
                    </div>
                </section>
            </Transition>
        </div>

        <div class="mb-dots" role="tablist">
            <button
                v-for="item in MODES"
                :key="item"
                type="button"
                class="mb-dot"
                :class="{ on: item === mode }"
                v-tooltip="formatMessage(messages[item])"
                :aria-label="formatMessage(messages[item])"
                @click="mode = item"
            />
        </div>

        <div class="mb-actions">
            <button
                type="button"
                class="mb-ctl"
                v-tooltip="formatMessage(messages.previous)"
                @click="musicPrev"
            >
                <ChevronLeftIcon class="size-3.5" />
            </button>
            <button
                type="button"
                class="mb-ctl is-main"
                v-tooltip="
                    music.playing ? formatMessage(messages.pause) : formatMessage(messages.play)
                "
                @click="musicToggle"
            >
                <PauseIcon v-if="music.playing" class="size-3.5" />
                <PlayIcon v-else class="size-3.5" />
            </button>
            <button
                type="button"
                class="mb-ctl"
                v-tooltip="formatMessage(messages.nextTrack)"
                @click="musicNext"
            >
                <ChevronRightIcon class="size-3.5" />
            </button>
        </div>

        <Transition name="mb-pop">
            <div v-if="pickerOpen" class="mb-picker" role="listbox">
                <button
                    v-for="row in instanceList"
                    :key="row.id"
                    type="button"
                    class="mb-picker-row"
                    :class="{ on: row.id === instance?.id }"
                    role="option"
                    :aria-selected="row.id === instance?.id"
                    @click="chooseInstance(row)"
                >
                    <span class="mb-picker-name">{{ row.name }}</span>
                    <span class="mb-picker-tag">
                        {{ (row.loader || 'vanilla').toUpperCase() }} {{ row.game_version ?? '' }}
                    </span>
                </button>
            </div>
        </Transition>

        <div v-if="activeDownload" class="mb-progress" aria-hidden="true">
            <i :style="{ width: `${activeDownload.percent}%` }" />
        </div>
    </div>
</template>

<style scoped lang="scss">
.magnetic-bar {
    position: fixed;
    left: 50%;
    bottom: 18px;
    z-index: 120;
    /* 水平居中交给内联 transform（拖动时要叠加偏移），这里只给回位过渡 */
    transition: transform 320ms var(--ease-out);
    touch-action: none;
    cursor: grab;

    display: flex;
    align-items: center;
    gap: 14px;
    width: min(640px, calc(100vw - 320px));
    min-height: 62px;
    padding: 9px 14px 9px 16px;

    border-radius: 20px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-2) 100%),
        color-mix(in srgb, var(--surface-1) var(--ui-card-alpha, 55%), transparent);
    -webkit-backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(180%);
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(180%);
    /* 只留贴边的细影：30px 的大投影在浅色壁纸下会让底部像被压暗一块 */
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
    color: var(--text-0);
    /* ⚠ 不能 overflow:hidden —— 「选择版本」的下拉是 relative 到条子、挂在条子上方的，
       一旦裁剪就整块看不见，表现就是"点了没反应"。进度带与振幅带各自带圆角，不需要裁。 */
    overflow: visible;
    user-select: none;
}

.mb-pages {
    flex: 1;
    min-width: 0;
    height: 44px;
    display: flex;
    align-items: center;
}

.mb-face {
    display: grid;
    gap: 1px;
    min-width: 0;
    width: 100%;
}

.mb-kicker {
    font-size: 9.5px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

/* 「启动游戏」那栏的版本图标（2026-10-24 22:50 主人要求补上） */
.mb-kicker--with-icon {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    letter-spacing: 0.06em;
}

.mb-ver-icon {
    width: 16px;
    height: 16px;
    flex: none;
    overflow: hidden;
    border-radius: 4px;

    :deep(img) {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
}

.mb-title {
    font-size: 14px;
    font-weight: 700;
    line-height: 1.25;
    letter-spacing: -0.01em;
    /* 2026-10-31：去掉 `text-overflow: ellipsis` —— 长句不再被省略号截断，
       改由 `.mb-title-track` 跑马灯（见下）。`overflow: hidden` 保留用于裁切。 */
    white-space: nowrap;
    overflow: hidden;
    /* 后台 LyricBar 的当前行是"描边书写"：字有轮廓、再填色。
       这里用一层描边 + 主题色微光近似（同一套观感，不必搬 canvas 字形）。 */
    -webkit-text-stroke: 0.35px color-mix(in srgb, currentColor 55%, transparent);
    text-shadow: 0 0 14px rgba(var(--c-a-rgb), 0.45);
    animation: mb-write 520ms var(--ease-out);
}

/* 歌词跑马灯：文字超出可见宽度时，先停 0.6s（等逐字书写跑完）→ 左移 → 停 0.8s → 回位。
   `--mb-marquee-shift` 由脚本按「溢出多少像素」写入，速度也是按距离算的。 */
.mb-title-track {
    display: inline-flex;
    align-items: baseline;
    will-change: transform;
}

.mb-title.is-marquee .mb-title-track {
    animation: mb-marquee var(--mb-marquee-dur, 6000ms) ease-in-out infinite;
}

@keyframes mb-marquee {
    0%,
    10% {
        transform: translateX(0);
    }
    88%,
    96% {
        transform: translateX(var(--mb-marquee-shift, 0));
    }
    100% {
        transform: translateX(0);
    }
}

@media (prefers-reduced-motion: reduce) {
    .mb-title.is-marquee .mb-title-track {
        animation: none;
    }
}

/* 当前行歌词 = 后台 `.lyr-line.on` 的口径：纯白填充 + 纯白描边，且描边压细到 0.02em
   —— 0.07em 是常规字号调的，放大行上会与填充糊成一团（后台实测的结论）。 */
.mb-title-stx :deep(.stx-c text) {
    fill: #fff;
    stroke: #fff;
    stroke-width: 0.02em;
    filter: drop-shadow(0 0 10px rgba(var(--c-a-rgb), 0.45));
}

@keyframes mb-write {
    from {
        opacity: 0.35;
        letter-spacing: 0.08em;
    }
    to {
        opacity: 1;
        letter-spacing: -0.01em;
    }
}

.mb-sub {
    font-size: 11px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

/* 歌词面：封面 + 打字机 + 声波横排 */
.mb-lyric {
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
}

.mb-cover {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    overflow: hidden;
    border: 1px solid var(--stroke-1);
    flex: none;

    img {
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
}

.mb-lyric-text {
    display: grid;
    gap: 0;
    min-width: 0;
}

.mb-caret {
    display: inline-block;
    width: 6px;
    height: 12px;
    margin-left: 2px;
    background: var(--c-a);
    vertical-align: -1px;
    animation: mb-blink 1s steps(2, start) infinite;
}

@keyframes mb-blink {
    to {
        visibility: hidden;
    }
}

/* 声波：**逐字照后台 LyricBar 的 `.wave`**（2026-10-24 22:45，主人要"一模一样"）
   —— 3px 宽、gap 3px、26px 高、`--grad-c` 渐变、1.05s 呼吸、6↔24px、
   延时 0/190/380/100/290ms。后台是纯 CSS 波动，不吃 --a，这里也一样。 */
.mb-wave {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 26px;
    flex: none;

    i {
        width: 3px;
        border-radius: 2px 2px 1px 1px;
        background: var(--grad-c);
        height: 5px;
        transition: height 420ms var(--ease);
    }

    &.on i {
        animation: mb-wave-beat 1.05s var(--ease) infinite;
    }
    &.on i:nth-child(1) {
        animation-delay: 0ms;
    }
    &.on i:nth-child(2) {
        animation-delay: 190ms;
    }
    &.on i:nth-child(3) {
        animation-delay: 380ms;
    }
    &.on i:nth-child(4) {
        animation-delay: 100ms;
    }
    &.on i:nth-child(5) {
        animation-delay: 290ms;
    }
}

@keyframes mb-wave-beat {
    0%,
    100% {
        height: 6px;
    }
    50% {
        height: 24px;
    }
}

/* 启动面：三个动作横排 */
.mb-actions-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
}

.mb-cta {
    height: 30px;
    padding: 0 14px;
    border: 0;
    border-radius: 10px;
    background: var(--grad-a);
    color: #fff;
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition:
        transform 160ms var(--ease-out),
        filter var(--t-fast) var(--ease);

    &:hover:not(:disabled) {
        transform: translateY(-1px);
        filter: brightness(1.08);
    }

    &:disabled {
        opacity: 0.5;
        cursor: default;
    }
}

.mb-chip {
    height: 30px;
    padding: 0 12px;
    border: 1px solid var(--stroke-1);
    border-radius: 10px;
    background: var(--glass-2);
    color: var(--text-1);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
    transition:
        background var(--t-fast) var(--ease),
        color var(--t-fast) var(--ease);

    &:hover:not(:disabled) {
        background: var(--glass-4);
        color: var(--text-0);
    }

    &:disabled {
        opacity: 0.45;
        cursor: default;
    }
}

/* 系统面：三条曲线 */
.mb-system {
    display: flex;
    align-items: center;
    gap: 16px;
}

.mb-graph {
    display: grid;
    gap: 2px;
    flex: 1;
    min-width: 0;

    svg {
        width: 100%;
        height: 24px;
        overflow: visible;
    }
}

.mb-graph-label {
    font-size: 9.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;

    b {
        color: var(--text-1);
        font-weight: 600;
        font-variant-numeric: tabular-nums;
    }
}

.mb-line {
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;

    &.is-cpu {
        stroke: var(--c-a);
    }
    &.is-mem {
        stroke: var(--c-b);
    }
    &.is-net {
        stroke: var(--c-ok);
    }
}

.mb-dots {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: none;
}

.mb-dot {
    width: 5px;
    height: 5px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--stroke-2);
    cursor: pointer;
    transition:
        background var(--t-base) var(--ease),
        transform var(--t-base) var(--ease);

    &.on {
        background: var(--c-a);
        transform: scale(1.35);
    }
}

.mb-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
}

.mb-ctl {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--stroke-1);
    border-radius: 50%;
    background: var(--glass-2);
    color: var(--text-1);
    cursor: pointer;
    transition:
        background var(--t-fast) var(--ease),
        color var(--t-fast) var(--ease);

    &:hover {
        background: var(--glass-4);
        color: var(--text-0);
    }

    &.is-main {
        width: 32px;
        height: 32px;
        background: var(--glass-4);
        color: var(--text-0);
    }
}

.magnetic-bar.is-dragging {
    cursor: grabbing;
    transition: none;
    box-shadow: 0 8px 24px -12px rgba(0, 0, 0, 0.35);
}

/* 底边振幅带：宽 100% 用 scaleX 伸缩，中间对齐 */
.mb-band {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 0;
    /* 3px + 光晕：2px 在浅色壁纸上几乎看不见（主人的原话是"不明显"） */
    height: 3px;
    transform-origin: 50% 100%;
    transform: scaleX(calc(0.06 + var(--a, 0) * 0.94));
    background: linear-gradient(90deg, transparent, var(--c-a) 18%, var(--c-a) 82%, transparent);
    box-shadow: 0 0 10px -1px rgba(var(--c-a-rgb), 0.9);
    opacity: 1;
    border-radius: 3px;
    pointer-events: none;
}

/* 选择版本的下拉：贴在条子上方 */
.mb-picker {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: calc(100% + 8px);
    max-height: 240px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px;
    border-radius: 14px;
    border: 1px solid var(--stroke-1);
    background:
        linear-gradient(180deg, var(--glass-3) 0%, var(--glass-1) 100%),
        color-mix(in srgb, var(--surface-2-opaque) var(--ui-card-alpha, 55%), transparent);
    -webkit-backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(160%);
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(160%);
    box-shadow: var(--sh-2);
}

.mb-picker-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 7px 10px;
    border: 0;
    border-radius: 9px;
    background: none;
    color: var(--text-1);
    font: inherit;
    text-align: left;
    cursor: pointer;

    &:hover {
        background: var(--glass-2);
        color: var(--text-0);
    }

    &.on {
        background: color-mix(in srgb, var(--c-a) 20%, transparent);
        color: var(--text-0);
    }
}

.mb-picker-name {
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.mb-picker-tag {
    font-size: 10px;
    color: var(--text-3);
    flex: none;
}

.mb-pop-enter-active,
.mb-pop-leave-active {
    transition:
        opacity 140ms var(--ease),
        transform 170ms var(--ease-out);
}

.mb-pop-enter-from,
.mb-pop-leave-to {
    opacity: 0;
    transform: translateY(6px);
}

.mb-progress {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 4px;
    height: 2px;
    border-radius: 2px;
    background: var(--stroke-1);
    overflow: hidden;

    i {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--grad-a);
        transition: width 240ms var(--ease);
    }
}

.mb-swap-enter-active,
.mb-swap-leave-active {
    transition:
        opacity 150ms var(--ease),
        transform 190ms var(--ease-out);
}

.mb-swap-enter-from {
    opacity: 0;
    transform: translateX(8px);
}

.mb-swap-leave-to {
    opacity: 0;
    transform: translateX(-8px);
}

@media (prefers-reduced-motion: reduce) {
    .mb-swap-enter-active,
    .mb-swap-leave-active {
        transition: opacity 120ms linear;
        transform: none;
    }
    .mb-wave.on i {
        animation: none;
        height: 60%;
    }
}

@media (max-width: 900px) {
    .magnetic-bar {
        width: calc(100vw - 120px);
    }
}
</style>
