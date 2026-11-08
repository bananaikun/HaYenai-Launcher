<script setup lang="ts">
import { CopyIcon, PlusIcon, SendIcon, ShareIcon, TrashIcon, UsersIcon } from '@modrinth/assets'
import { StyledInput, defineMessages, useVIntl } from '@modrinth/ui'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import {
    ROOM_PORT,
    roomCreate,
    roomGetSettings,
    roomHostInfo,
    roomHostStart,
    roomHostStop,
    roomJoin,
    roomList,
    roomMembers,
    roomPlugins,
    roomPull,
    roomRemove,
    roomSaveSettings,
    roomSend,
    roomShareFile,
    type HostInfo,
    type LanStatus,
    type PluginInfo,
    type RoomConfig,
    type RoomMember,
    type RoomMessage,
    type RoomSettings,
    lanEnv,
    lanStart,
    lanStatus,
    lanStop,
    type LanEnv,
} from '@/helpers/room'
import { completeWithAIPreset, listAIPresets, type AIPresetState } from '@/helpers/ai'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: { id: 'app.rooms.title', defaultMessage: 'Rooms' },
    subtitle: {
        id: 'app.rooms.subtitle',
        defaultMessage: 'Chat, share packs and files, all on your own network.',
    },
    create: { id: 'app.rooms.create', defaultMessage: 'New room' },
    empty: { id: 'app.rooms.empty', defaultMessage: 'No rooms yet. Create one to start.' },
    roomName: { id: 'app.rooms.name', defaultMessage: 'Room name' },
    yourName: { id: 'app.rooms.your-name', defaultMessage: 'Your name' },
    join: { id: 'app.rooms.join', defaultMessage: 'Join with code' },
    code: { id: 'app.rooms.code', defaultMessage: 'Invite code' },
    codeHint: {
        id: 'app.rooms.code-hint',
        defaultMessage: '32 characters from the host. It is also the network key.',
    },
    open: { id: 'app.rooms.open', defaultMessage: 'Start hosting' },
    close: { id: 'app.rooms.close', defaultMessage: 'Stop hosting' },
    connecting: { id: 'app.rooms.connecting', defaultMessage: 'Connecting…' },
    waitingNetwork: {
        id: 'app.rooms.waiting-network',
        defaultMessage: 'Waiting for the virtual network…',
    },
    members: { id: 'app.rooms.members', defaultMessage: 'Members' },
    settings: { id: 'app.rooms.settings', defaultMessage: 'Room settings' },
    plugins: { id: 'app.rooms.plugins', defaultMessage: 'Modules' },
    bot: { id: 'app.rooms.bot', defaultMessage: 'AI bot' },
    botName: { id: 'app.rooms.bot-name', defaultMessage: 'Bot name' },
    botPreset: { id: 'app.rooms.bot-preset', defaultMessage: 'Preset' },
    botMention: { id: 'app.rooms.bot-mention', defaultMessage: 'Only reply when mentioned' },
    qq: { id: 'app.rooms.qq', defaultMessage: 'QQ group sync' },
    qqGroup: { id: 'app.rooms.qq-group', defaultMessage: 'Group number' },
    qqPending: {
        id: 'app.rooms.qq-pending',
        defaultMessage: 'Saved, but not active yet in this version.',
    },
    placeholder: { id: 'app.rooms.placeholder', defaultMessage: 'Message the room…' },
    send: { id: 'app.rooms.send', defaultMessage: 'Send' },
    shareFile: { id: 'app.rooms.share-file', defaultMessage: 'Share file' },
    sharePack: { id: 'app.rooms.share-pack', defaultMessage: 'Share mod list' },
    copied: { id: 'app.rooms.copied', defaultMessage: 'Invite code copied' },
    hostAddress: { id: 'app.rooms.host-address', defaultMessage: 'Host address' },
    adminNeeded: {
        id: 'app.rooms.admin-needed',
        defaultMessage: 'The virtual network needs administrator rights to create its adapter.',
    },
    note: { id: 'app.rooms.note', defaultMessage: 'Note' },
    noPreset: { id: 'app.rooms.no-preset', defaultMessage: 'Create a preset first' },
    system: { id: 'app.rooms.system', defaultMessage: 'System' },
})

/** 房主的固定虚拟地址。成员靠它找到服务，所以不能交给 DHCP 随机分。 */
const HOST_VIRTUAL_IP = '10.126.126.1'

const rooms = ref<RoomConfig[]>([])
const activeRoom = ref<RoomConfig | null>(null)
const chat = ref<RoomMessage[]>([])
const members = ref<RoomMember[]>([])
const host = ref<HostInfo>({ running: false, roomId: null, addr: null })
const lan = ref<LanStatus | null>(null)
const lanInfo = ref<LanEnv | null>(null)
const plugins = ref<PluginInfo[]>([])
const presetState = ref<AIPresetState>({ presets: [], activeId: null })
const settings = ref<RoomSettings>({ plugins: {} })
const draft = ref('')
const busy = ref(false)
const notice = ref<string | null>(null)

/** `null` = 房主视角；有值 = 成员视角，值就是连房主的 base URL。 */
const memberBase = ref<string | null>(null)
const memberCode = ref<string | null>(null)
const myName = ref('')
const joinCode = ref('')

let pollTimer: number | null = null
let lastId = 0

const isHost = computed(() => memberBase.value === null)
/** 当前房间标识。成员没有本地房间记录，用邀请码派生一个稳定的键。 */
const activeRoomId = computed(
    () =>
        activeRoom.value?.id ??
        (memberCode.value ? `member-${memberCode.value.slice(0, 16)}` : ""),
)
const online = computed(() => host.value.running || memberBase.value !== null)
const canSend = computed(() => online.value && draft.value.trim().length > 0)
const bot = computed(() => settings.value.bot ?? null)
const qq = computed(() => settings.value.qq ?? null)

/**
 * EasyTier 的网络名从邀请码派生，不用房间 id。
 *
 * 这样成员**只需要邀请码**就能进同一个网络 —— 房间 id 是房主本地的 uuid，
 * 成员根本拿不到它。
 */
function networkNameOf(code: string): string {
    return `hy-${code.slice(0, 16)}`
}

function report(message: string) {
    notice.value = message
    window.setTimeout(() => {
        if (notice.value === message) notice.value = null
    }, 4200)
}

function errorText(error: unknown): string {
    if (typeof error === 'string') return error
    if (error && typeof error === 'object') {
        const value = (error as { message?: unknown }).message
        if (typeof value === 'string' && value.trim()) return value
    }
    return String(error)
}

async function refreshRooms() {
    try {
        rooms.value = await roomList()
        if (!activeRoom.value && rooms.value.length) activeRoom.value = rooms.value[0]
    } catch (error) {
        report(errorText(error))
    }
}

async function loadRoom(room: RoomConfig) {
    activeRoom.value = room
    settings.value = await roomGetSettings(room.id).catch(() => ({ plugins: {} }))
    await refreshChat(true)
}

async function refreshChat(reset = false) {
    try {
        const rows = await roomPull(
            activeRoomId.value,
            reset ? 0 : lastId,
            memberBase.value,
            memberCode.value,
        )
        if (reset) chat.value = []
        if (rows.length) {
            chat.value = [...chat.value, ...rows].slice(-500)
            lastId = rows[rows.length - 1].id
            void maybeBotReply(rows)
        }
    } catch (error) {
        // 轮询失败不打断用户：房主可能刚关，等着下次重试就行。
        report(errorText(error))
    }
}

async function createRoom() {
    const name = window.prompt(formatMessage(messages.roomName), '')
    if (!name?.trim()) return
    const who = myName.value.trim() || '房主'
    try {
        const room = await roomCreate(name.trim(), who)
        await refreshRooms()
        await loadRoom(room)
    } catch (error) {
        report(errorText(error))
    }
}

async function removeRoom(room: RoomConfig) {
    if (!window.confirm(`删除房间「${room.name}」？聊天记录会一起删掉。`)) return
    try {
        await roomRemove(room.id)
        if (activeRoom.value?.id === room.id) {
            activeRoom.value = null
            chat.value = []
        }
        await refreshRooms()
    } catch (error) {
        report(errorText(error))
    }
}

/** 开房：先组网拿虚拟地址，再在虚拟地址上起服务。顺序反了服务绑不上。 */
async function startHosting() {
    const room = activeRoom.value
    if (!room) return
    busy.value = true
    try {
        await lanStart({
            networkName: networkNameOf(room.code),
            networkSecret: room.code,
            peers: [],
            virtualIp: HOST_VIRTUAL_IP,
            role: 'host',
        })
        memberBase.value = null
        memberCode.value = null

        // 等网卡起来：提权进程是异步拉起的，只能轮询。
        let virtualIp: string | null = null
        for (let attempt = 0; attempt < 20; attempt += 1) {
            await new Promise((resolve) => window.setTimeout(resolve, 1500))
            lan.value = await lanStatus()
            if (lan.value.virtualIp) {
                virtualIp = lan.value.virtualIp
                break
            }
        }
        if (!virtualIp) {
            report(lanInfo.value?.isAdmin ? '网络没起来，看看是否被安全软件拦了' : formatMessage(messages.adminNeeded))
            return
        }

        const addr = await roomHostStart(room.id, virtualIp, ROOM_PORT)
        host.value = await roomHostInfo()
        await roomJoin(room.id, myName.value.trim() || '房主', 'host', '')
        members.value = await roomMembers(room.id)
        await refreshChat(true)
        startPolling()
        report(`房间已开启：${addr}`)
    } catch (error) {
        report(errorText(error))
    } finally {
        busy.value = false
    }
}

async function stopHosting() {
    try {
        await roomHostStop()
        await lanStop()
        host.value = { running: false, roomId: null, addr: null }
        lan.value = null
        stopPolling()
    } catch (error) {
        report(errorText(error))
    }
}

/** 加入：只凭邀请码。房主地址是固定的虚拟地址，不需要用户填。 */
async function joinRoom() {
    const code = joinCode.value.trim()
    if (code.length < 16) {
        report('邀请码不对，完整的是 32 位')
        return
    }
    busy.value = true
    try {
        await lanStart({
            networkName: networkNameOf(code),
            networkSecret: code,
            peers: [],
            role: 'member',
        })
        let virtualIp: string | null = null
        for (let attempt = 0; attempt < 20; attempt += 1) {
            await new Promise((resolve) => window.setTimeout(resolve, 1500))
            lan.value = await lanStatus()
            if (lan.value.virtualIp) {
                virtualIp = lan.value.virtualIp
                break
            }
        }
        if (!virtualIp) {
            report(formatMessage(messages.adminNeeded))
            return
        }
        memberBase.value = `http://${HOST_VIRTUAL_IP}:${ROOM_PORT}`
        memberCode.value = code
        lastId = 0
        chat.value = []
        await roomJoin(activeRoomId.value, myName.value.trim() || '玩家', 'offline', '')
        await refreshChat(true)
        startPolling()
        if (!chat.value.length) report('已连上，但还没拉到消息 —— 如果一直这样，确认房主开着房间')
    } catch (error) {
        report(errorText(error))
    } finally {
        busy.value = false
    }
}

async function leaveRoom() {
    await lanStop().catch(() => undefined)
    memberBase.value = null
    memberCode.value = null
    stopPolling()
    chat.value = []
    lastId = 0
}

function startPolling() {
    stopPolling()
    // 2 秒一轮。房间是聊天室不是行情，再快只是白耗电。
    pollTimer = window.setInterval(() => void refreshChat(), 2000)
}

function stopPolling() {
    if (pollTimer !== null) {
        window.clearInterval(pollTimer)
        pollTimer = null
    }
}

/**
 * 房主侧的机器人回复。
 *
 * 只在房主机器上跑一次 —— 每个成员都跑的话，同一条消息会被回 N 遍。
 * 用 `from === bot.name` 跳过自己的回复，否则会自己跟自己聊起来。
 */
async function maybeBotReply(incoming: RoomMessage[]) {
    const cfg = settings.value.bot
    if (!cfg || !cfg.presetId || !isHost.value || !host.value.running) return
    for (const message of incoming) {
        if (message.from === cfg.name) continue
        const text = message.text.trim()
        const prefix = `@${cfg.name}`
        if (cfg.mentionOnly && !text.startsWith(prefix)) continue
        const prompt = cfg.mentionOnly ? text.slice(prefix.length).trim() : text
        if (!prompt) continue
        try {
            const reply = await completeWithAIPreset(cfg.presetId, prompt)
            if (!reply.trim()) continue
            await roomSend({
                roomId: activeRoomId.value,
                from: cfg.name,
                text: reply.trim(),
                base: null,
                code: null,
            })
            await refreshChat()
        } catch (error) {
            report(`机器人没答上：${errorText(error)}`)
        }
    }
}

async function send() {
    const text = draft.value.trim()
    if (!text || !online.value) return
    draft.value = ''
    try {
        await roomSend({
            roomId: activeRoomId.value,
            from: myName.value.trim() || (isHost.value ? '房主' : '玩家'),
            text,
            base: memberBase.value,
            code: memberCode.value,
        })
        await refreshChat()
    } catch (error) {
        report(errorText(error))
        draft.value = text
    }
}

async function copyCode() {
    const code = memberCode.value ?? activeRoom.value?.code
    if (!code) return
    try {
        await navigator.clipboard.writeText(code)
        report(formatMessage(messages.copied))
    } catch {
        report(code)
    }
}

async function shareFile() {
    const room = activeRoom.value
    if (!room) return
    const picked = await open({ multiple: false, title: formatMessage(messages.shareFile) })
    if (typeof picked !== 'string') return
    try {
        await roomShareFile(
            room.id,
            picked,
            myName.value.trim() || '玩家',
            memberBase.value,
            memberCode.value,
        )
        await refreshChat()
    } catch (error) {
        report(errorText(error))
    }
}

async function persistSettings() {
    const room = activeRoom.value
    if (!room) return
    try {
        settings.value = await roomSaveSettings(room.id, settings.value)
        report('设置已保存')
    } catch (error) {
        report(errorText(error))
    }
}

function togglePlugin(id: string) {
    settings.value = {
        ...settings.value,
        plugins: { ...settings.value.plugins, [id]: !settings.value.plugins[id] },
    }
    void persistSettings()
}

function fileUrl(message: RoomMessage): string | null {
    const name = (message.payload?.name as string | undefined) ?? message.text
    if (!name) return null
    const base = memberBase.value ?? (host.value.addr ? `http://${host.value.addr}` : null)
    if (!base) return null
    const code = memberCode.value ?? activeRoom.value?.code ?? ''
    return `${base}/file?k=${encodeURIComponent(code)}&name=${encodeURIComponent(name)}`
}

watch(activeRoom, (room) => {
    if (room) void loadRoom(room)
})

onMounted(async () => {
    await Promise.all([refreshRooms(), roomPlugins().then((rows) => (plugins.value = rows))])
    lanInfo.value = await lanEnv().catch(() => null)
    host.value = await roomHostInfo().catch(() => ({ running: false, roomId: null, addr: null }))
    presetState.value = await listAIPresets().catch(() => ({ presets: [], activeId: null }))
    if (host.value.running) startPolling()
})

onBeforeUnmount(() => {
    stopPolling()
})
</script>

<template>
    <div class="rm">
        <aside class="rm-list">
            <div class="rm-list-head">
                <h1>{{ formatMessage(messages.title) }}</h1>
                <button type="button" class="rm-mini" @click="createRoom">
                    <PlusIcon class="size-4" />
                </button>
            </div>
            <p class="rm-sub">{{ formatMessage(messages.subtitle) }}</p>

            <p v-if="!rooms.length" class="rm-hint">{{ formatMessage(messages.empty) }}</p>
            <button
                v-for="room in rooms"
                :key="room.id"
                type="button"
                class="rm-item"
                :class="{ on: activeRoom?.id === room.id }"
                @click="loadRoom(room)"
            >
                <span class="rm-item-name">{{ room.name }}</span>
                <span class="rm-item-meta">{{ room.hostName }}</span>
                <span
                    class="rm-item-del"
                    @click.stop="removeRoom(room)"
                >
                    <TrashIcon class="size-3.5" />
                </span>
            </button>

            <div class="rm-join">
                <label>{{ formatMessage(messages.join) }}</label>
                <StyledInput v-model="myName" :placeholder="formatMessage(messages.yourName)" />
                <StyledInput v-model="joinCode" :placeholder="formatMessage(messages.code)" />
                <button type="button" class="rm-primary" :disabled="busy" @click="joinRoom">
                    {{ formatMessage(messages.join) }}
                </button>
            </div>
        </aside>

        <main class="rm-main">
            <header class="rm-head">
                <div class="rm-head-title">
                    <strong>{{ activeRoom?.name ?? formatMessage(messages.title) }}</strong>
                    <button
                        v-if="activeRoom"
                        type="button"
                        class="rm-code"
                        v-tooltip="formatMessage(messages.code)"
                        @click="copyCode"
                    >
                        <CopyIcon class="size-3" />
                        <code>{{ (memberCode ?? activeRoom.code).slice(0, 12) }}…</code>
                    </button>
                </div>
                <div class="rm-head-actions">
                    <span v-if="lan?.virtualIp" class="rm-pill">{{ lan.virtualIp }}</span>
                    <button
                        v-if="isHost && !host.running"
                        type="button"
                        class="rm-primary"
                        :disabled="busy || !activeRoom"
                        @click="startHosting"
                    >
                        {{ formatMessage(busy ? messages.connecting : messages.open) }}
                    </button>
                    <button
                        v-else-if="isHost"
                        type="button"
                        class="rm-ghost"
                        @click="stopHosting"
                    >
                        {{ formatMessage(messages.close) }}
                    </button>
                    <button v-else type="button" class="rm-ghost" @click="leaveRoom">
                        {{ formatMessage(messages.close) }}
                    </button>
                </div>
            </header>

            <p v-if="notice" class="rm-notice">{{ notice }}</p>

            <div class="rm-chat">
                <p v-if="!chat.length" class="rm-chat-empty">
                    {{ online ? formatMessage(messages.placeholder) : formatMessage(messages.waitingNetwork) }}
                </p>
                <div v-for="message in chat" :key="message.id" class="rm-msg">
                    <div class="rm-msg-head">
                        <strong>{{ message.from }}</strong>
                        <span>{{ new Date(message.ts).toLocaleTimeString() }}</span>
                    </div>
                    <a
                        v-if="message.kind === 'file' && fileUrl(message)"
                        :href="fileUrl(message) ?? '#'"
                        target="_blank"
                        rel="noopener"
                        class="rm-msg-file"
                    >
                        {{ (message.payload?.name as string) ?? message.text }}
                    </a>
                    <div v-else-if="message.kind === 'pack'" class="rm-msg-pack">
                        <div class="rm-msg-pack-title">
                            {{ (message.payload?.instance as string) ?? message.text }}
                        </div>
                        <div class="rm-msg-pack-meta">
                            {{ (message.payload?.gameVersion as string) ?? '' }} ·
                            {{ (message.payload?.modCount as number) ?? 0 }} 个模组
                        </div>
                        <details>
                            <summary>清单</summary>
                            <ul>
                                <li
                                    v-for="mod in (message.payload?.mods as { name: string }[]) ?? []"
                                    :key="mod.name"
                                >
                                    {{ mod.name }}
                                </li>
                            </ul>
                        </details>
                    </div>
                    <div v-else class="rm-msg-text">{{ message.text }}</div>
                </div>
            </div>

            <footer class="rm-compose">
                <button type="button" class="rm-icon" v-tooltip="formatMessage(messages.shareFile)" @click="shareFile">
                    <ShareIcon class="size-4" />
                </button>
                <textarea
                    v-model="draft"
                    :placeholder="formatMessage(messages.placeholder)"
                    rows="1"
                    @keydown.enter.exact.prevent="send"
                />
                <button type="button" class="rm-icon primary" :disabled="!canSend" @click="send">
                    <SendIcon class="size-4" />
                </button>
            </footer>
        </main>

        <aside class="rm-side">
            <section>
                <h2><UsersIcon class="size-3.5" /> {{ formatMessage(messages.members) }}</h2>
                <p v-if="!members.length" class="rm-hint">—</p>
                <div v-for="member in members" :key="member.name" class="rm-member">
                    <strong>{{ member.name }}</strong>
                    <span v-if="member.note">{{ member.note }}</span>
                </div>
            </section>

            <section v-if="isHost && activeRoom">
                <h2>{{ formatMessage(messages.plugins) }}</h2>
                <label v-for="plugin in plugins" :key="plugin.id" class="rm-check">
                    <input
                        type="checkbox"
                        :checked="!!settings.plugins[plugin.id]"
                        @change="togglePlugin(plugin.id)"
                    />
                    <span>{{ plugin.label }}</span>
                </label>

                <h2>{{ formatMessage(messages.bot) }}</h2>
                <StyledInput
                    :model-value="bot?.name ?? ''"
                    :placeholder="formatMessage(messages.botName)"
                    @update:model-value="
                        (value) => {
                            settings = {
                                ...settings,
                                bot: { name: value, presetId: bot?.presetId ?? '', mentionOnly: bot?.mentionOnly ?? true },
                            }
                        }
                    "
                />
                <select
                    :value="bot?.presetId ?? ''"
                    class="rm-select"
                    @change="
                        (event) => {
                            settings = {
                                ...settings,
                                bot: {
                                    name: bot?.name ?? '',
                                    presetId: (event.target as HTMLSelectElement).value,
                                    mentionOnly: bot?.mentionOnly ?? true,
                                },
                            }
                        }
                    "
                >
                    <option value="">{{ formatMessage(messages.noPreset) }}</option>
                    <option v-for="preset in presetState.presets" :key="preset.id" :value="preset.id">
                        {{ preset.name }}
                    </option>
                </select>
                <label class="rm-check">
                    <input
                        type="checkbox"
                        :checked="bot?.mentionOnly ?? true"
                        @change="
                            () => {
                                if (!bot) return
                                settings = { ...settings, bot: { ...bot, mentionOnly: !bot.mentionOnly } }
                            }
                        "
                    />
                    <span>{{ formatMessage(messages.botMention) }}</span>
                </label>

                <h2>{{ formatMessage(messages.qq) }}</h2>
                <label class="rm-check">
                    <input
                        type="checkbox"
                        :checked="qq?.enabled ?? false"
                        @change="
                            () => {
                                settings = {
                                    ...settings,
                                    qq: { enabled: !(qq?.enabled ?? false), groupId: qq?.groupId ?? '' },
                                }
                            }
                        "
                    />
                    <span>{{ formatMessage(messages.qq) }}</span>
                </label>
                <StyledInput
                    :model-value="qq?.groupId ?? ''"
                    :placeholder="formatMessage(messages.qqGroup)"
                    @update:model-value="
                        (value) => {
                            settings = { ...settings, qq: { enabled: qq?.enabled ?? false, groupId: value } }
                        }
                    "
                />
                <p class="rm-hint">{{ formatMessage(messages.qqPending) }}</p>

                <button type="button" class="rm-primary wide" @click="persistSettings">
                    {{ formatMessage(messages.settings) }}
                </button>
            </section>
        </aside>
    </div>
</template>

<style scoped>
.rm {
    display: flex;
    gap: 0.75rem;
    height: 100%;
    min-height: 0;
    padding: 1.25rem 1.5rem;
    box-sizing: border-box;
}

.rm-list {
    width: 16rem;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    overflow-y: auto;
}

.rm-list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.rm-list-head h1 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 600;
    color: var(--color-text-primary);
}

.rm-sub {
    margin: 0;
    font-size: 0.74rem;
    line-height: 1.4;
    color: var(--color-text-tertiary);
}

.rm-mini {
    display: grid;
    place-items: center;
    width: 1.9rem;
    height: 1.9rem;
    border-radius: 0.5rem;
    border: 1px solid var(--color-divider);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.rm-item {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.5rem 0.65rem;
    border-radius: 0.55rem;
    border: 1px solid transparent;
    background: transparent;
    text-align: left;
    cursor: pointer;
    color: var(--color-text-primary);
}

.rm-item.on {
    background: var(--surface-3, rgba(255, 255, 255, 0.07));
    border-color: var(--color-divider);
}

.rm-item-name {
    font-size: 0.85rem;
    font-weight: 600;
}

.rm-item-meta {
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}

.rm-item-del {
    position: absolute;
    right: 0.4rem;
    top: 50%;
    transform: translateY(-50%);
    color: var(--color-text-tertiary);
    opacity: 0;
    transition: opacity 0.15s;
}

.rm-item:hover .rm-item-del {
    opacity: 1;
}

.rm-join {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding-top: 0.75rem;
    border-top: 1px solid var(--color-divider);
}

.rm-join label {
    font-size: 0.74rem;
    color: var(--color-text-tertiary);
}

.rm-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}

.rm-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-shrink: 0;
}

.rm-head-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    margin-right: auto;
}

.rm-head-title strong {
    font-size: 1rem;
    color: var(--color-text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.rm-code {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    border: 1px solid var(--color-divider);
    background: transparent;
    border-radius: 999px;
    padding: 0.15rem 0.5rem;
    color: var(--color-text-tertiary);
    cursor: pointer;
    font-size: 0.7rem;
}

.rm-head-actions {
    display: flex;
    align-items: center;
    gap: 0.4rem;
}

.rm-pill {
    font-family: var(--font-mono, monospace);
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
    border: 1px solid var(--color-divider);
    border-radius: 999px;
    padding: 0.12rem 0.45rem;
}

.rm-primary {
    border: none;
    border-radius: 0.5rem;
    background: var(--color-brand);
    color: #fff;
    font-size: 0.78rem;
    padding: 0.42rem 0.8rem;
    cursor: pointer;
}

.rm-primary:disabled {
    opacity: 0.45;
    cursor: default;
}

.rm-primary.wide {
    width: 100%;
    margin-top: 0.5rem;
}

.rm-ghost {
    border: 1px solid var(--color-divider);
    background: transparent;
    color: var(--color-text-secondary);
    border-radius: 0.5rem;
    font-size: 0.78rem;
    padding: 0.4rem 0.75rem;
    cursor: pointer;
}

.rm-notice {
    margin: 0;
    font-size: 0.76rem;
    color: #f0b429;
}

.rm-chat {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
    padding-right: 0.25rem;
}

.rm-chat-empty {
    margin: auto;
    font-size: 0.82rem;
    color: var(--color-text-tertiary);
}

.rm-msg {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    max-width: 44rem;
}

.rm-msg-head {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
}

.rm-msg-head strong {
    font-size: 0.78rem;
    color: var(--color-text-primary);
}

.rm-msg-head span {
    font-size: 0.66rem;
    color: var(--color-text-tertiary);
}

.rm-msg-text {
    font-size: 0.86rem;
    line-height: 1.5;
    color: var(--color-text-primary);
    white-space: pre-wrap;
    word-break: break-word;
}

.rm-msg-file {
    font-size: 0.82rem;
    color: var(--color-brand);
    word-break: break-all;
}

.rm-msg-pack {
    border: 1px solid var(--color-divider);
    border-radius: 0.55rem;
    padding: 0.45rem 0.6rem;
    font-size: 0.78rem;
    color: var(--color-text-secondary);
}

.rm-msg-pack-title {
    font-weight: 600;
    color: var(--color-text-primary);
}

.rm-msg-pack ul {
    margin: 0.25rem 0 0;
    padding-left: 1rem;
    max-height: 12rem;
    overflow: auto;
}

.rm-compose {
    display: flex;
    align-items: flex-end;
    gap: 0.4rem;
    flex-shrink: 0;
}

.rm-compose textarea {
    flex: 1;
    resize: none;
    min-height: 2.4rem;
    max-height: 7rem;
    padding: 0.55rem 0.75rem;
    border-radius: 0.65rem;
    border: 1px solid var(--color-divider);
    background: var(--surface-3, rgba(255, 255, 255, 0.07));
    color: var(--color-text-primary);
    font-family: inherit;
    font-size: 0.85rem;
}

.rm-compose textarea:focus {
    outline: none;
    border-color: rgba(var(--color-brand-rgb, 120, 160, 255), 0.5);
}

.rm-icon {
    display: grid;
    place-items: center;
    width: 2.4rem;
    height: 2.4rem;
    flex-shrink: 0;
    border-radius: 0.65rem;
    border: 1px solid var(--color-divider);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.rm-icon.primary {
    border: none;
    background: var(--color-brand);
    color: #fff;
}

.rm-icon:disabled {
    opacity: 0.4;
    cursor: default;
}

.rm-side {
    width: 15rem;
    flex-shrink: 0;
    overflow-y: auto;
    border-left: 1px solid var(--color-divider);
    padding-left: 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

.rm-side h2 {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    margin: 0 0 0.4rem;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--color-text-tertiary);
}

.rm-member {
    display: flex;
    flex-direction: column;
    font-size: 0.78rem;
    color: var(--color-text-primary);
}

.rm-member span {
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.rm-check {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
    color: var(--color-text-secondary);
    margin-bottom: 0.25rem;
    cursor: pointer;
}

.rm-select {
    width: 100%;
    /* 原生下拉默认跟随系统深色主题，在浅色壁纸上会渲染成一块深灰。
       显式声明浅色，让它和旁边的输入框一致。 */
    color-scheme: light;
    margin: 0.3rem 0;
    padding: 0.38rem 0.5rem;
    border-radius: 0.5rem;
    border: 1px solid var(--color-divider);
    background: var(--surface-3, rgba(255, 255, 255, 0.07));
    color: var(--color-text-primary);
    font-size: 0.78rem;
}

.rm-hint {
    margin: 0.2rem 0 0;
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}
</style>
