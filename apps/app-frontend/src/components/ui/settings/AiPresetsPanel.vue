<script setup lang="ts">
/**
 * 「人格与方案」面板。
 *
 * 一个方案 = 人格（Persona）+ 规则（Rules）+ 绑定的厂商/模型，存成一条可复用的成品。
 * 机器人和 Agent 将来只读 `activeId`，不必各自维护一份人格配置。
 *
 * 自包含组件：只从父级拿「可选的厂商 / 模型」，其余全部自己读写，
 * 免得把 1900 行的 AISettings.vue 再撑大一圈。
 */
import { CheckIcon, PlusIcon, SpinnerIcon, TrashIcon } from '@modrinth/assets'
import { Button, Combobox, defineMessages, injectNotificationManager, StyledInput, useVIntl } from '@modrinth/ui'
import { computed, onMounted, ref } from 'vue'

import {
    type AIPreset,
    completeWithAIPreset,
    listAIPresets,
    removeAIPreset,
    saveAIPreset,
    setActiveAIPreset,
} from '@/helpers/ai'

const props = defineProps<{
    /** 已配置的厂商（含其模型），用来绑定方案。 */
    providers: Array<{
        providerId: string
        label: string
        models: Array<{ id: string; name: string }>
    }>
}>()

const { formatMessage } = useVIntl()
const { addNotification } = injectNotificationManager()

const messages = defineMessages({
    title: { id: 'app.ai-presets.title', defaultMessage: 'Persona & presets' },
    description: {
        id: 'app.ai-presets.description',
        defaultMessage:
            'A preset bundles a persona, its rules and the model that runs it. Bots and agents pick a preset instead of carrying their own instructions.',
    },
    empty: {
        id: 'app.ai-presets.empty',
        defaultMessage: 'No presets yet. Create one to describe how the model should behave.',
    },
    create: { id: 'app.ai-presets.create', defaultMessage: 'New preset' },
    duplicate: { id: 'app.ai-presets.duplicate', defaultMessage: 'Duplicate' },
    name: { id: 'app.ai-presets.name', defaultMessage: 'Name' },
    namePlaceholder: { id: 'app.ai-presets.name.placeholder', defaultMessage: 'e.g. 阿罗娜 · 助手' },
    provider: { id: 'app.ai-presets.provider', defaultMessage: 'Provider' },
    model: { id: 'app.ai-presets.model', defaultMessage: 'Model' },
    followProvider: {
        id: 'app.ai-presets.follow-provider',
        defaultMessage: 'Follow the calling feature',
    },
    persona: { id: 'app.ai-presets.persona', defaultMessage: 'Persona' },
    personaHint: {
        id: 'app.ai-presets.persona.hint',
        defaultMessage: 'Who the model is. Sent first, as the opening of the system prompt.',
    },
    personaPlaceholder: {
        id: 'app.ai-presets.persona.placeholder',
        defaultMessage: 'You are Arona, the assistant of the Sensei. You are cheerful but concise.',
    },
    rules: { id: 'app.ai-presets.rules', defaultMessage: 'Rules' },
    rulesHint: {
        id: 'app.ai-presets.rules.hint',
        defaultMessage:
            'What it must or must not do. Always placed after the persona, so it overrides it.',
    },
    rulesPlaceholder: {
        id: 'app.ai-presets.rules.placeholder',
        defaultMessage: 'Answer in Chinese.\nNever invent file paths or instance names.',
    },
    save: { id: 'app.ai-presets.save', defaultMessage: 'Save' },
    remove: { id: 'app.ai-presets.remove', defaultMessage: 'Delete' },
    setActive: { id: 'app.ai-presets.set-active', defaultMessage: 'Use this preset' },
    active: { id: 'app.ai-presets.active', defaultMessage: 'In use' },
    test: { id: 'app.ai-presets.test', defaultMessage: 'Try it' },
    testPlaceholder: { id: 'app.ai-presets.test.placeholder', defaultMessage: 'Say something…' },
    testReply: { id: 'app.ai-presets.test.reply', defaultMessage: 'Reply' },
    saved: { id: 'app.ai-presets.saved', defaultMessage: 'Preset saved' },
    deleted: { id: 'app.ai-presets.deleted', defaultMessage: 'Preset deleted' },
    needsName: { id: 'app.ai-presets.needs-name', defaultMessage: 'Give the preset a name first.' },
    needsModel: {
        id: 'app.ai-presets.needs-model',
        defaultMessage: 'Bind a provider and a model before running this preset.',
    },
})

const loading = ref(true)
const busy = ref(false)
const presets = ref<AIPreset[]>([])
const activeId = ref<string | null>(null)
/** 正在编辑的草稿；`id` 为空表示还没落盘的新方案。 */
const draft = ref<AIPreset | null>(null)
const testPrompt = ref('')
const testReply = ref('')

onMounted(() => {
    void refresh()
})

async function refresh(): Promise<void> {
    try {
        const state = await listAIPresets()
        presets.value = state.presets
        activeId.value = state.activeId
        // 选中的草稿还在就保留，否则落到第一条，避免右侧空白。
        if (draft.value?.id) {
            draft.value = state.presets.find((item) => item.id === draft.value?.id) ?? null
        }
    } catch (error) {
        reportError(error)
    } finally {
        loading.value = false
    }
}

function reportError(error: unknown): void {
    // Tauri 的命令错误是 `{ fieldName, code, message }` 对象，不是 Error ——
    // 直接 String() 会得到 "[object Object]"，用户看到的就只是一句废话。
    const text =
        error instanceof Error
            ? error.message
            : error && typeof error === 'object' && 'message' in error
              ? String((error as { message: unknown }).message)
              : String(error)
    addNotification({ title: formatMessage(messages.title), text, type: 'error' })
}

function blankPreset(): AIPreset {
    const now = Date.now()
    return {
        id: '',
        name: '',
        providerId: '',
        modelId: '',
        persona: '',
        rules: '',
        createdAt: now,
        updatedAt: now,
    }
}

function createPreset(): void {
    draft.value = blankPreset()
    testReply.value = ''
}

function selectPreset(preset: AIPreset): void {
    draft.value = { ...preset }
    testReply.value = ''
}

/** 复制当前草稿成一个新方案：人格那段通常要微调而不是重写。 */
function duplicatePreset(): void {
    if (!draft.value) return
    draft.value = { ...draft.value, id: '', name: `${draft.value.name} 副本`, createdAt: Date.now() }
}

const providerOptions = computed(() => [
    { value: '', label: formatMessage(messages.followProvider) },
    ...props.providers.map((provider) => ({ value: provider.providerId, label: provider.label })),
])

const modelOptions = computed(() => {
    const provider = props.providers.find((item) => item.providerId === draft.value?.providerId)
    return [
        { value: '', label: formatMessage(messages.followProvider) },
        ...(provider?.models ?? []).map((model) => ({ value: model.id, label: model.name || model.id })),
    ]
})

/** 换厂商时模型必须一起清空，否则会留下一个不属于该厂商的 model id。 */
function onProviderChange(value: string): void {
    if (!draft.value) return
    draft.value.providerId = value
    const provider = props.providers.find((item) => item.providerId === value)
    const stillValid = provider?.models.some((model) => model.id === draft.value?.modelId)
    if (!stillValid) draft.value.modelId = ''
}

async function save(): Promise<void> {
    const current = draft.value
    if (!current) return
    if (!current.name.trim()) {
        addNotification({ title: formatMessage(messages.title), text: formatMessage(messages.needsName), type: 'warning' })
        return
    }
    busy.value = true
    try {
        const saved = await saveAIPreset({
            id: current.id || null,
            name: current.name.trim(),
            providerId: current.providerId,
            modelId: current.modelId,
            persona: current.persona,
            rules: current.rules,
        })
        draft.value = { ...saved }
        await refresh()
        addNotification({ title: formatMessage(messages.title), text: formatMessage(messages.saved), type: 'success' })
    } catch (error) {
        reportError(error)
    } finally {
        busy.value = false
    }
}

async function remove(): Promise<void> {
    const current = draft.value
    if (!current?.id) return
    busy.value = true
    try {
        await removeAIPreset(current.id)
        draft.value = null
        await refresh()
        addNotification({ title: formatMessage(messages.title), text: formatMessage(messages.deleted), type: 'success' })
    } catch (error) {
        reportError(error)
    } finally {
        busy.value = false
    }
}

async function useThis(): Promise<void> {
    const current = draft.value
    if (!current?.id) return
    try {
        await setActiveAIPreset(current.id)
        await refresh()
    } catch (error) {
        reportError(error)
    }
}

/** 试运行：用当前（已保存的）方案发一句话，验证人格与规则是否真的生效。 */
async function runTest(): Promise<void> {
    const current = draft.value
    if (!current?.id) return
    if (!testPrompt.value.trim()) return
    if (!current.providerId || !current.modelId) {
        addNotification({ title: formatMessage(messages.title), text: formatMessage(messages.needsModel), type: 'warning' })
        return
    }
    busy.value = true
    testReply.value = ''
    try {
        testReply.value = await completeWithAIPreset(current.id, testPrompt.value.trim())
    } catch (error) {
        reportError(error)
    } finally {
        busy.value = false
    }
}
</script>

<template>
    <section class="ai-preset-layout">
        <aside class="ai-preset-sidebar">
            <header class="ai-preset-sidebar-header">
                <h2 class="m-0 text-base font-bold text-[var(--color-text-primary)]">
                    {{ formatMessage(messages.title) }}
                </h2>
                <Button type="base" @click="createPreset">
                    <PlusIcon />{{ formatMessage(messages.create) }}
                </Button>
            </header>

            <div v-if="loading" class="flex justify-center py-8">
                <SpinnerIcon class="size-5 animate-spin text-[var(--color-text-tertiary)]" />
            </div>

            <p v-else-if="!presets.length" class="m-0 px-3 py-6 text-sm text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.empty) }}
            </p>

            <div v-else class="ai-preset-list">
                <button
                    v-for="preset in presets"
                    :key="preset.id"
                    type="button"
                    class="ai-preset-item"
                    :class="{ selected: draft?.id === preset.id }"
                    @click="selectPreset(preset)"
                >
                    <span class="min-w-0 flex-1 truncate text-left text-sm font-semibold">
                        {{ preset.name }}
                    </span>
                    <span
                        v-if="activeId === preset.id"
                        class="shrink-0 rounded-full bg-surface-4 px-2 py-0.5 text-xs font-semibold text-[var(--color-text-secondary)]"
                    >
                        {{ formatMessage(messages.active) }}
                    </span>
                </button>
            </div>
        </aside>

        <div class="ai-preset-detail">
            <div v-if="!draft" class="ai-preset-placeholder">
                <p class="m-0 max-w-96 text-center text-sm text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.description) }}
                </p>
            </div>

            <template v-else>
                <header class="ai-preset-detail-header">
                    <p class="m-0 min-w-0 flex-1 text-sm text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.description) }}
                    </p>
                    <div class="flex shrink-0 flex-wrap items-center justify-end gap-2">
                        <Button v-if="draft.id" type="quiet" @click="duplicatePreset">
                            {{ formatMessage(messages.duplicate) }}
                        </Button>
                        <Button v-if="draft.id && activeId !== draft.id" type="base" @click="useThis">
                            {{ formatMessage(messages.setActive) }}
                        </Button>
                        <Button v-if="draft.id" type="quiet" color="red" @click="remove">
                            <TrashIcon />{{ formatMessage(messages.remove) }}
                        </Button>
                        <Button type="colored" color="brand" :disabled="busy" @click="save">
                            <CheckIcon />{{ formatMessage(messages.save) }}
                        </Button>
                    </div>
                </header>

                <div class="ai-preset-scroll">
                    <label class="ai-preset-field">
                        <span class="ai-preset-label">{{ formatMessage(messages.name) }}</span>
                        <StyledInput
                            v-model="draft.name"
                            :placeholder="formatMessage(messages.namePlaceholder)"
                            wrapper-class="w-full"
                        />
                    </label>

                    <div class="ai-preset-row">
                        <label class="ai-preset-field">
                            <span class="ai-preset-label">{{ formatMessage(messages.provider) }}</span>
                            <Combobox
                                :model-value="draft.providerId"
                                :options="providerOptions"
                                @update:model-value="onProviderChange"
                            />
                        </label>
                        <label class="ai-preset-field">
                            <span class="ai-preset-label">{{ formatMessage(messages.model) }}</span>
                            <Combobox
                                v-model="draft.modelId"
                                :options="modelOptions"
                                :disabled="!draft.providerId"
                            />
                        </label>
                    </div>

                    <label class="ai-preset-field">
                        <span class="ai-preset-label">{{ formatMessage(messages.persona) }}</span>
                        <span class="ai-preset-hint">{{ formatMessage(messages.personaHint) }}</span>
                        <textarea
                            v-model="draft.persona"
                            class="ai-preset-textarea"
                            rows="4"
                            :placeholder="formatMessage(messages.personaPlaceholder)"
                        />
                    </label>

                    <label class="ai-preset-field">
                        <span class="ai-preset-label">{{ formatMessage(messages.rules) }}</span>
                        <span class="ai-preset-hint">{{ formatMessage(messages.rulesHint) }}</span>
                        <textarea
                            v-model="draft.rules"
                            class="ai-preset-textarea"
                            rows="4"
                            :placeholder="formatMessage(messages.rulesPlaceholder)"
                        />
                    </label>

                    <div class="ai-preset-test">
                        <span class="ai-preset-label">{{ formatMessage(messages.test) }}</span>
                        <div class="flex gap-2">
                            <StyledInput
                                v-model="testPrompt"
                                :placeholder="formatMessage(messages.testPlaceholder)"
                                wrapper-class="flex-1"
                                @keydown.enter.prevent="runTest"
                            />
                            <Button
                                type="base"
                                :disabled="busy || !draft.id || !testPrompt.trim()"
                                @click="runTest"
                            >
                                <SpinnerIcon v-if="busy" class="size-4 animate-spin" />
                                <span v-else>{{ formatMessage(messages.test) }}</span>
                            </Button>
                        </div>
                        <div v-if="testReply" class="ai-preset-reply">
                            <span class="ai-preset-hint">{{ formatMessage(messages.testReply) }}</span>
                            <p class="m-0 whitespace-pre-wrap text-sm text-[var(--color-text-primary)]">
                                {{ testReply }}
                            </p>
                        </div>
                        <span v-else-if="!draft.id" class="ai-preset-hint">
                            {{ formatMessage(messages.needsName) }}
                        </span>
                    </div>
                </div>
            </template>
        </div>
    </section>
</template>

<style scoped>
.ai-preset-layout {
    display: grid;
    height: 100%;
    min-height: 0;
    grid-template-columns: 17rem minmax(0, 1fr);
}

.ai-preset-sidebar {
    display: flex;
    min-height: 0;
    flex-direction: column;
    gap: var(--gap-sm, 0.5rem);
    padding: var(--gap-lg);
    border-right: 1px solid
        var(--settings-divider, color-mix(in srgb, var(--surface-4) 55%, transparent));
}

.ai-preset-sidebar-header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: var(--gap-sm, 0.5rem);
}

.ai-preset-list {
    display: flex;
    min-height: 0;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
}

.ai-preset-item {
    display: flex;
    align-items: center;
    gap: var(--gap-sm, 0.5rem);
    padding: 0.5rem 0.625rem;
    border: none;
    border-radius: 0.5rem;
    background: transparent;
    color: var(--color-text-primary);
    cursor: pointer;
    text-align: left;
}

.ai-preset-item:hover {
    background: var(--surface-4);
}

.ai-preset-item.selected {
    background: var(--surface-4);
}

.ai-preset-detail {
    display: flex;
    min-height: 0;
    min-width: 0;
    flex-direction: column;
    overflow: hidden;
}

.ai-preset-placeholder {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    padding: var(--gap-xl);
}

.ai-preset-detail-header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: var(--gap-lg);
    padding: var(--gap-lg) var(--gap-xl);
    border-bottom: 1px solid
        var(--settings-divider, color-mix(in srgb, var(--surface-4) 55%, transparent));
}

.ai-preset-scroll {
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    gap: var(--gap-lg);
    overflow-y: auto;
    padding: var(--gap-xl);
    scrollbar-gutter: stable;
}

.ai-preset-row {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--gap-lg);
}

.ai-preset-field {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.375rem;
}

.ai-preset-label {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-text-primary);
}

.ai-preset-hint {
    font-size: 0.75rem;
    line-height: 1.4;
    color: var(--color-text-tertiary);
}

.ai-preset-textarea {
    width: 100%;
    min-height: 5rem;
    padding: 0.625rem 0.75rem;
    border: 1px solid var(--surface-5);
    border-radius: 0.5rem;
    background: var(--surface-2);
    color: var(--color-text-primary);
    font-family: inherit;
    font-size: 0.875rem;
    line-height: 1.5;
    resize: vertical;
}

.ai-preset-textarea:focus-visible {
    border-color: var(--color-brand);
    outline: none;
}

.ai-preset-test {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding-top: var(--gap-md, 1rem);
    border-top: 1px solid
        var(--settings-divider, color-mix(in srgb, var(--surface-4) 55%, transparent));
}

.ai-preset-reply {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.75rem;
    border-radius: 0.5rem;
    background: var(--surface-3);
}

@media (max-width: 900px) {
    .ai-preset-layout {
        grid-template-columns: minmax(0, 1fr);
    }

    .ai-preset-sidebar {
        border-right: none;
        border-bottom: 1px solid
            var(--settings-divider, color-mix(in srgb, var(--surface-4) 55%, transparent));
    }

    .ai-preset-row {
        grid-template-columns: minmax(0, 1fr);
    }
}
</style>
