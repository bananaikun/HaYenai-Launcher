<script setup lang="ts">
import {
    ChevronLeftIcon,
    ChevronRightIcon,
    CopyIcon,
    DownloadIcon,
    FileIcon,
    PlusIcon,
    SendIcon,
    ShareIcon,
    TrashIcon,
} from '@modrinth/assets'
import {
    Button,
    Combobox,
    IconButton,
    NewModal,
    StyledInput,
    Tabs,
    Toggle,
    defineMessages,
    useVIntl,
    type TabsValue,
} from '@modrinth/ui'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

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
    roomMyName,
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
    lanEnsure,
    lanStatus,
    lanStop,
    type LanEnv,
} from '@/helpers/room'
import { completeWithAIPreset, listAIPresets, type AIPresetState } from '@/helpers/ai'
import { useComposerOffset } from '@/helpers/composer-offset'
import { readGlobalModel } from '@/helpers/ai-model'

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
    noRoom: { id: 'app.rooms.no-room', defaultMessage: 'No room selected' },
    online: { id: 'app.rooms.online', defaultMessage: 'Online' },
    offline: { id: 'app.rooms.offline', defaultMessage: 'Offline' },
    toggleSide: { id: 'app.rooms.toggle-side', defaultMessage: 'Toggle details' },
    remove: { id: 'app.rooms.remove', defaultMessage: 'Delete room' },
    list: { id: 'app.rooms.list', defaultMessage: 'Contents' },
    today: { id: 'app.rooms.today', defaultMessage: 'Today' },
    yesterday: { id: 'app.rooms.yesterday', defaultMessage: 'Yesterday' },
    reconnecting: { id: 'app.rooms.reconnecting', defaultMessage: 'Reconnecting…' },
    disconnected: { id: 'app.rooms.disconnected', defaultMessage: 'Disconnected' },
    memberCount: { id: 'app.rooms.member-count', defaultMessage: '{count} member(s)' },
    noAccount: { id: 'app.rooms.no-account', defaultMessage: 'No account signed in' },
    accountHint: {
        id: 'app.rooms.account-hint',
        defaultMessage: 'Uses the account signed in to the launcher',
    },
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
        activeRoom.value?.id ?? (memberCode.value ? `member-${memberCode.value.slice(0, 16)}` : ''),
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
    // 成员也要跟着换房间重新拉，否则右栏会一直显示上一个房间的人。
    members.value = await roomMembers(room.id).catch(() => [])
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

async function createRoom(rawName: string) {
    const name = rawName.trim()
    if (!name) return
    const who = myName.value.trim() || '房主'
    try {
        const room = await roomCreate(name, who)
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
/**
 * 等虚拟网卡真的起来。提权拉起的进程是异步的，没有回调，只能轮询。
 * 返回拿到的地址；超时返回 null。
 */
async function waitForVirtualIp(attempts: number): Promise<string | null> {
    for (let attempt = 0; attempt < attempts; attempt += 1) {
        await new Promise((resolve) => window.setTimeout(resolve, 1500))
        lan.value = await lanStatus()
        if (lan.value.virtualIp) return lan.value.virtualIp
    }
    return null
}

/** 成员固定地址的本地缓存：按网络名存，下次进同一房间直接用。 */
function ipCacheKey(code: string): string {
    return `hayenai.room.ip.${networkNameOf(code)}`
}

async function startHosting() {
    const room = activeRoom.value
    if (!room) return
    busy.value = true
    try {
        // 幂等启动：同一个房间反复点「开启」不会重复提权（进程常驻，UAC 只来一次）。
        await lanEnsure({
            networkName: networkNameOf(room.code),
            networkSecret: room.code,
            peers: [],
            virtualIp: HOST_VIRTUAL_IP,
            role: 'host',
        })
        memberBase.value = null
        memberCode.value = null

        const virtualIp = await waitForVirtualIp(20)
        if (!virtualIp) {
            report(
                lanInfo.value?.isAdmin
                    ? '网络没起来，看看是否被安全软件拦了'
                    : formatMessage(messages.adminNeeded),
            )
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
        const networkName = networkNameOf(code)
        // 上次已经分配到的固定地址（房主侧落盘 + 本地缓存），有就直接用。
        const cachedIp = localStorage.getItem(ipCacheKey(code))

        let virtualIp: string | null = null
        if (cachedIp) {
            try {
                await lanEnsure({
                    networkName,
                    networkSecret: code,
                    peers: [],
                    virtualIp: cachedIp,
                    role: 'member',
                })
                virtualIp = await waitForVirtualIp(10)
            } catch {
                // 地址被占用等情况下别把用户卡死在这里，清缓存走自动分配。
                virtualIp = null
            }
            if (!virtualIp) {
                localStorage.removeItem(ipCacheKey(code))
                await lanStop().catch(() => undefined)
            }
        }

        if (!virtualIp) {
            await lanEnsure({ networkName, networkSecret: code, peers: [], role: 'member' })
            virtualIp = await waitForVirtualIp(20)
        }
        if (!virtualIp) {
            report(formatMessage(messages.adminNeeded))
            return
        }

        memberBase.value = `http://${HOST_VIRTUAL_IP}:${ROOM_PORT}`
        memberCode.value = code
        lastId = 0
        chat.value = []

        // 先登记才能拿到分配；房主按加入顺序发固定尾号。
        await roomJoin(activeRoomId.value, myName.value.trim() || '玩家', 'offline', '')
        members.value = await roomMembers(activeRoomId.value).catch(() => [])

        const me = members.value.find((member) => member.name === (myName.value.trim() || '玩家'))
        if (me?.virtualIp) {
            localStorage.setItem(ipCacheKey(code), me.virtualIp)
            if (me.virtualIp !== virtualIp) {
                report(`房主分配了固定地址 ${me.virtualIp}，下次进房直接用它`)
            }
        }

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
    // 常驻：这里不停网。退房间只是离开界面，EasyTier 留在后台 —— 再进来就不用
    // 重新提权（UAC 只该出现一次）。真要断网用「停止托管」或直接退出启动器。
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
            // 机器人只用方案的「人设」，模型走全局设置。
            const model = readGlobalModel()
            const reply = await completeWithAIPreset(cfg.presetId, prompt, {
                providerId: model?.providerId,
                modelId: model?.modelId,
            })
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
    // 进程可能是上次会话留下来的（常驻），进页面先把真实状态读回来，
    // 否则会显示「离线」误导人。
    lan.value = await lanStatus().catch(() => null)
    host.value = await roomHostInfo().catch(() => ({ running: false, roomId: null, addr: null }))
    presetState.value = await listAIPresets().catch(() => ({ presets: [], activeId: null }))
    // 昵称直接取当前登录账号：手填一个和游戏账号无关的名字，日志里就对不上人。
    accountName.value = await roomMyName().catch(() => null)
    myName.value = accountName.value ?? ''
    if (host.value.running) startPolling()
})

onBeforeUnmount(() => {
    stopPolling()
})

/* ------------------------------------------------------------------ *
 * QQ 化布局（2026-11-14）
 * 三区：左 264 会话列表 / 中聊天流 / 右 280 可折叠信息栏。
 * 只加视图状态，不碰任何 room_* / lan_* 调用。
 * ------------------------------------------------------------------ */

const sideOpen = ref(true)
const sideTab = ref<TabsValue>('members')

const sideTabs = computed(() => {
    // 标签只放名字：280px 宽的右栏里三个中文 tab 再加「· 12」会被压到截断。
    // 人数放到列表头那一行。
    const tabs = [{ value: 'members', label: formatMessage(messages.members) }]
    if (isHost.value && activeRoom.value) {
        tabs.push({ value: 'modules', label: formatMessage(messages.plugins) })
        tabs.push({ value: 'bot', label: formatMessage(messages.bot) })
    }
    return tabs
})

const createModal = ref<InstanceType<typeof NewModal> | null>(null)
const joinModal = ref<InstanceType<typeof NewModal> | null>(null)
const newRoomName = ref('')
const threadEl = ref<HTMLElement | null>(null)
/** 当前登录的 Minecraft 账号。开房/加入都用它当昵称。 */
const accountName = ref<string | null>(null)
const composeEl = ref<HTMLElement | null>(null)

// 磁悬条是 fixed，房间页也有底部输入框 —— 报高度让它上移，别压在输入框上。
useComposerOffset(composeEl)

function openCreate(): void {
    newRoomName.value = ''
    createModal.value?.show()
}

async function submitCreate(): Promise<void> {
    const name = newRoomName.value.trim()
    if (!name) return
    createModal.value?.hide()
    await createRoom(name)
}

function openJoin(): void {
    joinCode.value = ''
    joinModal.value?.show()
}

async function submitJoin(): Promise<void> {
    joinModal.value?.hide()
    await joinRoom()
}

/** 网卡状态文案：常驻进程挂了会自动重连，这里要把这件事说出来。 */
const lanStateText = computed(() => {
    const status = lan.value
    if (!status) return formatMessage(messages.offline)
    if (status.gaveUp) return formatMessage(messages.disconnected)
    if (!status.running && status.resident) return formatMessage(messages.reconnecting)
    return formatMessage(status.running ? messages.online : messages.offline)
})

const lanStateOn = computed(() => !!lan.value?.running)

/** 邀请码：成员拿的是自己输入的那份，房主用房间的。 */
const inviteCode = computed(() => memberCode.value ?? activeRoom.value?.code ?? '')

const myDisplayName = computed(() => myName.value.trim() || (isHost.value ? '房主' : '玩家'))

const presetOptions = computed(() =>
    presetState.value.presets.map((preset) => ({ value: preset.id, label: preset.name })),
)

/** 自己发的靠右，别人靠左；系统消息单独居中。 */
function isMine(message: RoomMessage): boolean {
    return message.kind !== 'system' && message.from === myDisplayName.value
}

/**
 * 上一条同一个人、同类消息、且 5 分钟内 → 合并成一组，不重复头像与名字。
 * QQ 就这么分组，能明显压缩纵向高度。
 */
const GROUP_WINDOW_MS = 5 * 60 * 1000
function isGrouped(index: number): boolean {
    if (index === 0) return false
    const current = chat.value[index]
    const previous = chat.value[index - 1]
    if (!current || !previous) return false
    if (current.kind === 'system' || previous.kind === 'system') return false
    return (
        current.from === previous.from &&
        current.kind === previous.kind &&
        current.ts - previous.ts < GROUP_WINDOW_MS
    )
}

function avatarOf(name: string): string {
    const trimmed = (name ?? '').trim()
    return trimmed ? trimmed.slice(0, 1).toUpperCase() : '?'
}

function clock(ts: number): string {
    return new Date(ts).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
}

/** 会话列表那行的时间：今天只显示时:分，跨天显示 月/日。 */
function dayLabel(ts: number): string {
    const date = new Date(ts)
    const today = new Date()
    const sameDay =
        date.getFullYear() === today.getFullYear() &&
        date.getMonth() === today.getMonth() &&
        date.getDate() === today.getDate()
    return sameDay ? clock(ts) : `${date.getMonth() + 1}/${date.getDate()}`
}

/** 跨天时在消息流里插一条日期分隔。 */
function showDayDivider(index: number): boolean {
    if (index === 0) return true
    const current = new Date(chat.value[index]?.ts ?? 0)
    const previous = new Date(chat.value[index - 1]?.ts ?? 0)
    return current.toDateString() !== previous.toDateString()
}

function dayText(ts: number): string {
    const date = new Date(ts)
    const today = new Date()
    const yesterday = new Date(today.getTime() - 86400_000)
    if (date.toDateString() === today.toDateString()) return formatMessage(messages.today)
    if (date.toDateString() === yesterday.toDateString()) return formatMessage(messages.yesterday)
    return `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()}`
}

/** 新消息进来滚到底；自己刚发完也一样。 */
watch(
    () => chat.value.length,
    async () => {
        await nextTick()
        const el = threadEl.value
        if (el) el.scrollTop = el.scrollHeight
    },
)
</script>

<template>
    <div class="rooms">
        <div class="rooms-shell card">
            <!-- ============ 左区：会话列表 ============ -->
            <aside class="rooms-list">
                <header class="rooms-bar">
                    <h1 class="rooms-title">{{ formatMessage(messages.title) }}</h1>
                    <IconButton :label="formatMessage(messages.create)" @click="openCreate">
                        <PlusIcon />
                    </IconButton>
                </header>

                <div class="rooms-account">
                    <span class="rooms-avatar">{{ avatarOf(accountName ?? '?') }}</span>
                    <span class="rooms-account-text">
                        <strong>
                            {{ accountName ?? formatMessage(messages.noAccount) }}
                        </strong>
                        <span>{{ formatMessage(messages.accountHint) }}</span>
                    </span>
                </div>

                <div class="rooms-list-body">
                    <p v-if="!rooms.length" class="rooms-empty">
                        {{ formatMessage(messages.empty) }}
                    </p>
                    <button
                        v-for="room in rooms"
                        :key="room.id"
                        type="button"
                        class="rooms-item"
                        :class="{ on: activeRoom?.id === room.id }"
                        @click="loadRoom(room)"
                    >
                        <span class="rooms-avatar">{{ avatarOf(room.name) }}</span>
                        <span class="rooms-item-text">
                            <span class="rooms-item-name">{{ room.name }}</span>
                            <span class="rooms-item-meta">{{ room.hostName }}</span>
                        </span>
                        <time class="rooms-item-time">{{ dayLabel(room.createdAt) }}</time>
                        <span
                            class="rooms-item-del"
                            :title="formatMessage(messages.remove)"
                            @click.stop="removeRoom(room)"
                        >
                            <TrashIcon />
                        </span>
                    </button>
                </div>

                <footer class="rooms-list-foot">
                    <Button type="base" :disabled="!accountName" @click="openJoin">
                        {{ formatMessage(messages.join) }}
                    </Button>
                </footer>
            </aside>

            <!-- ============ 中区：聊天流 ============ -->
            <main class="rooms-main">
                <header class="rooms-bar">
                    <div class="rooms-head-main">
                        <strong class="rooms-head-name">
                            {{ activeRoom?.name ?? formatMessage(messages.noRoom) }}
                        </strong>
                        <button
                            v-if="inviteCode"
                            type="button"
                            class="rooms-code"
                            v-tooltip="formatMessage(messages.codeHint)"
                            @click="copyCode"
                        >
                            <CopyIcon />
                            <code>{{ inviteCode.slice(0, 8) }}…</code>
                        </button>
                    </div>

                    <div class="rooms-head-actions">
                        <span v-if="lan?.virtualIp" class="rooms-pill">{{ lan.virtualIp }}</span>
                        <span class="rooms-state" :class="{ on: lanStateOn }">
                            {{ lanStateText }}
                        </span>
                        <template v-if="isHost">
                            <Button
                                v-if="!host.running"
                                type="colored"
                                color="brand"
                                :disabled="busy || !activeRoom || !accountName"
                                @click="startHosting"
                            >
                                {{ formatMessage(busy ? messages.connecting : messages.open) }}
                            </Button>
                            <Button v-else type="base" @click="stopHosting">
                                {{ formatMessage(messages.close) }}
                            </Button>
                        </template>
                        <Button v-else type="base" @click="leaveRoom">
                            {{ formatMessage(messages.close) }}
                        </Button>
                        <IconButton
                            :label="formatMessage(messages.toggleSide)"
                            @click="sideOpen = !sideOpen"
                        >
                            <ChevronRightIcon v-if="sideOpen" />
                            <ChevronLeftIcon v-else />
                        </IconButton>
                    </div>
                </header>

                <p v-if="notice" class="rooms-notice">{{ notice }}</p>

                <div ref="threadEl" class="rooms-thread">
                    <p v-if="!chat.length" class="rooms-empty">
                        {{
                            online
                                ? formatMessage(messages.placeholder)
                                : formatMessage(messages.waitingNetwork)
                        }}
                    </p>

                    <template v-for="(message, index) in chat" :key="message.id">
                        <p v-if="showDayDivider(index)" class="rooms-day">
                            {{ dayText(message.ts) }}
                        </p>

                        <p v-if="message.kind === 'system'" class="rooms-system">
                            {{ message.text }}
                        </p>

                        <div
                            v-else
                            class="rooms-row"
                            :class="{ mine: isMine(message), grouped: isGrouped(index) }"
                        >
                            <span class="rooms-avatar">{{ avatarOf(message.from) }}</span>
                            <div class="rooms-msg">
                                <span v-if="!isGrouped(index)" class="rooms-msg-from">
                                    {{ message.from }}
                                    <time>{{ clock(message.ts) }}</time>
                                </span>

                                <a
                                    v-if="message.kind === 'file' && fileUrl(message)"
                                    :href="fileUrl(message) ?? '#'"
                                    target="_blank"
                                    rel="noopener"
                                    class="rooms-file"
                                >
                                    <FileIcon />
                                    <span>
                                        {{ (message.payload?.name as string) ?? message.text }}
                                    </span>
                                    <DownloadIcon />
                                </a>

                                <div v-else-if="message.kind === 'pack'" class="rooms-pack">
                                    <strong>
                                        {{ (message.payload?.instance as string) ?? message.text }}
                                    </strong>
                                    <span>
                                        {{ (message.payload?.gameVersion as string) ?? '' }} ·
                                        {{ (message.payload?.modCount as number) ?? 0 }} 个模组
                                    </span>
                                    <details>
                                        <summary>{{ formatMessage(messages.list) }}</summary>
                                        <ul>
                                            <li
                                                v-for="mod in (message.payload?.mods as {
                                                    name: string
                                                }[]) ?? []"
                                                :key="mod.name"
                                            >
                                                {{ mod.name }}
                                            </li>
                                        </ul>
                                    </details>
                                </div>

                                <div v-else class="rooms-bubble">{{ message.text }}</div>
                            </div>
                        </div>
                    </template>
                </div>

                <footer ref="composeEl" class="rooms-compose">
                    <IconButton
                        :label="formatMessage(messages.shareFile)"
                        :disabled="!activeRoom"
                        @click="shareFile"
                    >
                        <ShareIcon />
                    </IconButton>
                    <textarea
                        v-model="draft"
                        class="rooms-input"
                        :placeholder="formatMessage(messages.placeholder)"
                        rows="1"
                        @keydown.enter.exact.prevent="send"
                    />
                    <IconButton
                        type="colored"
                        color="brand"
                        :label="formatMessage(messages.send)"
                        :disabled="!canSend"
                        @click="send"
                    >
                        <SendIcon />
                    </IconButton>
                </footer>
            </main>

            <!-- ============ 右区：可折叠信息栏 ============ -->
            <aside class="rooms-side" :class="{ closed: !sideOpen }">
                <template v-if="sideOpen">
                    <div class="rooms-side-head">
                        <Tabs v-model:value="sideTab" :tabs="sideTabs" />
                    </div>

                    <div class="rooms-side-body">
                        <!-- 成员 -->
                        <template v-if="sideTab === 'members'">
                            <p class="rooms-side-count">
                                {{ formatMessage(messages.memberCount, { count: members.length }) }}
                            </p>
                            <p v-if="!members.length" class="rooms-empty">—</p>
                            <div v-for="member in members" :key="member.name" class="rooms-member">
                                <span class="rooms-avatar">{{ avatarOf(member.name) }}</span>
                                <span class="rooms-member-text">
                                    <strong>{{ member.name }}</strong>
                                    <span v-if="member.note">{{ member.note }}</span>
                                    <span v-if="member.virtualIp" class="rooms-member-ip">
                                        {{ member.virtualIp }}
                                    </span>
                                </span>
                            </div>
                        </template>

                        <!-- 模块 -->
                        <template v-else-if="sideTab === 'modules'">
                            <label v-for="plugin in plugins" :key="plugin.id" class="rooms-switch">
                                <span>{{ plugin.label }}</span>
                                <Toggle
                                    :model-value="!!settings.plugins[plugin.id]"
                                    @update:model-value="() => togglePlugin(plugin.id)"
                                />
                            </label>
                            <p v-if="!plugins.length" class="rooms-empty">—</p>
                        </template>

                        <!-- 机器人 / QQ -->
                        <template v-else>
                            <label class="rooms-field">
                                <span class="rooms-label">
                                    {{ formatMessage(messages.botName) }}
                                </span>
                                <StyledInput
                                    :model-value="bot?.name ?? ''"
                                    :placeholder="formatMessage(messages.botName)"
                                    @update:model-value="
                                        (value) =>
                                            (settings = {
                                                ...settings,
                                                bot: {
                                                    name: String(value),
                                                    presetId: bot?.presetId ?? '',
                                                    mentionOnly: bot?.mentionOnly ?? true,
                                                },
                                            })
                                    "
                                />
                            </label>

                            <label class="rooms-field">
                                <span class="rooms-label">
                                    {{ formatMessage(messages.botPreset) }}
                                </span>
                                <Combobox
                                    :model-value="bot?.presetId ?? ''"
                                    :options="presetOptions"
                                    :placeholder="formatMessage(messages.noPreset)"
                                    @update:model-value="
                                        (value) =>
                                            (settings = {
                                                ...settings,
                                                bot: {
                                                    name: bot?.name ?? '',
                                                    presetId: String(value),
                                                    mentionOnly: bot?.mentionOnly ?? true,
                                                },
                                            })
                                    "
                                />
                            </label>

                            <label class="rooms-switch">
                                <span>{{ formatMessage(messages.botMention) }}</span>
                                <Toggle
                                    :model-value="bot?.mentionOnly ?? true"
                                    @update:model-value="
                                        () => {
                                            if (!bot) return
                                            settings = {
                                                ...settings,
                                                bot: {
                                                    ...bot,
                                                    mentionOnly: !bot.mentionOnly,
                                                },
                                            }
                                        }
                                    "
                                />
                            </label>

                            <div class="rooms-divider" />

                            <label class="rooms-switch">
                                <span>{{ formatMessage(messages.qq) }}</span>
                                <Toggle
                                    :model-value="qq?.enabled ?? false"
                                    @update:model-value="
                                        () =>
                                            (settings = {
                                                ...settings,
                                                qq: {
                                                    enabled: !(qq?.enabled ?? false),
                                                    groupId: qq?.groupId ?? '',
                                                },
                                            })
                                    "
                                />
                            </label>
                            <StyledInput
                                :model-value="qq?.groupId ?? ''"
                                :placeholder="formatMessage(messages.qqGroup)"
                                @update:model-value="
                                    (value) =>
                                        (settings = {
                                            ...settings,
                                            qq: {
                                                enabled: qq?.enabled ?? false,
                                                groupId: String(value),
                                            },
                                        })
                                "
                            />
                            <p class="rooms-hint">{{ formatMessage(messages.qqPending) }}</p>

                            <Button type="base" @click="persistSettings">
                                {{ formatMessage(messages.settings) }}
                            </Button>
                        </template>
                    </div>
                </template>
            </aside>
        </div>

        <!-- ============ 新建房间 ============ -->
        <NewModal ref="createModal" :header="formatMessage(messages.create)" max-width="26rem">
            <div class="rooms-modal">
                <StyledInput
                    v-model="newRoomName"
                    :placeholder="formatMessage(messages.roomName)"
                    @keydown.enter.exact.prevent="submitCreate"
                />
                <Button type="colored" color="brand" @click="submitCreate">
                    {{ formatMessage(messages.create) }}
                </Button>
            </div>
        </NewModal>

        <!-- ============ 邀请码加入 ============ -->
        <NewModal ref="joinModal" :header="formatMessage(messages.join)" max-width="26rem">
            <div class="rooms-modal">
                <StyledInput
                    v-model="joinCode"
                    :placeholder="formatMessage(messages.code)"
                    :disabled="busy"
                    @keydown.enter.exact.prevent="submitJoin"
                />
                <p class="rooms-hint">{{ formatMessage(messages.codeHint) }}</p>
                <Button type="colored" color="brand" :disabled="busy" @click="submitJoin">
                    {{ formatMessage(busy ? messages.connecting : messages.join) }}
                </Button>
            </div>
        </NewModal>
    </div>
</template>

<style scoped>
/* ------------------------------------------------------------------ *
 * 一块玻璃外壳，三区之间只画发丝线 —— 不再每个区域套一层卡片
 * （主人立的规矩：不要大卡片套卡片）。
 * ------------------------------------------------------------------ */
.rooms {
    display: flex;
    height: 100%;
    min-height: 0;
    padding: 1rem 1.15rem;
    box-sizing: border-box;
}

.rooms-shell {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    /* 圆角与描边由 `.card` 提供，这里只保证裁切 */
    border-radius: var(--radius-lg);
}

/* ---------------- 共用 ---------------- */

.rooms-bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 0.75rem;
    min-height: 3rem;
    box-sizing: border-box;
    flex-shrink: 0;
}

.rooms-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: var(--color-text-primary);
    flex: 1;
    min-width: 0;
}

.rooms-empty {
    margin: 0;
    padding: 1.5rem 0.75rem;
    text-align: center;
    font-size: 0.78rem;
    line-height: 1.6;
    /* 玻璃底上 tertiary 在花壁纸上读不出来，文字提到 secondary —— 不压暗任何背景 */
    color: var(--color-text-secondary);
}

.rooms-hint {
    margin: 0;
    font-size: 0.72rem;
    line-height: 1.5;
    color: var(--color-text-tertiary);
}

.rooms-divider {
    height: 1px;
    margin: 0.35rem 0;
    background: var(--stroke-1);
}

.rooms-avatar {
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    flex-shrink: 0;
    border-radius: var(--radius-max);
    /* 头像只用玻璃档 + 文字首字母：不引入新色板，浅深两档都读得清 */
    background: var(--glass-3);
    border: 1px solid var(--stroke-1);
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    user-select: none;
}

/* ---------------- 左区 ---------------- */

.rooms-list {
    width: 264px;
    flex: 0 0 264px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--stroke-1);
}

.rooms-list-name {
    padding: 0 0.75rem 0.5rem;
}

.rooms-account {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 0.75rem 0.6rem;
}

.rooms-account-text {
    display: flex;
    flex-direction: column;
    gap: 0.05rem;
    min-width: 0;
}

.rooms-account-text strong {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--color-text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.rooms-account-text span {
    font-size: 0.66rem;
    color: var(--color-text-tertiary);
}

.rooms-list-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 0.4rem;
}

.rooms-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.45rem 0.5rem;
    margin-bottom: 0.15rem;
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: transparent;
    text-align: left;
    cursor: pointer;
    color: var(--color-text-primary);
    transition: background-color 140ms ease;
}

.rooms-item:hover {
    background: var(--glass-2);
}

.rooms-item.on {
    background: var(--glass-3);
    border-color: var(--stroke-1);
}

.rooms-item-text {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    flex: 1;
    min-width: 0;
}

.rooms-item-name {
    font-size: 0.82rem;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.rooms-item-meta {
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.rooms-item-time {
    font-size: 0.66rem;
    color: var(--color-text-tertiary);
    flex-shrink: 0;
}

.rooms-item-del {
    display: none;
    place-items: center;
    width: 1.15rem;
    height: 1.15rem;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    color: var(--color-text-tertiary);
}

.rooms-item:hover .rooms-item-del {
    display: grid;
}

.rooms-item-del:hover {
    color: var(--color-text-primary);
    background: var(--glass-3);
}

.rooms-item-del :deep(svg) {
    width: 0.8rem;
    height: 0.8rem;
}

.rooms-list-foot {
    padding: 0.5rem 0.6rem 0.65rem;
    border-top: 1px solid var(--stroke-1);
}

.rooms-list-foot :deep(button) {
    width: 100%;
}

/* ---------------- 中区 ---------------- */

.rooms-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
}

.rooms-head-main {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1;
    min-width: 0;
}

.rooms-head-name {
    font-size: 0.92rem;
    font-weight: 600;
    color: var(--color-text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.rooms-code {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.15rem 0.4rem;
    border: 1px solid var(--stroke-1);
    border-radius: var(--radius-sm);
    background: var(--glass-2);
    color: var(--color-text-secondary);
    font-size: 0.7rem;
    cursor: pointer;
}

.rooms-code :deep(svg) {
    width: 0.75rem;
    height: 0.75rem;
}

.rooms-head-actions {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-shrink: 0;
}

.rooms-pill {
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-max);
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    font-size: 0.68rem;
    font-variant-numeric: tabular-nums;
    color: var(--color-text-secondary);
}

.rooms-state {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.rooms-state::before {
    content: '';
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--color-text-tertiary);
}

.rooms-state.on {
    color: var(--color-text-secondary);
}

.rooms-state.on::before {
    background: var(--color-brand);
}

.rooms-notice {
    margin: 0;
    padding: 0.4rem 0.85rem;
    font-size: 0.74rem;
    color: var(--color-text-secondary);
    background: var(--glass-2);
    border-top: 1px solid var(--stroke-1);
    border-bottom: 1px solid var(--stroke-1);
}

/* ---------------- 消息流 ---------------- */

.rooms-thread {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.75rem 0.9rem;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
}

.rooms-day {
    align-self: center;
    margin: 0.35rem 0;
    padding: 0.1rem 0.55rem;
    border-radius: var(--radius-max);
    background: var(--glass-2);
    font-size: 0.68rem;
    color: var(--color-text-tertiary);
}

.rooms-system {
    align-self: center;
    margin: 0;
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
    text-align: center;
}

.rooms-row {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    max-width: 76%;
}

.rooms-row.mine {
    align-self: flex-end;
    flex-direction: row-reverse;
}

.rooms-row.grouped {
    margin-top: -0.35rem;
}

/* 合并组里补一个头像宽度的空白，气泡才能对齐 */
.rooms-row.grouped .rooms-avatar {
    visibility: hidden;
}

.rooms-msg {
    display: flex;
    flex-direction: column;
    gap: 0.18rem;
    min-width: 0;
}

.rooms-row.mine .rooms-msg {
    align-items: flex-end;
}

.rooms-msg-from {
    display: inline-flex;
    align-items: baseline;
    gap: 0.35rem;
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.rooms-msg-from time {
    font-size: 0.64rem;
}

.rooms-bubble {
    padding: 0.45rem 0.7rem;
    border-radius: var(--radius-md);
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--color-text-primary);
    font-size: 0.82rem;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
}

/* 自己的消息：主色淡底，一眼能分开，但不是实心深色块 */
.rooms-row.mine .rooms-bubble {
    background: color-mix(in srgb, var(--color-brand) 16%, var(--glass-2));
    border-color: color-mix(in srgb, var(--color-brand) 26%, var(--stroke-1));
}

.rooms-file {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.45rem 0.65rem;
    border-radius: var(--radius-md);
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--color-text-primary);
    font-size: 0.78rem;
    text-decoration: none;
    max-width: 100%;
}

.rooms-file span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.rooms-file :deep(svg) {
    width: 0.9rem;
    height: 0.9rem;
    flex-shrink: 0;
}

.rooms-pack {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.5rem 0.7rem;
    border-radius: var(--radius-md);
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    font-size: 0.76rem;
    color: var(--color-text-secondary);
}

.rooms-pack strong {
    color: var(--color-text-primary);
    font-size: 0.8rem;
}

.rooms-pack summary {
    cursor: pointer;
    font-size: 0.7rem;
}

.rooms-pack ul {
    margin: 0.2rem 0 0;
    padding-left: 1rem;
    font-size: 0.7rem;
}

/* ---------------- 输入区 ---------------- */

.rooms-compose {
    display: flex;
    align-items: flex-end;
    gap: 0.45rem;
    padding: 0.6rem 0.75rem;
    border-top: 1px solid var(--stroke-1);
    flex-shrink: 0;
}

.rooms-input {
    flex: 1;
    min-width: 0;
    max-height: 7rem;
    padding: 0.5rem 0.65rem;
    border-radius: var(--radius-md);
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--color-text-primary);
    font: inherit;
    font-size: 0.82rem;
    line-height: 1.5;
    resize: none;
    outline: none;
}

.rooms-input:focus {
    border-color: var(--color-brand);
}

.rooms-input::placeholder {
    color: var(--color-text-tertiary);
}

/* ---------------- 右区 ---------------- */

.rooms-side {
    width: 280px;
    flex: 0 0 280px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--stroke-1);
    transition:
        flex-basis 180ms ease,
        width 180ms ease;
}

.rooms-side.closed {
    width: 0;
    flex-basis: 0;
    border-left: none;
    overflow: hidden;
}

.rooms-side-head {
    padding: 0.55rem 0.6rem;
    flex-shrink: 0;
}

.rooms-side-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    padding: 0 0.75rem 0.9rem;
}

.rooms-side-count {
    margin: 0 0 0.15rem;
    font-size: 0.68rem;
    color: var(--color-text-tertiary);
}

.rooms-member {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

.rooms-member-text {
    display: flex;
    flex-direction: column;
    gap: 0.05rem;
    min-width: 0;
}

.rooms-member-text strong {
    font-size: 0.8rem;
    font-weight: 500;
    color: var(--color-text-primary);
}

.rooms-member-text span {
    font-size: 0.68rem;
    color: var(--color-text-tertiary);
}

.rooms-member-ip {
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
}

.rooms-switch {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    font-size: 0.78rem;
    color: var(--color-text-primary);
}

.rooms-field {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.rooms-label {
    font-size: 0.72rem;
    color: var(--color-text-secondary);
}

/* ---------------- 弹窗 ---------------- */

.rooms-modal {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
}
</style>
