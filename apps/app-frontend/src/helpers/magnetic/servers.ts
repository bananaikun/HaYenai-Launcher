/**
 * 磁悬条的服务器数据源：**面板服务器卡与条子服务器页共用这一份**。
 *
 * 为什么不是各组件自己 `invoke`：条子在每个页面都挂着，面板一打开就是两处同时探测，
 * 4 台服务器会变成 8 次 SLP 往返 / 每 15 秒。这里用「引用计数 + 单例定时器」：
 * 有几个组件在用就共用一个轮询，谁都不用了才停。
 *
 * 探测规则（与 1.0.48 的口径一致，只是从单目标扩成多目标）：
 * - `host[:port]` → Rust `server_status`（SLP 握手）：真实延迟、在线/上限、MOTD、图标；
 * - `http(s)://…` → Rust `http_probe`：只有 2xx 才算在线（404/500 是"服务在但坏了"）。
 */
import { invoke } from '@tauri-apps/api/core'
import { onBeforeUnmount, type Ref,ref } from 'vue'

import {
    MAGNETIC_MAX_SERVERS,
    type MagneticServerTarget,
    onMagneticSettingsChange,
} from '@/helpers/magnetic-settings'

export interface MagneticServerRow {
    id: string
    name: string
    address: string
    status: 'unknown' | 'online' | 'offline'
    /** 往返毫秒；非 MC 目标由前端计时，连不上为 null。 */
    latency: number | null
    playersOnline: number | null
    playersMax: number | null
    /** 已剥掉 § 颜色码的纯文本，换行为 `\n`（Rust 侧处理）。 */
    motd: string | null
    /** `data:image/png;base64,…`，直接塞 `<img src>`。 */
    favicon: string | null
    version: string | null
    /** 服务器在，但状态查询被挡（只有延迟是真的）。 */
    statusBlocked: boolean
}

/** Rust 侧 `server_status` 的返回（serde camelCase）。 */
interface MinecraftStatus {
    online: boolean
    latencyMs: number | null
    playersOnline: number | null
    playersMax: number | null
    motd: string | null
    favicon: string | null
    version: string | null
    statusBlocked: boolean
}

const POLL_INTERVAL = 15_000
const DEFAULT_TIMEOUT = 4_000

const rows = ref<MagneticServerRow[]>([])
let targets: MagneticServerTarget[] = []
let timeoutMs = DEFAULT_TIMEOUT
let timer: number | null = null
let holders = 0
let inflight = false
let signature = ''

function emptyRow(target: MagneticServerTarget): MagneticServerRow {
    return {
        id: target.id,
        name: target.name || target.address,
        address: target.address,
        status: 'unknown',
        latency: null,
        playersOnline: null,
        playersMax: null,
        motd: null,
        favicon: null,
        version: null,
        statusBlocked: false,
    }
}

function isMinecraftAddress(value: string): boolean {
    return Boolean(value) && !/^https?:\/\//i.test(value) && !value.includes('/')
}

/** `host[:port]` 拆分：缺省端口交给后端（25565）。 */
function splitHostPort(value: string): [string, number | null] {
    const trimmed = value.trim()
    const match = trimmed.match(/^(.+?):(\d{1,5})$/)
    if (!match) return [trimmed, null]
    return [match[1], Number(match[2])]
}

function withTimeout<T>(task: Promise<T>, ms: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        const id = window.setTimeout(() => reject(new Error('timeout')), ms)
        task.then(
            (value) => {
                window.clearTimeout(id)
                resolve(value)
            },
            (error) => {
                window.clearTimeout(id)
                reject(error)
            },
        )
    })
}

function offline(patch: Partial<MagneticServerRow> = {}): Partial<MagneticServerRow> {
    return {
        status: 'offline',
        latency: null,
        playersOnline: null,
        playersMax: null,
        motd: null,
        favicon: null,
        version: null,
        statusBlocked: false,
        ...patch,
    }
}

async function probe(target: MagneticServerRow): Promise<Partial<MagneticServerRow>> {
    const address = target.address

    if (isMinecraftAddress(address)) {
        const [host, port] = splitHostPort(address)
        try {
            const status = await withTimeout(
                invoke<MinecraftStatus>('server_status', { host, port }),
                Math.max(1_500, timeoutMs * 3),
            )
            return {
                status: status.online ? 'online' : 'offline',
                latency: status.latencyMs ?? null,
                playersOnline: status.playersOnline ?? null,
                playersMax: status.playersMax ?? null,
                motd: status.motd ?? null,
                favicon: status.favicon ?? null,
                version: status.version ?? null,
                statusBlocked: Boolean(status.statusBlocked),
            }
        } catch {
            return offline()
        }
    }

    const startedAt = performance.now()
    try {
        const result = await invoke<{ status: number; ok: boolean }>('http_probe', {
            url: address,
            timeoutMs,
        })
        return {
            status: result.ok ? 'online' : 'offline',
            latency: Math.round(performance.now() - startedAt),
        }
    } catch {
        return offline()
    }
}

async function poll(): Promise<void> {
    if (inflight || !targets.length) return
    inflight = true
    const snapshot = rows.value.slice()
    try {
        const results = await Promise.all(snapshot.map((row) => probe(row)))
        // 期间配置可能被改过：按「地址 + 顺序」写回，不按数组下标硬套。
        const next = rows.value.map((row) => {
            const index = snapshot.findIndex(
                (item) => item.address === row.address && item.id === row.id,
            )
            return index >= 0 ? { ...row, ...results[index] } : row
        })
        rows.value = next
    } finally {
        inflight = false
    }
}

function acquire(): void {
    holders += 1
    if (holders > 1) return
    void poll()
    timer = window.setInterval(() => void poll(), POLL_INTERVAL)
}

function release(): void {
    holders = Math.max(0, holders - 1)
    if (holders > 0 || timer === null) return
    window.clearInterval(timer)
    timer = null
}

/**
 * 同步目标列表。地址没变就只更新显示名（不清掉已探到的数据）；
 * 增删或改地址则重建行并立刻探测一次。
 */
export function syncServers(list: MagneticServerTarget[], timeout = DEFAULT_TIMEOUT): void {
    targets = list
        .filter((item) => item.address.trim())
        .slice(0, MAGNETIC_MAX_SERVERS)
        .map((item) => ({ ...item, address: item.address.trim() }))
    timeoutMs = Number.isFinite(timeout) ? Math.max(500, timeout) : DEFAULT_TIMEOUT

    const nextSignature = targets.map((item) => `${item.id}@${item.address}`).join('|')
    if (nextSignature === signature && rows.value.length === targets.length) {
        rows.value = rows.value.map((row, index) => ({
            ...row,
            name: targets[index]?.name || row.address,
        }))
        return
    }
    signature = nextSignature

    const previous = rows.value
    rows.value = targets.map((target) => {
        const kept = previous.find((row) => row.id === target.id && row.address === target.address)
        return kept ? { ...kept, name: target.name || target.address } : emptyRow(target)
    })
    if (holders > 0) void poll()
}

/**
 * 在组件里用：返回共享的行数据 + 显式的 start/stop。
 *
 * 为什么不在 composable 里直接 onMounted：面板是常驻挂载、只在打开时才有意义的，
 * 交给各自的生命周期决定什么时候开始轮询（条子 = 挂载即开，面板 = 打开才开）。
 */
export function useMagneticServers(
    getTargets: () => MagneticServerTarget[],
    getTimeout?: () => number,
): {
    rows: Ref<MagneticServerRow[]>
    start: () => void
    stop: () => void
    refresh: () => Promise<void>
} {
    const sync = () => syncServers(getTargets(), getTimeout?.())
    sync()

    const stop = onMagneticSettingsChange(sync)
    onBeforeUnmount(stop)

    return {
        rows,
        start: acquire,
        stop: release,
        refresh: poll,
    }
}
