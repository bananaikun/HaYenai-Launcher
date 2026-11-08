<script setup lang="ts">
import { ArrowUpIcon, BotIcon, CheckIcon, TrashIcon, XIcon } from '@modrinth/assets'
import { defineMessages, useVIntl } from '@modrinth/ui'
import { computed, nextTick, onMounted, ref } from 'vue'

import {
    agentErrorMessage,
    agentRunTool,
    agentTools,
    agentTurn,
    type AgentMessage,
    type AgentToolDefinition,
} from '@/helpers/agent'
import { listAIPresets, type AIPresetState } from '@/helpers/ai'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: { id: 'app.agent.title', defaultMessage: 'Agent' },
    subtitle: {
        id: 'app.agent.subtitle',
        defaultMessage: 'Reads your instances and logs, and changes them only when you allow it.',
    },
    preset: { id: 'app.agent.preset', defaultMessage: 'Preset' },
    noPreset: { id: 'app.agent.no-preset', defaultMessage: 'No preset' },
    noPresetHint: {
        id: 'app.agent.no-preset-hint',
        defaultMessage: 'Create a persona preset under AI settings first.',
    },
    allowWrite: { id: 'app.agent.allow-write', defaultMessage: 'Allow changes' },
    allowWriteHint: {
        id: 'app.agent.allow-write-hint',
        defaultMessage: 'Renaming instances and changing memory',
    },
    readOnly: { id: 'app.agent.read-only', defaultMessage: 'Read-only' },
    clear: { id: 'app.agent.clear', defaultMessage: 'Clear' },
    placeholder: {
        id: 'app.agent.placeholder',
        defaultMessage: 'Ask about an instance, a crash, or a setting…',
    },
    send: { id: 'app.agent.send', defaultMessage: 'Send' },
    empty: {
        id: 'app.agent.empty',
        defaultMessage: 'Ask something. It will read real files instead of guessing.',
    },
    toolsTitle: { id: 'app.agent.tools', defaultMessage: 'Tools' },
    thinking: { id: 'app.agent.thinking', defaultMessage: 'Working…' },
    stop: { id: 'app.agent.stop', defaultMessage: 'Stop' },
    you: { id: 'app.agent.you', defaultMessage: 'You' },
})

interface ToolCard {
    id: string
    name: string
    args: unknown
    result?: unknown
    error?: string
    running: boolean
}

interface Bubble {
    role: 'user' | 'assistant'
    text: string
    tools: ToolCard[]
}

const bubbles = ref<Bubble[]>([])
const draft = ref('')
const busy = ref(false)
const allowWrite = ref(false)
const presetState = ref<AIPresetState>({ presets: [], activeId: null })
const presetId = ref<string | null>(null)
const tools = ref<AgentToolDefinition[]>([])
const threadEl = ref<HTMLElement | null>(null)
/** 真正发给模型的整段对话。UI 只渲染 bubbles，两者形状不同。 */
let history: AgentMessage[] = []
let cancelled = false

const activePreset = computed(
    () => presetState.value.presets.find((preset) => preset.id === presetId.value) ?? null,
)
const modelBound = computed(
    () => !!activePreset.value?.providerId && !!activePreset.value?.modelId,
)
const canSend = computed(() => !busy.value && draft.value.trim().length > 0 && modelBound.value)

async function scrollToBottom() {
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

async function loadTools() {
    try {
        tools.value = await agentTools(allowWrite.value)
    } catch {
        tools.value = []
    }
}

async function toggleWrite() {
    allowWrite.value = !allowWrite.value
    await loadTools()
}

function clearAll() {
    bubbles.value = []
    history = []
}

/**
 * 一轮完整对话。
 *
 * 循环结构：模型可能连续调多个工具，每次都要把结果回灌再问一遍 ——
 * 只跑一次 turn 的话，模型永远只能"说要做什么"，做不了。
 */
async function send() {
    const text = draft.value.trim()
    if (!text || busy.value || !modelBound.value) return

    draft.value = ''
    busy.value = true
    cancelled = false

    bubbles.value.push({ role: 'user', text, tools: [] })
    history.push({ role: 'user', content: text })

    const reply: Bubble = { role: 'assistant', text: '', tools: [] }
    bubbles.value.push(reply)
    await scrollToBottom()

    try {
        for (let round = 0; round < 8; round += 1) {
            if (cancelled) break
            const message = await agentTurn(
                {
                    presetId: presetId.value,
                    allowWrite: allowWrite.value,
                    messages: history,
                },
                (event) => {
                    if (event.kind === 'delta') {
                        reply.text += event.text
                        void scrollToBottom()
                    }
                },
            )

            history.push(message)
            if (message.content?.trim()) reply.text = message.content

            const calls = message.tool_calls ?? []
            if (calls.length === 0) break

            for (const call of calls) {
                if (cancelled) break
                const card: ToolCard = {
                    id: call.id,
                    name: call.name,
                    args: call.args,
                    running: true,
                }
                reply.tools.push(card)
                await scrollToBottom()
                try {
                    const result = await agentRunTool(call.name, call.args, allowWrite.value)
                    card.result = result
                    card.running = false
                    history.push({
                        role: 'tool',
                        tool_call_id: call.id,
                        content: JSON.stringify(result),
                    })
                } catch (error) {
                    card.error = agentErrorMessage(error)
                    card.running = false
                    // 工具失败也要回灌，否则下一轮模型会以为这一步没发生过。
                    history.push({
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
        busy.value = false
        await scrollToBottom()
    }
}

function stop() {
    cancelled = true
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

onMounted(async () => {
    await Promise.all([loadPresets(), loadTools()])
})
</script>

<template>
    <div class="ag">
        <header class="ag-bar">
            <BotIcon class="ag-bar-icon" />
            <div class="ag-bar-text">
                <h1>{{ formatMessage(messages.title) }}</h1>
                <p>{{ formatMessage(messages.subtitle) }}</p>
            </div>

            <label class="ag-preset">
                <span>{{ formatMessage(messages.preset) }}</span>
                <select v-model="presetId">
                    <option v-if="!presetState.presets.length" :value="null">
                        {{ formatMessage(messages.noPreset) }}
                    </option>
                    <option v-for="preset in presetState.presets" :key="preset.id" :value="preset.id">
                        {{ preset.name }}
                    </option>
                </select>
            </label>

            <button
                type="button"
                class="ag-toggle"
                :class="{ on: allowWrite }"
                @click="toggleWrite"
            >
                <CheckIcon v-if="allowWrite" class="size-3.5" />
                <XIcon v-else class="size-3.5" />
                {{ formatMessage(allowWrite ? messages.allowWrite : messages.readOnly) }}
            </button>

            <button
                type="button"
                class="ag-icon"
                :disabled="!bubbles.length"
                v-tooltip="formatMessage(messages.clear)"
                @click="clearAll"
            >
                <TrashIcon class="size-4" />
            </button>
        </header>

        <div class="ag-body">
            <div ref="threadEl" class="ag-thread">
                <p v-if="!bubbles.length" class="ag-empty">{{ formatMessage(messages.empty) }}</p>

                <div
                    v-for="(bubble, index) in bubbles"
                    :key="index"
                    class="ag-bubble"
                    :class="bubble.role"
                >
                    <div v-if="bubble.text" class="ag-text">{{ bubble.text }}</div>

                    <div v-for="card in bubble.tools" :key="card.id" class="ag-tool">
                        <div class="ag-tool-head">
                            <code>{{ card.name }}</code>
                            <span v-if="card.running" class="ag-tool-spin" />
                            <span v-else-if="card.error" class="ag-tool-bad">!</span>
                        </div>
                        <div v-if="prettyArgs(card.args)" class="ag-tool-args">
                            {{ prettyArgs(card.args) }}
                        </div>
                        <details v-if="!card.running && !card.error" class="ag-tool-out">
                            <summary>{{ formatMessage(messages.toolsTitle) }}</summary>
                            <pre>{{ prettyResult(card.result) }}</pre>
                        </details>
                        <div v-else-if="card.error" class="ag-tool-err">{{ card.error }}</div>
                    </div>
                </div>

                <p v-if="busy" class="ag-working">{{ formatMessage(messages.thinking) }}</p>
            </div>

            <aside class="ag-side">
                <h2>{{ formatMessage(messages.toolsTitle) }}</h2>
                <p v-if="!modelBound" class="ag-side-warn">
                    {{ formatMessage(messages.noPresetHint) }}
                </p>
                <ul>
                    <li v-for="tool in tools" :key="tool.function.name">
                        <code>{{ tool.function.name }}</code>
                        <span>{{ tool.function.description }}</span>
                    </li>
                </ul>
            </aside>
        </div>

        <footer class="ag-compose">
            <textarea
                v-model="draft"
                :placeholder="formatMessage(messages.placeholder)"
                rows="1"
                @keydown="onKeydown"
            />
            <button v-if="busy" type="button" class="ag-send stop" @click="stop">
                {{ formatMessage(messages.stop) }}
            </button>
            <button
                v-else
                type="button"
                class="ag-send"
                :disabled="!canSend"
                v-tooltip="formatMessage(messages.send)"
                @click="send"
            >
                <ArrowUpIcon class="size-4" />
            </button>
        </footer>
    </div>
</template>

<style scoped>
.ag {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    gap: 0.75rem;
    padding: 1.25rem 1.5rem;
    box-sizing: border-box;
}

.ag-bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-shrink: 0;
}

.ag-bar-icon {
    width: 1.35rem;
    height: 1.35rem;
    color: var(--color-brand);
    flex-shrink: 0;
}

.ag-bar-text {
    min-width: 0;
    margin-right: auto;
}

.ag-bar-text h1 {
    margin: 0;
    font-size: 1.2rem;
    font-weight: 600;
    color: var(--color-text-primary);
}

.ag-bar-text p {
    margin: 0;
    font-size: 0.8rem;
    color: var(--color-text-tertiary);
}

.ag-preset {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8rem;
    color: var(--color-text-secondary);
}

.ag-preset select {
    background: var(--surface-3, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--color-divider);
    border-radius: 0.5rem;
    color: var(--color-text-primary);
    padding: 0.3rem 0.5rem;
    font-size: 0.8rem;
    max-width: 12rem;
}

.ag-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.35rem 0.7rem;
    border-radius: 999px;
    border: 1px solid var(--color-divider);
    background: transparent;
    color: var(--color-text-secondary);
    font-size: 0.78rem;
    cursor: pointer;
}

.ag-toggle.on {
    color: #f0b429;
    border-color: rgba(240, 180, 41, 0.45);
}

.ag-icon {
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border-radius: 0.5rem;
    border: 1px solid var(--color-divider);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.ag-icon:disabled {
    opacity: 0.35;
    cursor: default;
}

.ag-body {
    display: flex;
    gap: 0.75rem;
    flex: 1;
    min-height: 0;
}

.ag-thread {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding-right: 0.25rem;
}

.ag-empty {
    margin: auto;
    color: var(--color-text-tertiary);
    font-size: 0.85rem;
    text-align: center;
    max-width: 22rem;
}

.ag-bubble {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-width: 46rem;
}

.ag-bubble.user {
    align-self: flex-end;
    align-items: flex-end;
}

.ag-text {
    padding: 0.6rem 0.85rem;
    border-radius: 0.85rem;
    font-size: 0.88rem;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--color-text-primary);
    background: var(--surface-3, rgba(255, 255, 255, 0.07));
    border: 1px solid var(--color-divider);
}

.ag-bubble.user .ag-text {
    background: rgba(var(--color-brand-rgb, 120, 160, 255), 0.16);
    border-color: rgba(var(--color-brand-rgb, 120, 160, 255), 0.32);
}

.ag-tool {
    border: 1px solid var(--color-divider);
    border-radius: 0.6rem;
    padding: 0.4rem 0.6rem;
    font-size: 0.78rem;
    background: var(--surface-2, rgba(255, 255, 255, 0.04));
}

.ag-tool-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--color-text-secondary);
}

.ag-tool-head code {
    font-size: 0.76rem;
    color: var(--color-text-primary);
}

.ag-tool-spin {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 50%;
    border: 1.5px solid var(--color-text-tertiary);
    border-top-color: transparent;
    animation: ag-spin 0.7s linear infinite;
}

@keyframes ag-spin {
    to {
        transform: rotate(360deg);
    }
}

.ag-tool-bad {
    color: #e5534b;
    font-weight: 700;
}

.ag-tool-args {
    color: var(--color-text-tertiary);
    font-family: var(--font-mono, monospace);
    font-size: 0.72rem;
    margin-top: 0.2rem;
    word-break: break-all;
}

.ag-tool-out summary {
    cursor: pointer;
    color: var(--color-text-tertiary);
    font-size: 0.72rem;
    margin-top: 0.25rem;
}

.ag-tool-out pre {
    margin: 0.3rem 0 0;
    max-height: 14rem;
    overflow: auto;
    font-size: 0.7rem;
    color: var(--color-text-secondary);
    background: rgba(0, 0, 0, 0.18);
    border-radius: 0.4rem;
    padding: 0.4rem;
}

.ag-tool-err {
    margin-top: 0.25rem;
    color: #e5534b;
    font-size: 0.74rem;
}

.ag-working {
    margin: 0;
    font-size: 0.78rem;
    color: var(--color-text-tertiary);
}

.ag-side {
    width: 15rem;
    flex-shrink: 0;
    overflow-y: auto;
    border-left: 1px solid var(--color-divider);
    padding-left: 0.85rem;
}

.ag-side h2 {
    margin: 0 0 0.5rem;
    font-size: 0.74rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--color-text-tertiary);
}

.ag-side-warn {
    margin: 0 0 0.5rem;
    font-size: 0.76rem;
    color: #f0b429;
}

.ag-side ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
}

.ag-side li {
    display: flex;
    flex-direction: column;
    gap: 0.12rem;
}

.ag-side li code {
    font-size: 0.74rem;
    color: var(--color-text-primary);
}

.ag-side li span {
    font-size: 0.72rem;
    line-height: 1.4;
    color: var(--color-text-tertiary);
}

.ag-compose {
    display: flex;
    align-items: flex-end;
    gap: 0.5rem;
    flex-shrink: 0;
}

.ag-compose textarea {
    flex: 1;
    resize: none;
    min-height: 2.5rem;
    max-height: 8rem;
    padding: 0.6rem 0.8rem;
    border-radius: 0.7rem;
    border: 1px solid var(--color-divider);
    background: var(--surface-3, rgba(255, 255, 255, 0.07));
    color: var(--color-text-primary);
    font-size: 0.86rem;
    font-family: inherit;
    line-height: 1.4;
}

.ag-compose textarea:focus {
    outline: none;
    border-color: rgba(var(--color-brand-rgb, 120, 160, 255), 0.5);
}

.ag-send {
    display: grid;
    place-items: center;
    width: 2.5rem;
    height: 2.5rem;
    flex-shrink: 0;
    border-radius: 0.7rem;
    border: none;
    background: var(--color-brand);
    color: #fff;
    cursor: pointer;
    font-size: 0.76rem;
}

.ag-send:disabled {
    opacity: 0.4;
    cursor: default;
}

.ag-send.stop {
    width: auto;
    padding: 0 0.85rem;
    background: rgba(229, 83, 75, 0.85);
}
</style>
