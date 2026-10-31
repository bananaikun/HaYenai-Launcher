<script setup lang="ts">
import { BugIcon, ReportIcon, SupportChatIcon } from '@modrinth/assets'
import {
    Button,
    Checkbox,
    Chips,
    commonMessages,
    defineMessages,
    injectNotificationManager,
    NewModal,
    StyledInput,
    useVIntl,
} from '@modrinth/ui'
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, ref } from 'vue'

import {
    buildIssueBody,
    buildIssueUrl,
    collectEnvironment,
    environmentBlock,
    FEEDBACK_LOG_LINE_OPTIONS,
    fetchLogTail,
    type FeedbackEnvironment,
    type FeedbackKind,
    type FeedbackLogTail,
} from '@/helpers/feedback'
import { exportLauncherLogs } from '@/helpers/utils'
import { useError } from '@/store/error'

/**
 * 反馈面板：崩溃 / 功能异常 / 建议。
 *
 * 提交走 GitHub 的 new-issue 预填页 —— 应用内不带任何写权限 token。
 * 附带信息全部可取消勾选，默认三项（环境、日志尾部、最近报错）是排查最省事的组合。
 */
const { formatMessage } = useVIntl()
const { addNotification, handleError } = injectNotificationManager()
const errorStore = useError()

const messages = defineMessages({
    trigger: { id: 'app.feedback.trigger', defaultMessage: 'Send feedback' },
    title: { id: 'app.feedback.title', defaultMessage: 'Send feedback' },
    intro: {
        id: 'app.feedback.intro',
        defaultMessage:
            'Crash, bug, or an idea — this opens a pre-filled GitHub issue. You review and submit it in the browser, so nothing is sent without you seeing it first.',
    },
    kindCrash: { id: 'app.feedback.kind.crash', defaultMessage: 'Crash' },
    kindBug: { id: 'app.feedback.kind.bug', defaultMessage: 'Bug' },
    kindSuggestion: { id: 'app.feedback.kind.suggestion', defaultMessage: 'Suggestion' },
    summaryLabel: { id: 'app.feedback.summary', defaultMessage: 'Summary' },
    summaryPlaceholder: {
        id: 'app.feedback.summary.placeholder',
        defaultMessage: 'One line: what went wrong?',
    },
    detailLabel: { id: 'app.feedback.detail', defaultMessage: 'Details' },
    detailPlaceholder: {
        id: 'app.feedback.detail.placeholder',
        defaultMessage:
            'What were you doing? What did you expect, and what happened instead? Steps that reproduce it help the most.',
    },
    attachTitle: { id: 'app.feedback.attach', defaultMessage: 'Attach diagnostics' },
    attachEnv: { id: 'app.feedback.attach.env', defaultMessage: 'System & environment' },
    attachEnvHint: {
        id: 'app.feedback.attach.env.hint',
        defaultMessage: 'Version, OS, CPU, memory, GPU, screen, runtime.',
    },
    attachLog: { id: 'app.feedback.attach.log', defaultMessage: 'Launcher log tail' },
    attachLogHint: {
        id: 'app.feedback.attach.log.hint',
        defaultMessage: 'Info level and above. Tokens, Minecraft tokens and IPs are replaced automatically.',
    },
    attachError: { id: 'app.feedback.attach.error', defaultMessage: 'Last error' },
    attachErrorHint: {
        id: 'app.feedback.attach.error.hint',
        defaultMessage: 'Captured by the error dialog — no need to reproduce it again.',
    },
    attachErrorEmpty: {
        id: 'app.feedback.attach.error.empty',
        defaultMessage: 'No error has been captured this session.',
    },
    logState: {
        id: 'app.feedback.log.state',
        defaultMessage: '{lines} lines from {file}',
    },
    logEmpty: { id: 'app.feedback.log.empty', defaultMessage: 'No launcher log found yet.' },
    copy: { id: 'app.feedback.copy', defaultMessage: 'Copy diagnostics' },
    saveLogs: { id: 'app.feedback.save', defaultMessage: 'Save full log bundle' },
    openGithub: { id: 'app.feedback.open', defaultMessage: 'Open on GitHub' },
    copied: { id: 'app.feedback.copied', defaultMessage: 'Diagnostics copied to clipboard.' },
    opened: {
        id: 'app.feedback.opened',
        defaultMessage: 'Opened a pre-filled issue in your browser. Sign in and press Submit.',
    },
    trimmed: {
        id: 'app.feedback.trimmed',
        defaultMessage:
            'The issue body was shortened to fit the link. Use “Copy diagnostics” for the full text.',
    },
    saved: { id: 'app.feedback.saved', defaultMessage: 'Log bundle saved to {path}' },
})

const modal = ref<InstanceType<typeof NewModal> | null>(null)
const kind = ref<FeedbackKind>('bug')
const summary = ref('')
const detail = ref('')
const includeEnv = ref(true)
const includeLog = ref(true)
const includeError = ref(true)
const logLines = ref<number>(200)
const env = ref<FeedbackEnvironment | null>(null)
const logTail = ref<FeedbackLogTail | null>(null)
const busy = ref(false)

const kindItems = computed<FeedbackKind[]>(() => ['crash', 'bug', 'suggestion'])
const lineOptions = [...FEEDBACK_LOG_LINE_OPTIONS]

const lastError = computed(() => errorStore.lastError ?? null)
const canSubmit = computed(() => summary.value.trim().length > 0 || detail.value.trim().length > 0)

function kindLabel(item: FeedbackKind): string {
    if (item === 'crash') return formatMessage(messages.kindCrash)
    if (item === 'bug') return formatMessage(messages.kindBug)
    return formatMessage(messages.kindSuggestion)
}

async function loadLog() {
    logTail.value = await fetchLogTail(logLines.value)
}

function currentDraft() {
    return {
        kind: kind.value,
        title: summary.value,
        description: detail.value,
        environment: includeEnv.value && env.value ? environmentBlock(env.value) : null,
        log: includeLog.value ? logTail.value : null,
        error: includeError.value ? lastError.value : null,
    }
}

function show() {
    void refresh()
    modal.value?.show()
}

async function refresh() {
    busy.value = true
    try {
        ;[env.value, logTail.value] = await Promise.all([
            collectEnvironment(),
            fetchLogTail(logLines.value),
        ])
    } finally {
        busy.value = false
    }
}

function changeLogLines(value: number) {
    logLines.value = value
    void loadLog()
}

async function copyDiagnostics() {
    try {
        await navigator.clipboard.writeText(buildIssueBody(currentDraft()))
        addNotification({ title: formatMessage(messages.copied), type: 'success' })
    } catch (error) {
        handleError(error)
    }
}

async function saveLogBundle() {
    try {
        const path = await exportLauncherLogs({
            range: 'last2Hours',
            level: 'info',
            includeSystemInfo: includeEnv.value,
            includeInstanceLogs: false,
        })
        if (path) {
            addNotification({
                title: formatMessage(messages.saved, { path }),
                type: 'success',
            })
        }
    } catch (error) {
        handleError(error)
    }
}

async function openIssue() {
    const { url, truncated } = buildIssueUrl(currentDraft())
    try {
        await openUrl(url)
        addNotification({
            title: formatMessage(messages.opened),
            text: truncated ? formatMessage(messages.trimmed) : undefined,
            type: 'success',
        })
        modal.value?.hide()
    } catch (error) {
        handleError(error)
    }
}

defineExpose({ show })
</script>

<template>
    <NewModal ref="modal" :header="formatMessage(messages.title)" max-width="42rem" scrollable>
        <div class="flex flex-col gap-4">
            <p class="m-0 text-sm text-secondary">{{ formatMessage(messages.intro) }}</p>

            <Chips v-model="kind" :items="kindItems" :format-label="kindLabel" :never-empty="true" />

            <div class="flex flex-col gap-2">
                <label class="text-sm font-medium text-primary" for="feedback-summary">
                    {{ formatMessage(messages.summaryLabel) }}
                </label>
                <StyledInput
                    id="feedback-summary"
                    v-model="summary"
                    :placeholder="formatMessage(messages.summaryPlaceholder)"
                    :maxlength="120"
                />
            </div>

            <div class="flex flex-col gap-2">
                <label class="text-sm font-medium text-primary" for="feedback-detail">
                    {{ formatMessage(messages.detailLabel) }}
                </label>
                <textarea
                    id="feedback-detail"
                    v-model="detail"
                    rows="6"
                    class="w-full resize-y rounded-xl border border-solid border-surface-5 bg-surface-4 p-3 text-sm text-primary outline-none focus:border-brand"
                    :placeholder="formatMessage(messages.detailPlaceholder)"
                />
            </div>

            <div class="flex flex-col gap-3">
                <div class="text-sm font-medium text-primary">
                    {{ formatMessage(messages.attachTitle) }}
                </div>

                <div class="flex flex-col gap-1">
                    <Checkbox v-model="includeEnv" :label="formatMessage(messages.attachEnv)" />
                    <span class="pl-8 text-xs text-secondary">{{
                        formatMessage(messages.attachEnvHint)
                    }}</span>
                </div>

                <div class="flex flex-col gap-1">
                    <Checkbox v-model="includeLog" :label="formatMessage(messages.attachLog)" />
                    <span class="pl-8 text-xs text-secondary">{{
                        formatMessage(messages.attachLogHint)
                    }}</span>
                    <div v-if="includeLog" class="flex flex-wrap items-center gap-3 pl-8 pt-1">
                        <Chips
                            :model-value="logLines"
                            :items="lineOptions"
                            :format-label="(n: number) => `${n}`"
                            :never-empty="true"
                            size="small"
                            @update:model-value="changeLogLines"
                        />
                        <span class="text-xs text-secondary">
                            {{
                                logTail && logTail.lines
                                    ? formatMessage(messages.logState, {
                                          lines: logTail.lines,
                                          file: logTail.file ?? '—',
                                      })
                                    : formatMessage(messages.logEmpty)
                            }}
                        </span>
                    </div>
                </div>

                <div class="flex flex-col gap-1">
                    <Checkbox
                        v-model="includeError"
                        :disabled="!lastError"
                        :label="formatMessage(messages.attachError)"
                    />
                    <span class="pl-8 text-xs text-secondary">
                        {{
                            lastError
                                ? formatMessage(messages.attachErrorHint)
                                : formatMessage(messages.attachErrorEmpty)
                        }}
                    </span>
                </div>
            </div>

            <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex flex-wrap gap-2">
                    <Button type="quiet" :disabled="busy" @click="copyDiagnostics">
                        {{ formatMessage(messages.copy) }}
                    </Button>
                    <Button type="quiet" :disabled="busy" @click="saveLogBundle">
                        {{ formatMessage(messages.saveLogs) }}
                    </Button>
                </div>
                <div class="flex gap-2">
                    <Button type="quiet" @click="modal?.hide()">
                        {{ formatMessage(commonMessages.cancelButton) }}
                    </Button>
                    <Button type="colored" color="brand" :disabled="busy || !canSubmit" @click="openIssue">
                        {{ formatMessage(messages.openGithub) }}
                    </Button>
                </div>
            </div>
        </div>
    </NewModal>
</template>
