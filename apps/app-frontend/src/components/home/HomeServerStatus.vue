<script setup lang="ts">
/**
 * 首页小组件：服务器状态
 *
 * 每一行是一个服务器，直连 Java 版 SLP（Server List Ping）拿四样东西：
 * **延迟 / 在线人数 / MOTD / 版本**。协议实现在 Rust（`api::server_status`），
 * 因为 WebView 发不出裸 TCP。
 *
 * 刻意的取舍：
 * 1. **列表存 localStorage**，不进 settings 表 —— 它只是几个地址，改一次不用重建配置表；
 * 2. **轮询 30s，只在可见时跑**（`document.hidden` 时不打），避免后台常驻白耗连接；
 * 3. 服务器「在但状态被挡」（部分服关 query）时**不显示成离线** —— 显示「仅延迟」，
 *    因为那台机器确实 ping 得通。
 */
import { invoke } from '@tauri-apps/api/core'
import { defineMessages, useVIntl } from '@modrinth/ui'
import { onBeforeUnmount, onMounted, ref } from 'vue'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    add: { id: 'app.home.servers.add', defaultMessage: 'Add server' },
    placeholder: { id: 'app.home.servers.placeholder', defaultMessage: 'play.example.com:25565' },
    offline: { id: 'app.home.servers.offline', defaultMessage: 'Offline' },
    checking: { id: 'app.home.servers.checking', defaultMessage: 'Checking…' },
    blocked: { id: 'app.home.servers.blocked', defaultMessage: 'ping only' },
    empty: { id: 'app.home.servers.empty', defaultMessage: 'No servers yet' },
    remove: { id: 'app.home.servers.remove', defaultMessage: 'Remove' },
})

type ServerEntry = { host: string; port: number; id: string }

type ServerStatus = {
    online: boolean
    latencyMs: number | null
    version: string | null
    protocol: number | null
    playersOnline: number | null
    playersMax: number | null
    motd: string | null
    favicon: string | null
    statusBlocked: boolean
    error: string | null
}

const STORAGE_KEY = 'hayenai:home-servers'
const POLL_MS = 30_000

const entries = ref<ServerEntry[]>([])
const states = ref<Record<string, ServerStatus | 'pending'>>({})
const draft = ref('')
let timer: number | null = null

function loadEntries(): ServerEntry[] {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return []
        const parsed = JSON.parse(raw)
        if (!Array.isArray(parsed)) return []
        return parsed
            .filter((item) => item && typeof item.host === 'string')
            .map((item) => ({
                host: String(item.host),
                port: Number(item.port) || 25565,
                id: `${item.host}:${Number(item.port) || 25565}`,
            }))
    } catch {
        return []
    }
}

function saveEntries(): void {
    try {
        localStorage.setItem(
            STORAGE_KEY,
            JSON.stringify(entries.value.map(({ host, port }) => ({ host, port }))),
        )
    } catch {
        // 隐私模式写不进去：本次会话仍然有效
    }
}

/** `host` / `host:port` / `[::1]:25565` 都能解析。 */
function parseTarget(text: string): { host: string; port: number } | null {
    const trimmed = text.trim()
    if (!trimmed) return null
    const bracket = /^\[([^\]]+)\](?::(\d+))?$/.exec(trimmed)
    if (bracket) return { host: bracket[1], port: Number(bracket[2] ?? 25565) }
    const parts = trimmed.split(':')
    if (parts.length === 2 && /^\d+$/.test(parts[1])) {
        return { host: parts[0], port: Number(parts[1]) }
    }
    return { host: trimmed, port: 25565 }
}

function addEntry(): void {
    const target = parseTarget(draft.value)
    if (!target) return
    const id = `${target.host}:${target.port}`
    if (!entries.value.some((entry) => entry.id === id)) {
        entries.value = [...entries.value, { ...target, id }]
        saveEntries()
    }
    draft.value = ''
    void refreshOne(id)
}

function removeEntry(id: string): void {
    entries.value = entries.value.filter((entry) => entry.id !== id)
    const next = { ...states.value }
    delete next[id]
    states.value = next
    saveEntries()
}

async function refreshOne(id: string): Promise<void> {
    const entry = entries.value.find((item) => item.id === id)
    if (!entry) return
    states.value = { ...states.value, [id]: 'pending' }
    try {
        const status = await invoke<ServerStatus>('server_status', {
            host: entry.host,
            port: entry.port,
        })
        states.value = { ...states.value, [id]: status }
    } catch (error) {
        states.value = {
            ...states.value,
            [id]: {
                online: false,
                latencyMs: null,
                version: null,
                protocol: null,
                playersOnline: null,
                playersMax: null,
                motd: null,
                favicon: null,
                statusBlocked: false,
                error: String(error),
            },
        }
    }
}

function refreshAll(): void {
    if (document.hidden) return
    for (const entry of entries.value) void refreshOne(entry.id)
}

function stateOf(id: string): ServerStatus | 'pending' | undefined {
    return states.value[id]
}

/** 延迟分档：绿 <80 / 黄 <160 / 红 其余 —— 不写死颜色值，交给 CSS 类。 */
function latencyClass(ms: number | null): string {
    if (ms === null) return 'is-unknown'
    if (ms < 80) return 'is-good'
    if (ms < 160) return 'is-fair'
    return 'is-slow'
}

onMounted(() => {
    entries.value = loadEntries()
    refreshAll()
    timer = window.setInterval(refreshAll, POLL_MS)
    document.addEventListener('visibilitychange', refreshAll)
})

onBeforeUnmount(() => {
    if (timer !== null) window.clearInterval(timer)
    timer = null
    document.removeEventListener('visibilitychange', refreshAll)
})
</script>

<template>
    <div class="home-srv">
        <form class="home-srv-add" @submit.prevent="addEntry">
            <input
                v-model="draft"
                class="home-srv-input"
                :placeholder="formatMessage(messages.placeholder)"
                spellcheck="false"
                autocomplete="off"
            />
            <button type="submit" class="home-srv-btn">{{ formatMessage(messages.add) }}</button>
        </form>

        <p v-if="!entries.length" class="home-srv-empty">
            {{ formatMessage(messages.empty) }}
        </p>

        <ul v-else class="home-srv-list">
            <li v-for="entry in entries" :key="entry.id" class="home-srv-row">
                <span class="home-srv-ping" :class="latencyClass((stateOf(entry.id) as ServerStatus)?.latencyMs ?? null)">
                    <i />
                    {{
                        stateOf(entry.id) === 'pending'
                            ? '…'
                            : ((stateOf(entry.id) as ServerStatus)?.latencyMs != null
                              ? `${(stateOf(entry.id) as ServerStatus).latencyMs} ms`
                              : formatMessage(messages.offline))
                    }}
                </span>

                <span class="home-srv-main">
                    <span class="home-srv-host">
                        <img
                            v-if="(stateOf(entry.id) as ServerStatus)?.favicon"
                            class="home-srv-icon"
                            :src="(stateOf(entry.id) as ServerStatus).favicon ?? undefined"
                            alt=""
                            draggable="false"
                        />
                        <i v-else class="home-srv-icon is-fallback">
                            {{ entry.host.slice(0, 1).toUpperCase() }}
                        </i>{{ entry.host }}<template v-if="entry.port !== 25565">:{{ entry.port }}</template></span>
                    <span class="home-srv-motd">
                        {{
                            stateOf(entry.id) === 'pending'
                                ? formatMessage(messages.checking)
                                : ((stateOf(entry.id) as ServerStatus)?.motd ??
                                  (stateOf(entry.id) as ServerStatus)?.error ??
                                  formatMessage(messages.offline))
                        }}
                    </span>
                </span>

                <span class="home-srv-players">
                    <template v-if="(stateOf(entry.id) as ServerStatus)?.playersOnline != null">
                        {{ (stateOf(entry.id) as ServerStatus).playersOnline }}<span class="sep">/</span>{{ (stateOf(entry.id) as ServerStatus).playersMax ?? '?' }}
                    </template>
                    <i v-else-if="(stateOf(entry.id) as ServerStatus)?.statusBlocked" class="home-srv-blocked">
                        {{ formatMessage(messages.blocked) }}
                    </i>
                </span>

                <button
                    type="button"
                    class="home-srv-del"
                    :aria-label="formatMessage(messages.remove)"
                    @click="removeEntry(entry.id)"
                >
                    ×
                </button>
            </li>
        </ul>
    </div>
</template>

<style scoped lang="scss">
.home-srv {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: thin;
}

.home-srv-add {
    display: flex;
    gap: 6px;
    flex: none;
}

.home-srv-input {
    flex: 1;
    min-width: 0;
    padding: 5px 9px;
    font-size: 11.5px;
    border-radius: 8px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    color: var(--text-0);

    &::placeholder {
        color: var(--text-3);
    }

    &:focus-visible {
        outline: 2px solid color-mix(in srgb, var(--c-a) 60%, transparent);
        outline-offset: 1px;
    }
}

.home-srv-btn,
.home-srv-del {
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--text-1);
    border-radius: 8px;
    cursor: pointer;
    font-size: 11.5px;
    transition:
        background-color var(--t-base, 360ms) ease,
        color var(--t-base, 360ms) ease;
}

.home-srv-btn {
    padding: 5px 10px;
    white-space: nowrap;

    &:hover {
        background: var(--glass-3);
        color: var(--text-0);
    }
}

.home-srv-del {
    width: 20px;
    height: 20px;
    line-height: 1;
    flex: none;
    opacity: 0.5;

    &:hover {
        opacity: 1;
        color: var(--c-bad, #ff453a);
    }
}

.home-srv-empty {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-3);
}

.home-srv-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
}

.home-srv-row {
    display: grid;
    grid-template-columns: 62px minmax(0, 1fr) auto 20px;
    align-items: center;
    gap: 8px;
    padding: 5px 7px;
    border-radius: 9px;
    background: var(--glass-1);
    border: 1px solid var(--stroke-1);
}

.home-srv-ping {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);

    i {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: currentColor;
        box-shadow: 0 0 7px currentColor;
        animation: srv-pulse 2.4s ease-in-out infinite;
    }

    &.is-good {
        color: var(--c-ok, #4fb27a);
    }
    &.is-fair {
        color: var(--c-warn, #e0a13a);
    }
    &.is-slow {
        color: var(--c-bad, #ff453a);
    }
    &.is-unknown {
        color: var(--text-3);
    }
}

.home-srv-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
}

.home-srv-icon {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    margin-right: 5px;
    vertical-align: -3px;
    object-fit: cover;

    &.is-fallback {
        display: inline-grid;
        place-items: center;
        background: var(--glass-3);
        color: var(--text-2);
        font-size: 9px;
        font-style: normal;
        font-weight: 700;
    }
}

.home-srv-host {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.home-srv-motd {
    font-size: 10.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.home-srv-players {
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--text-1);
    white-space: nowrap;

    .sep {
        opacity: 0.4;
        margin: 0 1px;
    }
}

.home-srv-blocked {
    font-size: 9.5px;
    font-style: normal;
    color: var(--text-3);
}

@keyframes srv-pulse {
    0%,
    100% {
        opacity: 0.55;
    }
    50% {
        opacity: 1;
    }
}

@media (prefers-reduced-motion: reduce) {
    .home-srv-ping i {
        animation: none;
    }
}
</style>
