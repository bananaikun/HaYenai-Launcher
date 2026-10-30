<script setup lang="ts">
/**
 * 首页小组件：系统资源
 *
 * 四条曲线同屏：CPU / GPU / 磁盘 / 网络。
 * 数据来自 Rust 的 `system_stats`（sysinfo 读 CPU/内存/网卡/磁盘，GPU 走 nvidia-smi），
 * 前端只做「定长环形缓冲 + 折线」，不在渲染层做任何统计。
 *
 * 两个刻意的取舍：
 * 1. **轮询只在挂载期间跑**，组件卸载即停 —— 首页常驻，不停表就是白白占 CPU；
 * 2. **GPU 取不到时显示 `—` 并画一条空曲线**，不用 0 冒充「空闲」。
 *    非 NVIDIA 机器上它永远是 `—`，这是事实而不是故障。
 */
import { invoke } from '@tauri-apps/api/core'
import { defineMessages, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    cpu: { id: 'app.home.system.cpu', defaultMessage: 'CPU' },
    gpu: { id: 'app.home.system.gpu', defaultMessage: 'GPU' },
    disk: { id: 'app.home.system.disk', defaultMessage: 'Disk' },
    network: { id: 'app.home.system.network', defaultMessage: 'Network' },
    unavailable: { id: 'app.home.system.unavailable', defaultMessage: 'not available' },
})

/** 采样点数：1.5s × 48 ≈ 72 秒窗口 */
const SAMPLES = 48
const INTERVAL_MS = 1500

type Stats = {
    cpu: number
    gpu: number | null
    gpu_name: string | null
    mem: number
    disk: number
    net: number
}

const stats = ref<Stats>({ cpu: 0, gpu: null, gpu_name: null, mem: 0, disk: 0, net: 0 })
const history = ref<{
    cpu: number[]
    gpu: number[]
    disk: number[]
    net: number[]
}>({ cpu: [], gpu: [], disk: [], net: [] })

let timer: number | null = null
let diskCeiling = 16 * 1024 * 1024
let netCeiling = 64 * 1024

function push(list: number[], value: number): number[] {
    const next = [...list, Math.max(0, Math.min(100, value))]
    return next.length > SAMPLES ? next.slice(next.length - SAMPLES) : next
}

async function poll(): Promise<void> {
    try {
        const raw = await invoke<{
            cpu: number
            memory_used: number
            memory_total: number
            rx_bytes_per_sec: number
            tx_bytes_per_sec: number
            disk_read_bytes_per_sec: number
            disk_write_bytes_per_sec: number
            gpu: number | null
            gpu_name: string | null
        }>('system_stats')

        const netBytes = raw.rx_bytes_per_sec + raw.tx_bytes_per_sec
        const diskBytes = raw.disk_read_bytes_per_sec + raw.disk_write_bytes_per_sec
        // 速率没有固定上限：用滑动峰值当分母，曲线才不会被一次尖峰压成直线
        netCeiling = Math.max(netCeiling * 0.98, netBytes, 64 * 1024)
        diskCeiling = Math.max(diskCeiling * 0.98, diskBytes, 1024 * 1024)

        stats.value = {
            cpu: raw.cpu,
            gpu: raw.gpu,
            gpu_name: raw.gpu_name,
            mem: raw.memory_total ? (raw.memory_used / raw.memory_total) * 100 : 0,
            disk: (diskBytes / diskCeiling) * 100,
            net: (netBytes / netCeiling) * 100,
        }

        history.value = {
            cpu: push(history.value.cpu, raw.cpu),
            gpu: push(history.value.gpu, raw.gpu ?? 0),
            disk: push(history.value.disk, (diskBytes / diskCeiling) * 100),
            net: push(history.value.net, (netBytes / netCeiling) * 100),
        }
    } catch {
        // 命令缺失（旧内核）时保持 0，不弹错
    }
}

function start(): void {
    void poll()
    if (timer === null) timer = window.setInterval(poll, INTERVAL_MS)
}

function stop(): void {
    if (timer !== null) {
        window.clearInterval(timer)
        timer = null
    }
}

/** 0-100 序列 → SVG 折线（viewBox 0 0 100 28） */
function path(values: number[]): string {
    if (values.length < 2) return ''
    const step = 100 / (SAMPLES - 1)
    const offset = SAMPLES - values.length
    return values
        .map((value, index) => {
            const x = (offset + index) * step
            const y = 28 - (Math.max(0, Math.min(100, value)) / 100) * 28
            return `${index === 0 ? 'M' : 'L'}${x.toFixed(2)},${y.toFixed(2)}`
        })
        .join(' ')
}

/** 面积路径：同一条折线闭合到底边，用来铺渐变 */
function area(values: number[]): string {
    const line = path(values)
    if (!line) return ''
    const step = 100 / (SAMPLES - 1)
    const offset = SAMPLES - values.length
    const startX = (offset * step).toFixed(2)
    const endX = ((offset + values.length - 1) * step).toFixed(2)
    return `${line} L${endX},40 L${startX},40 Z`
}

const series = computed(() => [
    { key: 'cpu', label: formatMessage(messages.cpu), value: cpuLabel.value, cls: 'is-cpu', data: history.value.cpu },
    { key: 'gpu', label: formatMessage(messages.gpu), value: gpuLabel.value, cls: 'is-gpu', data: history.value.gpu, note: stats.value.gpu === null },
    { key: 'disk', label: formatMessage(messages.disk), value: memLabel.value, cls: 'is-disk', data: history.value.disk },
    { key: 'net', label: formatMessage(messages.network), value: netLabel.value, cls: 'is-net', data: history.value.net },
])

const cpuLabel = computed(() => `${Math.round(stats.value.cpu)}%`)
const gpuLabel = computed(() =>
    stats.value.gpu === null ? '—' : `${Math.round(stats.value.gpu)}%`,
)
const memLabel = computed(() => `${Math.round(stats.value.mem)}%`)
const netLabel = computed(() => `${(stats.value.net / 1024).toFixed(1)} KB/s`)

onMounted(start)
onBeforeUnmount(stop)
</script>

<template>
    <div class="home-sys">
        <div class="home-sys-legend">
            <span v-for="s in series" :key="s.key" class="home-sys-key" :class="s.cls">
                <i class="home-sys-dot" />
                <span class="home-sys-name">{{ s.label }}</span>
                <b>{{ s.value }}</b>
                <i v-if="s.note" class="home-sys-note">{{ formatMessage(messages.unavailable) }}</i>
            </span>
        </div>
        <svg class="home-sys-chart" viewBox="0 0 100 40" preserveAspectRatio="none" aria-hidden="true">
            <defs>
                <linearGradient
                    v-for="s in series"
                    :id="`sys-grad-${s.key}`"
                    :key="`g-${s.key}`"
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                >
                    <stop offset="0%" class="stop-top" :class="s.cls" />
                    <stop offset="100%" class="stop-bottom" :class="s.cls" />
                </linearGradient>
            </defs>
            <g v-for="s in series" :key="s.key" :class="s.cls">
                <path :d="area(s.data)" class="home-sys-area" :fill="`url(#sys-grad-${s.key})`" />
                <path :d="path(s.data)" class="home-sys-line" />
            </g>
        </svg>
    </div>
</template>

<style scoped lang="scss">
/* 容器查询做「多尺寸」：同一个组件在 1×1 / 2×1 / 2×2 格子里自动调整信息密度，
   不需要额外的 size prop，拖大拖小即刻生效。 */
.home-sys {
    container-type: size;
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
    height: 100%;
    min-height: 0;
}

.home-sys-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 14px;
    flex: none;
}

.home-sys-key {
    display: inline-flex;
    align-items: baseline;
    gap: 5px;
    font-size: 10.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
    white-space: nowrap;

    b {
        color: var(--text-0);
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        letter-spacing: 0;
    }
}

.home-sys-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 8px currentColor;
    transform: translateY(-1px);
}

.home-sys-note {
    font-size: 9px;
    font-style: normal;
    text-transform: none;
    color: var(--text-3);
    opacity: 0.7;
}

.home-sys-chart {
    flex: 1;
    width: 100%;
    min-height: 0;
    overflow: visible;
}

/* 一句话造型：线自己「画」出来（入场描边），面积随呼吸明暗 —— 不再是死线条 */
.home-sys-line {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    vector-effect: non-scaling-stroke;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 420;
    stroke-dashoffset: 420;
    animation: sys-draw 900ms cubic-bezier(0.22, 0.68, 0.16, 1) forwards;
    filter: drop-shadow(0 0 4px currentColor);
}

.home-sys-area {
    stroke: none;
    opacity: 0.16;
    animation: sys-breathe 3.6s ease-in-out infinite;
}

@keyframes sys-draw {
    to {
        stroke-dashoffset: 0;
    }
}

@keyframes sys-breathe {
    0%,
    100% {
        opacity: 0.12;
    }
    50% {
        opacity: 0.26;
    }
}

.is-cpu {
    color: var(--c-a);
}
.is-gpu {
    color: var(--c-b);
}
.is-disk {
    color: var(--c-warn, #e0a13a);
}
.is-net {
    color: var(--c-ok, #4fb27a);
}

/* 渐变 stop 不在 <g> 里，颜色只能按类给 */
.stop-top {
    stop-opacity: 0.55;
}
.stop-bottom {
    stop-opacity: 0;
}
.stop-top.is-cpu,
.stop-bottom.is-cpu {
    stop-color: var(--c-a);
}
.stop-top.is-gpu,
.stop-bottom.is-gpu {
    stop-color: var(--c-b);
}
.stop-top.is-disk,
.stop-bottom.is-disk {
    stop-color: var(--c-warn, #e0a13a);
}
.stop-top.is-net,
.stop-bottom.is-net {
    stop-color: var(--c-ok, #4fb27a);
}

/* —— 多尺寸：只调密度，**数值永远显示**（主人 06:44：「曲线图没有数字、没有信息」）——
   之前 <150px 就把数值 display:none，结果 1×1 里只剩四条没有刻度的线。 */
@container (max-height: 120px) {
    .home-sys-key {
        font-size: 9.5px;
        gap: 3px;
    }
    .home-sys-legend {
        gap: 0 8px;
    }
}
@container (max-width: 220px) {
    .home-sys-name {
        display: none;
    }
}

@media (prefers-reduced-motion: reduce) {
    .home-sys-line {
        animation: none;
        stroke-dashoffset: 0;
    }
    .home-sys-area {
        animation: none;
    }
}
</style>
