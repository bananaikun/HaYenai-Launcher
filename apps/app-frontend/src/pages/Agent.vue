<script setup lang="ts">
import {
    ArrowUpIcon,
    BotIcon,
    CheckIcon,
    CpuIcon,
    FileImageIcon,
    HistoryIcon,
    PlusIcon,
    SettingsIcon,
    ShieldIcon,
    TrashIcon,
    XIcon,
} from '@modrinth/assets'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { readFile } from '@tauri-apps/plugin-fs'
import {
    Button,
    Card,
    Combobox,
    type ComboboxOption,
    defineMessages,
    IconButton,
    injectNotificationManager,
    NewModal,
    StatusLabel,
    Tabs,
    type TabsValue,
    Toggle,
    useVIntl,
} from '@modrinth/ui'
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'

import { agentErrorMessage, type AgentMessage, agentRunTool, agentTurn } from '@/helpers/agent'
import {
    type AIPreset,
    type AIPresetState,
    type AIProviderConfig,
    getAIState,
    listAIPresets,
    saveAIPreset,
} from '@/helpers/ai'
import { useComposerOffset } from '@/helpers/composer-offset'
import {
    buildModelOptions,
    decodeModel,
    encodeModel,
    readGlobalModel,
    writeGlobalModel,
    type ModelRef,
} from '@/helpers/ai-model'
import { list as listGameInstances } from '@/helpers/instance'

const { formatMessage } = useVIntl()
const { addNotification } = injectNotificationManager()

const messages = defineMessages({
    title: { id: 'app.agent.title', defaultMessage: 'Agent' },
    subtitle: {
        id: 'app.agent.subtitle',
        defaultMessage: 'Reads your instances and logs, and changes them only when you allow it.',
    },
    settings: { id: 'app.agent.settings', defaultMessage: 'Settings' },
    newChat: { id: 'app.agent.new-chat', defaultMessage: 'New chat' },
    history: { id: 'app.agent.history', defaultMessage: 'History' },
    noPreset: { id: 'app.agent.no-preset', defaultMessage: 'No preset' },
    preset: { id: 'app.agent.preset', defaultMessage: 'Preset' },
    noPresetHint: {
        id: 'app.agent.no-preset-hint',
        defaultMessage: 'Create a persona preset under AI settings first.',
    },
    placeholder: {
        id: 'app.agent.placeholder',
        defaultMessage: 'Ask about an instance, a crash, or a setting…',
    },
    send: { id: 'app.agent.send', defaultMessage: 'Send' },
    empty: {
        id: 'app.agent.empty',
        defaultMessage: 'Ask something. It will read real files instead of guessing.',
    },
    thinking: { id: 'app.agent.thinking', defaultMessage: 'Working…' },
    stop: { id: 'app.agent.stop', defaultMessage: 'Stop' },
    anyInstance: { id: 'app.agent.any-instance', defaultMessage: 'Any instance' },
    model: { id: 'app.agent.model', defaultMessage: 'Model settings' },
    work: { id: 'app.agent.work', defaultMessage: 'Work' },
    tasks: { id: 'app.agent.tasks', defaultMessage: 'Tasks' },
    plan: { id: 'app.agent.plan', defaultMessage: 'Plan' },
    todos: { id: 'app.agent.todos', defaultMessage: 'My to-dos' },
    noPlan: {
        id: 'app.agent.no-plan',
        defaultMessage:
            'No plan yet. For a task that needs several steps, the AI lists them here and ticks them off.',
    },
    done: { id: 'app.agent.done', defaultMessage: 'Done' },
    noHistory: { id: 'app.agent.no-history', defaultMessage: 'No previous chats yet.' },
    permission: { id: 'app.agent.permission', defaultMessage: 'Tool permission' },
    readOnlyNote: {
        id: 'app.agent.read-only-note',
        defaultMessage: 'Reads files and logs only; write tools are hidden.',
    },
    fullNote: {
        id: 'app.agent.full-note',
        defaultMessage: 'Can rename instances and change memory when you ask.',
    },
    pickModel: { id: 'app.agent.pick-model', defaultMessage: 'Provider & model' },
    manageModel: {
        id: 'app.agent.manage-model',
        defaultMessage: 'Providers and API keys live in Settings → AI.',
    },
    delete: { id: 'app.agent.delete', defaultMessage: 'Delete' },
    attachImage: { id: 'app.agent.attach-image', defaultMessage: 'Attach image' },
    phaseReplying: { id: 'app.agent.phase-replying', defaultMessage: 'Replying…' },
    phaseThinking: { id: 'app.agent.phase-thinking', defaultMessage: 'Thinking…' },
    phaseTool: { id: 'app.agent.phase-tool', defaultMessage: 'Calling {name}' },
    copy: { id: 'app.agent.copy', defaultMessage: 'Copy' },
    regenerate: { id: 'app.agent.regenerate', defaultMessage: 'Regenerate' },
    delete: { id: 'app.agent.delete', defaultMessage: 'Delete' },
    askTitle: { id: 'app.agent.ask-title', defaultMessage: 'Quick replies' },
    askMore: { id: 'app.agent.ask-more', defaultMessage: 'Tell me more' },
    askExample: { id: 'app.agent.ask-example', defaultMessage: 'Give an example' },
    askFix: { id: 'app.agent.ask-fix', defaultMessage: 'Just fix it' },
    phaseDone: {
        id: 'app.agent.phase-done',
        defaultMessage: 'Done in {seconds}s · {tools} tool call(s)',
    },
    removeImage: { id: 'app.agent.remove-image', defaultMessage: 'Remove image' },
    imageCount: {
        id: 'app.agent.image-count',
        defaultMessage: '{count} image(s)',
    },
    imageTooLarge: {
        id: 'app.agent.image-too-large',
        defaultMessage: 'That image could not be read, or it is too large.',
    },
})

interface ToolCard {
    id: string
    name: string
    args: unknown
    result?: unknown
    error?: string
    running: boolean
    /** 开始执行的时间戳，用来算耗时 —— 「对话过程要详细」的一部分。 */
    startedAt?: number
    /** 耗时毫秒。跑完才填。 */
    elapsedMs?: number
}

interface Bubble {
    role: 'user' | 'assistant'
    text: string
    tools: ToolCard[]
    /** 当前阶段文案（思考中/调用工具/生成中），只在生成过程中有值。 */
    phase?: string
    /** 这一轮真正开始的时间，用来显示"已用 N 秒"。 */
    startedAt?: number
    /** 结束时间。有值 = 这一轮结束了。 */
    finishedAt?: number
    /** 用户消息里带的图（data URL，只用于显示）。 */
    images?: string[]
    /** 存 localStorage 时图片会被丢掉，只留张数，历史里提示「N 张图」。 */
    imageCount?: number
}

interface Session {
    id: string
    title: string
    updatedAt: number
    bubbles: Bubble[]
    history: AgentMessage[]
}

interface GameInstanceLite {
    id: string
    name: string
}

const STORE_KEY = 'hayenai.agent.sessions'

const bubbles = ref<Bubble[]>([])
const draft = ref('')
const busy = ref(false)
const cancelled = ref(false)
const threadEl = ref<HTMLElement | null>(null)
const composerEl = ref<HTMLElement | null>(null)
useComposerOffset(composerEl)

/** 真正发给模型的整段对话。UI 只渲染 bubbles，两者形状不同。 */
const history = ref<AgentMessage[]>([])

const presetState = ref<AIPresetState>({ presets: [], activeId: null })
const presetId = ref<string | null>(null)
const providers = ref<AIProviderConfig[]>([])
const instances = ref<GameInstanceLite[]>([])
const instanceId = ref<string | null>(null)

interface PendingImage {
    id: string
    /** 发给模型用的 base64 与 mime */
    attachment: AgentAttachment
    /** 仅本地预览用的 data URL */
    preview: string
}

/** 单张图缩放后的上限：1280 长边、JPEG 0.85。再大对视觉问答没有收益，只会拖慢上传。 */
const IMAGE_MAX_EDGE = 1280
const IMAGE_QUALITY = 0.85
/** 一次最多带几张，防止一条消息把上下文撑爆。 */
const MAX_IMAGES = 4

const pendingImages = ref<PendingImage[]>([])
const picking = ref(false)

const accessFull = ref(false)
const settingsModal = ref<InstanceType<typeof NewModal> | null>(null)
const historyModal = ref<InstanceType<typeof NewModal> | null>(null)
const taskTab = ref<TabsValue>('plan')

const sessions = ref<Session[]>([])
const sessionId = ref<string>(newId())

function newId(): string {
    return Math.random().toString(36).slice(2) + Date.now().toString(36)
}

const activePreset = computed<AIPreset | null>(
    () => presetState.value.presets.find((preset) => preset.id === presetId.value) ?? null,
)
/**
 * 用哪个模型是**全局**的（见 helpers/ai-model.ts）。
 *
 * 人格方案只管「说话的性格/人设/规则」，不再兼任模型配置 ——
 * 主人 2026-11-14 明确定义。所以这里读写 localStorage，不碰 preset。
 */
const globalModel = ref<ModelRef | null>(readGlobalModel())
const modelBound = computed(() => !!globalModel.value)
const canSend = computed(
    () =>
        !busy.value &&
        (draft.value.trim().length > 0 || pendingImages.value.length > 0) &&
        modelBound.value,
)

const instanceOptions = computed<ComboboxOption<string | null>[]>(() => [
    { value: null, label: formatMessage(messages.anyInstance) },
    ...instances.value.map((instance) => ({ value: instance.id, label: instance.name })),
])

const presetOptions = computed<ComboboxOption<string | null>[]>(() =>
    presetState.value.presets.length
        ? presetState.value.presets.map((preset) => ({ value: preset.id, label: preset.name }))
        : [{ value: null, label: formatMessage(messages.noPreset) }],
)

/** 一张表：所有已添加的模型，按厂商 A–Z、同厂商内模型名 A–Z。 */
const modelOptions = computed<ComboboxOption<string>[]>(() =>
    buildModelOptions(
        providers.value.map((provider) => ({
            providerId: provider.provider_id,
            label: provider.custom_name ?? provider.provider_id,
            models: provider.models,
        })),
    ),
)

const modelSelection = computed<string>({
    get: () => encodeModel(globalModel.value),
    set: (value: string) => {
        globalModel.value = decodeModel(value)
        writeGlobalModel(globalModel.value)
    },
})

const taskTabs = computed(() => [
    { value: 'plan', label: formatMessage(messages.plan) },
    { value: 'todos', label: formatMessage(messages.todos) },
])

const modelLabel = computed(
    () =>
        modelOptions.value.find((option) => option.value === modelSelection.value)?.label ??
        formatMessage(messages.pickModel),
)

/** 右栏「工作」= 本次会话里真实跑过的工具步骤，不编造任务。 */
const workSteps = computed(() =>
    bubbles.value.flatMap((bubble) =>
        bubble.tools.map((tool) => ({
            id: tool.id,
            label: tool.name,
            running: tool.running,
            failed: !!tool.error,
        })),
    ),
)

const currentSessionTitle = computed(() => {
    const firstUser = bubbles.value.find((bubble) => bubble.role === 'user')?.text ?? ''
    return firstUser.trim().slice(0, 40) || formatMessage(messages.newChat)
})

/** 复制这一条（只复制正文，不带工具卡片）。 */
async function copyBubble(bubble: Bubble): Promise<void> {
    try {
        await navigator.clipboard.writeText(bubble.text)
    } catch {
        // 剪贴板权限被拒：静默失败比弹一个错误框好。
    }
}

/** 删掉这一条，并把它的前后文从历史里一起去掉（否则下一轮模型会看到"凭空消失"）。 */
function deleteBubble(index: number): void {
    const bubble = bubbles.value[index]
    if (!bubble) return
    bubbles.value.splice(index, 1)
    // 历史里对应的那条也要删：user 与 assistant 是成对 push 的，按内容匹配最近一条。
    const historyIndex = history.value.findLastIndex(
        (item) => item.role === bubble.role && item.content === bubble.text,
    )
    if (historyIndex >= 0) history.value.splice(historyIndex, 1)
    snapshotCurrent()
}

/** 重新生成：删掉这条回复，用上一条 user 消息重跑。 */
async function regenerate(index: number): Promise<void> {
    if (busy.value) return
    let userIndex = index - 1
    while (userIndex >= 0 && bubbles.value[userIndex]?.role !== 'user') userIndex -= 1
    const userBubble = bubbles.value[userIndex]
    if (!userBubble) return
    deleteBubble(index)
    const historyIndex = history.value.findLastIndex(
        (item) => item.role === 'user' && item.content === userBubble.text,
    )
    if (historyIndex >= 0) history.value.splice(historyIndex, 1)
    draft.value = userBubble.text
    await send()
}

async function scrollToBottom(): Promise<void> {
    await nextTick()
    const el = threadEl.value
    if (el) el.scrollTop = el.scrollHeight
}

async function loadPresets() {
    try {
        presetState.value = await listAIPresets()
        presetId.value = presetState.value.activeId ?? presetState.value.presets[0]?.id ?? null
    } catch {
        // 读不到方案不是致命错误：面板照样能打开，只是发不出消息。
    }
}

async function loadProviders() {
    try {
        const state = await getAIState()
        providers.value = state.providers.filter((provider) => provider.enabled)
    } catch {
        providers.value = []
    }
}

async function loadInstances() {
    try {
        instances.value = await listGameInstances()
    } catch {
        instances.value = []
    }
}

/* ---------------------------------------------------------------- *
 * 会话：存 localStorage，重启后「历史对话」还在
 * ---------------------------------------------------------------- */

function readSessions(): Session[] {
    try {
        const raw = localStorage.getItem(STORE_KEY)
        if (!raw) return []
        const parsed = JSON.parse(raw) as Session[]
        return Array.isArray(parsed) ? parsed : []
    } catch {
        return []
    }
}

function persistSessions() {
    try {
        localStorage.setItem(STORE_KEY, JSON.stringify(sessions.value.slice(0, 60)))
    } catch {
        // 存不下不算错误（localStorage 满），不影响当前对话继续。
    }
}

function snapshotCurrent() {
    if (!bubbles.value.length) return
    const record: Session = {
        id: sessionId.value,
        title: currentSessionTitle.value,
        updatedAt: Date.now(),
        // 图片是 base64，一条就好几百 KB；历史里只留「N 张图」的痕迹。
        bubbles: JSON.parse(JSON.stringify(bubbles.value)).map((bubble: Bubble) => ({
            ...bubble,
            images: undefined,
        })),
        history: JSON.parse(JSON.stringify(history.value)),
    }
    const index = sessions.value.findIndex((item) => item.id === record.id)
    if (index >= 0) sessions.value[index] = record
    else sessions.value.unshift(record)
    sessions.value.sort((a, b) => b.updatedAt - a.updatedAt)
    persistSessions()
}

function startNewChat() {
    snapshotCurrent()
    sessionId.value = newId()
    bubbles.value = []
    history.value = []
    void scrollToBottom()
}

function openSession(session: Session) {
    snapshotCurrent()
    sessionId.value = session.id
    bubbles.value = JSON.parse(JSON.stringify(session.bubbles))
    history.value = JSON.parse(JSON.stringify(session.history))
    historyModal.value?.hide()
    void scrollToBottom()
}

function removeSession(session: Session) {
    sessions.value = sessions.value.filter((item) => item.id !== session.id)
    persistSessions()
    if (session.id === sessionId.value) startNewChat()
}

/* ---------------------------------------------------------------- *
 * 方案 / 模型 / 权限
 * ---------------------------------------------------------------- */

/* ---------------------------------------------------------------- *
 * 一轮对话
 * ---------------------------------------------------------------- */

/**
 * 选图 → 读字节 → 用 canvas 缩到 IMAGE_MAX_EDGE → 存 base64。
 *
 * 为什么在前端缩：原图 base64 之后动辄 5 MB+，一路塞进请求体会把上游打爆，
 * 也会让「点发送到出字」多等好几秒。降采样放在这边只花几十毫秒。
 */
async function shrinkToAttachment(path: string): Promise<PendingImage | null> {
    const bytes = await readFile(path)
    const blob = new Blob([bytes])
    const bitmap = await createImageBitmap(blob)
    const scale = Math.min(1, IMAGE_MAX_EDGE / Math.max(bitmap.width, bitmap.height))
    const width = Math.max(1, Math.round(bitmap.width * scale))
    const height = Math.max(1, Math.round(bitmap.height * scale))

    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const context = canvas.getContext('2d')
    if (!context) return null
    // JPEG 没有透明通道，透明图先铺白，否则 PNG 转过来会变黑底。
    context.fillStyle = '#ffffff'
    context.fillRect(0, 0, width, height)
    context.drawImage(bitmap, 0, 0, width, height)
    bitmap.close()

    const dataUrl = canvas.toDataURL('image/jpeg', IMAGE_QUALITY)
    const comma = dataUrl.indexOf(',')
    if (comma < 0) return null
    return {
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        attachment: { mime: 'image/jpeg', data: dataUrl.slice(comma + 1) },
        preview: dataUrl,
    }
}

async function pickImages(): Promise<void> {
    if (picking.value || pendingImages.value.length >= MAX_IMAGES) return
    picking.value = true
    try {
        const picked = await openDialog({
            multiple: true,
            filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp'] }],
        })
        const paths = Array.isArray(picked) ? picked : picked ? [picked] : []
        for (const path of paths) {
            if (pendingImages.value.length >= MAX_IMAGES) break
            try {
                const image = await shrinkToAttachment(path)
                if (image) pendingImages.value.push(image)
            } catch (error) {
                console.warn('[agent] 读图失败', error)
                addNotification({
                    title: formatMessage(messages.attachImage),
                    text: formatMessage(messages.imageTooLarge),
                    type: 'warning',
                })
            }
        }
    } finally {
        picking.value = false
    }
}

function removeImage(index: number): void {
    pendingImages.value.splice(index, 1)
}

async function send() {
    const text = draft.value.trim()
    if (!text || busy.value || !modelBound.value) return

    draft.value = ''
    busy.value = true
    cancelled.value = false

    const attached = pendingImages.value.map((image) => image.attachment)
    const previews = pendingImages.value.map((image) => image.preview)
    pendingImages.value = []

    bubbles.value.push({
        role: 'user',
        text,
        tools: [],
        images: previews.length ? previews : undefined,
        imageCount: previews.length || undefined,
    })
    // 选了实例就把上下文塞进这条消息里，模型才知道该先看哪个实例。
    const instance = instances.value.find((item) => item.id === instanceId.value)
    history.value.push({
        role: 'user',
        content: instance
            ? `（当前实例：${instance.name} / id=${instance.id}）
${text}`
            : text,
        ...(attached.length ? { attachments: attached } : {}),
    })

    /**
     * ★ 必须是 `reactive`。
     *
     * 之前是普通对象：`reply.text += delta` 改的是原始对象，Vue 的依赖追踪看不到，
     * 所以流式增量根本不会上屏 —— 只有最后 `reply.text = message.content` 时
     * 靠别处的 ref 变化顺带重渲染一次，看起来就像「一次性吐出全文」。
     */
    const reply = reactive<Bubble>({
        role: 'assistant',
        text: '',
        tools: [],
        phase: formatMessage(messages.phaseThinking),
        startedAt: Date.now(),
    })
    bubbles.value.push(reply)
    await scrollToBottom()

    try {
        for (let round = 0; round < 8; round += 1) {
            if (cancelled.value) break
            const message = await agentTurn(
                {
                    presetId: presetId.value,
                    // 模型来自全局设置（人格方案不再带模型）。
                    providerId: globalModel.value?.providerId ?? null,
                    modelId: globalModel.value?.modelId ?? null,
                    allowWrite: accessFull.value,
                    messages: history.value,
                },
                (event) => {
                    if (event.kind === 'delta') {
                        reply.phase = formatMessage(messages.phaseReplying)
                        reply.text += event.text
                        void scrollToBottom()
                    } else if (event.kind === 'thinking') {
                        // 推理模型会先发 thinking：告诉用户「它在想」，别让人以为卡住。
                        reply.phase = formatMessage(messages.phaseThinking)
                        void scrollToBottom()
                    } else if (event.kind === 'tool') {
                        reply.phase = formatMessage(messages.phaseTool, { name: event.text })
                    }
                },
            )

            history.value.push(message)
            // 只有流式没吐出任何内容时才用最终文本兜底 —— 否则会把已经流式上屏的
            // 内容再覆盖一遍，偶尔还会因为两边分块差异丢字。
            if (!reply.text.trim() && message.content?.trim()) reply.text = message.content

            const calls = message.tool_calls ?? []
            if (calls.length === 0) break

            for (const call of calls) {
                if (cancelled.value) break
                const card: ToolCard = {
                    id: call.id,
                    name: call.name,
                    args: call.args,
                    running: true,
                    startedAt: Date.now(),
                }
                reply.tools.push(card)
                await scrollToBottom()
                try {
                    const result = await agentRunTool(call.name, call.args, accessFull.value)
                    card.result = result
                    card.running = false
                    if (card.startedAt) card.elapsedMs = Date.now() - card.startedAt
                    history.value.push({
                        role: 'tool',
                        tool_call_id: call.id,
                        content: JSON.stringify(result),
                    })
                } catch (error) {
                    card.error = agentErrorMessage(error)
                    card.running = false
                    if (card.startedAt) card.elapsedMs = Date.now() - card.startedAt
                    // 工具失败也要回灌，否则下一轮模型会以为这一步没发生过。
                    history.value.push({
                        role: 'tool',
                        tool_call_id: call.id,
                        content: JSON.stringify({ error: card.error }),
                    })
                }
                await scrollToBottom()
            }
        }
    } catch (error) {
        reply.text = `${reply.text}\n\n${agentErrorMessage(error)}`.trim()
    } finally {
        // 收尾时把阶段清掉、记结束时间 —— 气泡上就变成「用时 N 秒 · M 次工具调用」。
        reply.phase = undefined
        reply.finishedAt = Date.now()
        busy.value = false
        snapshotCurrent()
        await scrollToBottom()
    }
}

function stop() {
    cancelled.value = true
    busy.value = false
}

function onKeydown(event: KeyboardEvent) {
    // Enter 发送、Shift+Enter 换行 —— 和聊天软件一致，不用解释。
    if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault()
        void send()
    }
}

function prettyArgs(args: unknown): string {
    try {
        const text = JSON.stringify(args)
        return text === '{}' ? '' : text
    } catch {
        return String(args)
    }
}

function prettyResult(value: unknown): string {
    try {
        return JSON.stringify(value, null, 2)
    } catch {
        return String(value)
    }
}

/** 这一轮用了多少秒（一位小数）。没开始时间就返回 0。 */
function elapsedSeconds(bubble: Bubble): string {
    if (!bubble.startedAt) return '0.0'
    const end = bubble.finishedAt ?? Date.now()
    return ((end - bubble.startedAt) / 1000).toFixed(1)
}

function toolMillis(card: ToolCard): string {
    if (!card.elapsedMs) return ''
    return card.elapsedMs < 1000
        ? `${card.elapsedMs} ms`
        : `${(card.elapsedMs / 1000).toFixed(1)} s`
}

function timeLabel(value: number): string {
    const date = new Date(value)
    return `${date.getMonth() + 1}/${date.getDate()} ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}

onMounted(async () => {
    sessions.value = readSessions()
    await Promise.all([loadPresets(), loadProviders(), loadInstances()])
})
</script>

<template>
    <div class="agent-page">
        <header class="agent-head">
            <BotIcon class="agent-head-icon" />
            <div class="agent-head-text">
                <h1>{{ formatMessage(messages.title) }}</h1>
                <p>{{ formatMessage(messages.subtitle) }}</p>
            </div>
            <div class="agent-head-actions">
                <IconButton
                    :label="formatMessage(messages.settings)"
                    @click="settingsModal?.show()"
                >
                    <SettingsIcon />
                </IconButton>
                <IconButton :label="formatMessage(messages.newChat)" @click="startNewChat">
                    <PlusIcon />
                </IconButton>
                <IconButton :label="formatMessage(messages.history)" @click="historyModal?.show()">
                    <HistoryIcon />
                </IconButton>
            </div>
        </header>

        <div class="agent-body">
            <section class="agent-chat">
                <div ref="threadEl" class="agent-thread">
                    <p v-if="!bubbles.length" class="agent-empty">
                        {{ formatMessage(messages.empty) }}
                    </p>

                    <div
                        v-for="(bubble, index) in bubbles"
                        :key="index"
                        class="agent-bubble"
                        :class="bubble.role"
                    >
                        <div v-if="bubble.images?.length" class="agent-bubble-images">
                            <img
                                v-for="(image, imageIndex) in bubble.images"
                                :key="imageIndex"
                                :src="image"
                                alt=""
                            />
                        </div>
                        <div v-if="bubble.imageCount && !bubble.images?.length" class="agent-hint">
                            {{ formatMessage(messages.imageCount, { count: bubble.imageCount }) }}
                        </div>
                        <div
                            v-if="
                                bubble.role === 'assistant' && (bubble.phase || bubble.finishedAt)
                            "
                            class="agent-phase"
                        >
                            <span
                                class="agent-phase-dot"
                                :class="{ live: !bubble.finishedAt && bubble.phase }"
                            />
                            <span v-if="bubble.phase">{{ bubble.phase }}</span>
                            <span v-else>
                                {{
                                    formatMessage(messages.phaseDone, {
                                        seconds: elapsedSeconds(bubble),
                                        tools: bubble.tools.length,
                                    })
                                }}
                            </span>
                        </div>
                        <div v-if="bubble.text" class="agent-text card">{{ bubble.text }}</div>

                        <div
                            v-if="bubble.role === 'assistant' && !bubble.phase && bubble.text"
                            class="agent-msg-actions"
                        >
                            <button
                                type="button"
                                v-tooltip="formatMessage(messages.copy)"
                                @click="copyBubble(bubble)"
                            >
                                <svg viewBox="0 0 24 24" aria-hidden="true">
                                    <rect x="9" y="9" width="11" height="11" rx="2" />
                                    <path d="M5 15V5a2 2 0 0 1 2-2h10" />
                                </svg>
                            </button>
                            <button
                                type="button"
                                v-tooltip="formatMessage(messages.regenerate)"
                                :disabled="busy"
                                @click="regenerate(index)"
                            >
                                <svg viewBox="0 0 24 24" aria-hidden="true">
                                    <path d="M3 12a9 9 0 1 0 3-6.7" />
                                    <path d="M3 4v5h5" />
                                </svg>
                            </button>
                            <button
                                type="button"
                                v-tooltip="formatMessage(messages.delete)"
                                :disabled="busy"
                                @click="deleteBubble(index)"
                            >
                                <svg viewBox="0 0 24 24" aria-hidden="true">
                                    <path d="M4 7h16" />
                                    <path d="M9 7V4h6v3" />
                                    <path d="M6 7l1 13h10l1-13" />
                                </svg>
                            </button>
                        </div>

                        <Card v-for="card in bubble.tools" :key="card.id" class="agent-tool">
                            <div class="agent-tool-head">
                                <StatusLabel :label="card.name" class="agent-tool-name" />
                                <span v-if="card.running" class="agent-tool-spin" />
                                <span v-else class="agent-tool-time">{{ toolMillis(card) }}</span>
                                <span v-if="!card.running && card.error" class="agent-tool-bad">
                                    !
                                </span>
                                <CheckIcon v-else-if="!card.running" class="agent-tool-ok" />
                            </div>
                            <div v-if="prettyArgs(card.args)" class="agent-tool-args">
                                {{ prettyArgs(card.args) }}
                            </div>
                            <details v-if="!card.running && !card.error" class="agent-tool-out">
                                <summary>{{ card.name }}</summary>
                                <pre class="bg-surface-2">{{ prettyResult(card.result) }}</pre>
                            </details>
                            <div v-else-if="card.error" class="agent-tool-err">
                                {{ card.error }}
                            </div>
                        </Card>
                    </div>

                    <p
                        v-if="busy && !bubbles.some((bubble) => bubble.phase && !bubble.finishedAt)"
                        class="agent-working"
                    >
                        {{ formatMessage(messages.thinking) }}
                    </p>
                </div>

                <footer ref="composerEl" class="agent-compose">
                    <div v-if="pendingImages.length" class="agent-attach-row">
                        <div
                            v-for="(image, index) in pendingImages"
                            :key="image.id"
                            class="agent-attach"
                        >
                            <img :src="image.preview" alt="" />
                            <button
                                type="button"
                                class="agent-attach-x"
                                :aria-label="formatMessage(messages.removeImage)"
                                @click="removeImage(index)"
                            >
                                <XIcon />
                            </button>
                        </div>
                    </div>

                    <div class="agent-compose-box card">
                        <IconButton
                            :label="formatMessage(messages.attachImage)"
                            :disabled="picking"
                            @click="pickImages"
                        >
                            <FileImageIcon />
                        </IconButton>
                        <textarea
                            v-model="draft"
                            :placeholder="formatMessage(messages.placeholder)"
                            rows="1"
                            class="agent-input"
                            @keydown="onKeydown"
                        />
                        <Button v-if="busy" type="colored" color="red" size="sm" @click="stop">
                            {{ formatMessage(messages.stop) }}
                        </Button>
                        <IconButton
                            v-else
                            type="colored"
                            color="brand"
                            :label="formatMessage(messages.send)"
                            :disabled="!canSend"
                            @click="send"
                        >
                            <ArrowUpIcon />
                        </IconButton>
                    </div>
                </footer>
            </section>

            <aside class="agent-aside">
                <Combobox
                    v-model="instanceId"
                    :options="instanceOptions"
                    width="full"
                    :placeholder="formatMessage(messages.anyInstance)"
                />

                <Card>
                    <div class="agent-perm">
                        <div>
                            <StatusLabel
                                :icon="ShieldIcon"
                                :label="formatMessage(messages.permission)"
                            />
                            <p class="agent-hint">
                                {{
                                    formatMessage(
                                        accessFull ? messages.fullNote : messages.readOnlyNote,
                                    )
                                }}
                            </p>
                        </div>
                        <Toggle v-model="accessFull" />
                    </div>
                </Card>

                <Button class="agent-model-btn" @click="settingsModal?.show()">
                    <CpuIcon />
                    <span>{{ formatMessage(messages.model) }}</span>
                    <span class="agent-model-note">{{ modelLabel }}</span>
                </Button>

                <Card>
                    <template #header>
                        <h2 class="agent-card-title">{{ formatMessage(messages.work) }}</h2>
                    </template>
                    <ul v-if="workSteps.length" class="agent-steps">
                        <li v-for="step in workSteps" :key="step.id">
                            <CheckIcon v-if="!step.running && !step.failed" class="agent-step-ok" />
                            <span v-else-if="step.running" class="agent-tool-spin" />
                            <span v-else class="agent-tool-bad">!</span>
                            <span class="agent-step-label">{{ step.label }}</span>
                            <span class="agent-step-state">
                                {{
                                    step.running
                                        ? formatMessage(messages.thinking)
                                        : formatMessage(messages.done)
                                }}
                            </span>
                        </li>
                    </ul>
                    <p v-else class="agent-hint">{{ formatMessage(messages.noPlan) }}</p>
                </Card>

                <Card>
                    <template #header>
                        <h2 class="agent-card-title">{{ formatMessage(messages.tasks) }}</h2>
                    </template>
                    <Tabs v-model:value="taskTab" :tabs="taskTabs" />
                    <p class="agent-hint agent-hint-gap">{{ formatMessage(messages.noPlan) }}</p>
                </Card>
            </aside>
        </div>

        <NewModal
            ref="settingsModal"
            :header="formatMessage(messages.settings)"
            max-width="640px"
            scrollable
        >
            <div class="agent-form">
                <div class="agent-field">
                    <label>{{ formatMessage(messages.preset) }}</label>
                    <Combobox v-model="presetId" :options="presetOptions" width="full" />
                    <p class="agent-hint">{{ formatMessage(messages.manageModel) }}</p>
                </div>

                <div class="agent-field">
                    <label>{{ formatMessage(messages.pickModel) }}</label>
                    <Combobox
                        v-model="modelSelection"
                        :options="modelOptions"
                        width="full"
                        :placeholder="formatMessage(messages.pickModel)"
                    />
                </div>

                <div class="agent-field">
                    <label>{{ formatMessage(messages.permission) }}</label>
                    <div class="agent-perm">
                        <p class="agent-hint">
                            {{
                                formatMessage(
                                    accessFull ? messages.fullNote : messages.readOnlyNote,
                                )
                            }}
                        </p>
                        <Toggle v-model="accessFull" />
                    </div>
                </div>

                <!-- 人格/规则只在「设置 → AI 功能 → 人格与方案」里编辑，这里再放一份就是两套真源 -->
                <p v-if="!activePreset" class="agent-hint">
                    {{ formatMessage(messages.noPresetHint) }}
                </p>
            </div>
        </NewModal>

        <NewModal
            ref="historyModal"
            :header="formatMessage(messages.history)"
            max-width="560px"
            scrollable
        >
            <p v-if="!sessions.length" class="agent-hint">
                {{ formatMessage(messages.noHistory) }}
            </p>
            <ul v-else class="agent-history">
                <li v-for="session in sessions" :key="session.id">
                    <Button class="agent-history-item" @click="openSession(session)">
                        <span class="agent-history-title">{{ session.title }}</span>
                        <span class="agent-history-time">{{ timeLabel(session.updatedAt) }}</span>
                    </Button>
                    <IconButton
                        :label="formatMessage(messages.delete)"
                        @click="removeSession(session)"
                    >
                        <TrashIcon />
                    </IconButton>
                </li>
            </ul>
        </NewModal>
    </div>
</template>

<style scoped>
.agent-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    /* 只有对话区能滚：页面本身溢出就直接裁掉，别让外层跟着滚 */
    overflow: hidden;
    gap: 0.9rem;
    padding: 1.15rem 1.35rem 0;
    box-sizing: border-box;
}

.agent-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-shrink: 0;
}

.agent-head-icon {
    width: 1.35rem;
    height: 1.35rem;
    color: var(--c-a);
    flex-shrink: 0;
}

.agent-head-text {
    min-width: 0;
    margin-right: auto;
}

.agent-head-text h1 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 600;
    line-height: 1.3;
    color: var(--color-text-primary);
}

.agent-head-text p {
    margin: 0;
    font-size: 0.78rem;
    color: var(--color-text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.agent-head-actions {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-shrink: 0;
}

.agent-body {
    display: flex;
    gap: 0.9rem;
    flex: 1;
    min-height: 0;
}

.agent-chat {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
}

.agent-thread {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
    padding: 0.1rem 0.35rem 0.9rem 0.1rem;
}

.agent-empty {
    margin: auto;
    max-width: 22rem;
    text-align: center;
    font-size: 0.85rem;
    color: var(--color-text-tertiary);
}

.agent-bubble {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    max-width: 46rem;
}

.agent-bubble.user {
    align-self: flex-end;
    align-items: flex-end;
}

.agent-text {
    padding: 0.6rem 0.9rem;
    border-radius: var(--radius-md);
    font-size: 0.88rem;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--color-text-primary);
}

.agent-bubble.user .agent-text {
    background: color-mix(in srgb, var(--c-a) 16%, transparent);
    border-color: color-mix(in srgb, var(--c-a) 34%, transparent);
}

.agent-msg-actions {
    display: flex;
    align-items: center;
    gap: 0.15rem;
    padding-left: 0.1rem;
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
}

.agent-bubble:hover .agent-msg-actions,
.agent-msg-actions:focus-within {
    opacity: 1;
}

.agent-msg-actions button {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--color-text-tertiary);
    cursor: pointer;
    transition:
        background var(--t-fast) var(--ease),
        color var(--t-fast) var(--ease);
}

.agent-msg-actions button:hover:not(:disabled) {
    background: var(--glass-2);
    border-color: var(--stroke-1);
    color: var(--text-0);
}

.agent-msg-actions button:disabled {
    opacity: 0.4;
    cursor: default;
}

.agent-msg-actions svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
}

.agent-tool {
    padding: 0.5rem 0.7rem;
    font-size: 0.78rem;
}

.agent-tool-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
}

.agent-tool-name {
    font-family: var(--font-mono, monospace);
    color: var(--color-text-primary);
}

.agent-tool-ok {
    width: 0.9rem;
    height: 0.9rem;
    color: var(--c-ok);
}

.agent-tool-spin {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 50%;
    border: 1.5px solid var(--color-text-tertiary);
    border-top-color: transparent;
    animation: agent-spin 0.7s linear infinite;
    flex-shrink: 0;
}

@keyframes agent-spin {
    to {
        transform: rotate(360deg);
    }
}

.agent-tool-bad {
    color: var(--c-bad);
    font-weight: 700;
}

.agent-tool-args {
    margin-top: 0.2rem;
    font-family: var(--font-mono, monospace);
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
    word-break: break-all;
}

.agent-tool-out summary {
    margin-top: 0.25rem;
    cursor: pointer;
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}

.agent-tool-out pre {
    margin: 0.3rem 0 0;
    max-height: 14rem;
    overflow: auto;
    padding: 0.45rem;
    border-radius: var(--radius-xs);
    font-size: 0.7rem;
    color: var(--color-text-secondary);
}

.agent-tool-err {
    margin-top: 0.25rem;
    font-size: 0.74rem;
    color: var(--c-bad);
}

.agent-working {
    margin: 0;
    font-size: 0.78rem;
    color: var(--color-text-tertiary);
}

.agent-compose {
    flex-shrink: 0;
    padding: 0.35rem 0.1rem 0.9rem;
}

.agent-compose-box {
    display: flex;
    align-items: flex-end;
    gap: 0.4rem;
    padding: 0.35rem;
    border-radius: var(--radius-lg);
}

/* 附件预览条：贴在输入框上方 */
.agent-attach-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-bottom: 0.45rem;
}

.agent-attach {
    position: relative;
    width: 4rem;
    height: 4rem;
    border-radius: var(--radius-sm);
    overflow: hidden;
    border: 1px solid var(--stroke-1);
}

.agent-attach img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}

.agent-attach-x {
    position: absolute;
    top: 2px;
    right: 2px;
    display: grid;
    place-items: center;
    width: 1.1rem;
    height: 1.1rem;
    padding: 0;
    border: none;
    border-radius: var(--radius-max);
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    cursor: pointer;
}

.agent-attach-x svg {
    width: 0.7rem;
    height: 0.7rem;
}

/* 气泡里的图：最多一行，超出裁掉，别把对话顶开 */
.agent-bubble-images {
    display: flex;
    gap: 0.35rem;
    flex-wrap: wrap;
    max-width: 24rem;
}

.agent-bubble-images img {
    max-width: 11rem;
    max-height: 11rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--stroke-1);
    object-fit: cover;
}

/* 阶段行：生成中显示「正在生成…」，结束后变成「用时 N 秒 · M 次工具调用」 */
.agent-phase {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0 0.15rem;
    font-size: 0.7rem;
    color: var(--color-text-tertiary);
}

.agent-phase-dot {
    width: 0.32rem;
    height: 0.32rem;
    border-radius: 50%;
    background: var(--color-text-tertiary);
    flex-shrink: 0;
}

.agent-phase-dot.live {
    background: var(--color-brand);
    animation: agent-phase-pulse 1.1s ease-in-out infinite;
}

@keyframes agent-phase-pulse {
    0%,
    100% {
        opacity: 0.35;
        transform: scale(0.85);
    }
    50% {
        opacity: 1;
        transform: scale(1);
    }
}

.agent-tool-time {
    font-size: 0.66rem;
    font-variant-numeric: tabular-nums;
    color: var(--color-text-tertiary);
    margin-left: auto;
}

.agent-input {
    flex: 1;
    resize: none;
    min-height: 1.9rem;
    max-height: 8rem;
    padding: 0.4rem 0.5rem;
    border: none;
    background: transparent;
    color: var(--color-text-primary);
    font-family: inherit;
    font-size: 0.86rem;
    line-height: 1.5;
    outline: none;
}

.agent-aside {
    width: 16rem;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding-bottom: 0.9rem;
    /* ⚠ flex 子项不加 min-height:0，overflow-y 就是个摆设 —— 内容会把右栏撑高，
       连带整个 body 溢出，看起来就是「右栏跟着对话区一起滚」。 */
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
}

.agent-perm {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.6rem;
}

.agent-hint {
    margin: 0.3rem 0 0;
    font-size: 0.72rem;
    line-height: 1.5;
    color: var(--color-text-tertiary);
}

.agent-hint-gap {
    margin-top: 0.55rem;
}

.agent-card-title {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--color-text-primary);
}

.agent-model-btn {
    width: 100%;
    justify-content: flex-start;
}

.agent-model-note {
    margin-left: auto;
    max-width: 8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}

.agent-steps {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
}

.agent-steps li {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.76rem;
    color: var(--color-text-secondary);
}

.agent-step-ok {
    width: 0.85rem;
    height: 0.85rem;
    flex-shrink: 0;
    color: var(--c-ok);
}

.agent-step-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono, monospace);
}

.agent-step-state {
    flex-shrink: 0;
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}

.agent-form {
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

.agent-field {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
}

.agent-field > label {
    font-size: 0.8rem;
    color: var(--color-text-secondary);
}

.agent-row {
    display: flex;
    gap: 0.5rem;
}

.agent-history {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
}

.agent-history li {
    display: flex;
    align-items: center;
    gap: 0.35rem;
}

.agent-history-item {
    flex: 1;
    min-width: 0;
    justify-content: flex-start;
}

.agent-history-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
}

.agent-history-time {
    flex-shrink: 0;
    font-size: 0.72rem;
    color: var(--color-text-tertiary);
}
</style>
