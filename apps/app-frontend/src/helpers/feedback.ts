/**
 * 反馈面板的数据装配：把「用户描述 + 可选的诊断信息」拼成一条 GitHub Issue。
 *
 * 走**预填链接**而不是应用内直调 API —— exe 里不能内置有 repo 权限的 token
 * （反编译即可拿到，等于把仓库交给别人）。代价是用户要在浏览器里登录 GitHub，
 * 换来的是仓库安全。
 *
 * 体积是硬约束：GitHub 的 new-issue 预填走 query，太长会被截断。所以正文按
 * 预算裁剪，超出的部分让用户用「复制诊断信息」贴成评论。
 */
import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'

import { BrandConfig } from '@/config'

export type FeedbackKind = 'crash' | 'bug' | 'suggestion'

/** 预填链接的字符预算。留足余量：正文还要 URL 编码，中文会膨胀 3 倍。 */
const ISSUE_URL_BUDGET = 5200
/** 日志最多带多少行（面板里可调，这里再夹一次）。 */
export const FEEDBACK_LOG_LINE_OPTIONS = [80, 200, 500] as const

export interface FeedbackLogTail {
    file: string | null
    text: string
    lines: number
    dropped: boolean
}

export interface FeedbackEnvironment {
    version: string
    os: string
    arch: string
    cpu: string
    memory: string
    gpu: string
    screen: string
    webview: string
}

/** 「最近一次报错」——由 store/error.js 在 showError 时记下。 */
export interface LastErrorInfo {
    message: string
    stack?: string
    context?: string
    at: number
}

const UNKNOWN = '未知'

function archOf(): string {
    const ua = navigator.userAgent
    if (/arm64|aarch64/i.test(ua)) return 'arm64'
    if (/WOW64|Win64|x64|x86_64/i.test(ua)) return 'x64'
    return 'x86'
}

function formatBytes(bytes: number | undefined): string {
    if (!bytes || !Number.isFinite(bytes)) return UNKNOWN
    return `${(bytes / 1024 ** 3).toFixed(1)} GB`
}

/**
 * 采集本机环境。任何一项取不到就写「未知」，不让整个面板因为一个 IPC 失败而挂掉。
 */
export async function collectEnvironment(): Promise<FeedbackEnvironment> {
    const [version, os] = await Promise.all([
        getVersion().catch(() => UNKNOWN),
        invoke<string>('plugin:utils|get_os').catch(() => UNKNOWN),
    ])

    let cpu = UNKNOWN
    let memory = UNKNOWN
    let gpu = UNKNOWN
    try {
        const stats = await invoke<{
            cpu?: number
            memoryTotal?: number
            memory_total?: number
            gpuName?: string | null
            gpu_name?: string | null
        }>('system_stats')
        const cores = navigator.hardwareConcurrency
        cpu = cores ? `${cores} 线程` : UNKNOWN
        memory = formatBytes(stats.memoryTotal ?? stats.memory_total)
        gpu = stats.gpuName ?? stats.gpu_name ?? UNKNOWN
    } catch {
        // system_stats 读 sysinfo 失败时不影响其余信息
    }

    return {
        version: `${BrandConfig.productName} ${version}`,
        os: `${os} ${navigator.platform || ''}`.trim(),
        arch: archOf(),
        cpu,
        memory,
        gpu,
        screen: `${window.screen.width}×${window.screen.height} @${window.devicePixelRatio}x`,
        webview: navigator.userAgent.replace(/^Mozilla\/5\.0\s*/, '').slice(0, 160),
    }
}

/** 取当前会话日志尾部（Rust 侧已按 info 过滤并脱敏）。 */
export async function fetchLogTail(lines: number): Promise<FeedbackLogTail> {
    try {
        return await invoke<FeedbackLogTail>('plugin:utils|feedback_log_tail', { lines })
    } catch {
        return { file: null, text: '', lines: 0, dropped: false }
    }
}

export function environmentBlock(env: FeedbackEnvironment): string {
    return [
        `- 版本：${env.version}`,
        `- 系统：${env.os}（${env.arch}）`,
        `- CPU：${env.cpu}`,
        `- 内存：${env.memory}`,
        `- 显卡：${env.gpu}`,
        `- 屏幕：${env.screen}`,
        `- 语言：${navigator.language}`,
        `- 运行时：${env.webview}`,
    ].join('\n')
}

export function lastErrorBlock(error: LastErrorInfo): string {
    const when = new Date(error.at).toLocaleString()
    const head = `时间：${when}\n上下文：${error.context || '—'}\n信息：${error.message}`
    return error.stack ? `${head}\n\n\`\`\`\n${error.stack.slice(0, 4000)}\n\`\`\`` : head
}

export interface IssueDraftInput {
    kind: FeedbackKind
    title: string
    description: string
    environment?: string | null
    log?: FeedbackLogTail | null
    error?: LastErrorInfo | null
}

const KIND_LABEL: Record<FeedbackKind, string> = {
    crash: '启动器崩溃',
    bug: '功能异常',
    suggestion: '功能建议',
}

/** 拼完整正文（不裁剪）。也用于「复制诊断信息」。 */
export function buildIssueBody(input: IssueDraftInput): string {
    const parts: string[] = []
    parts.push(`### 反馈类型\n${KIND_LABEL[input.kind]}`)
    parts.push(`### 描述\n${input.description.trim() || '_（未填写）_'}`)

    if (input.environment) {
        parts.push(
            `### 环境\n<details><summary>展开</summary>\n\n${input.environment}\n\n</details>`,
        )
    }
    if (input.error) {
        parts.push(
            `### 最近一次报错\n<details><summary>展开</summary>\n\n${lastErrorBlock(input.error)}\n\n</details>`,
        )
    }
    if (input.log?.text) {
        parts.push(
            `### 日志尾部（${input.log.lines} 行，已自动脱敏${
                input.log.dropped ? '，更早的内容已省略' : ''
            }）\n<details><summary>展开</summary>\n\n\`\`\`\n${input.log.text}\n\`\`\`\n\n</details>`,
        )
    }
    return parts.join('\n\n')
}

/** 编码后的 query 长度（`+` 与 `%20` 都算 1 字符，取编码后长度最准）。 */
function encodedLength(value: string): number {
    return encodeURIComponent(value).length
}

/**
 * 把正文压进预算：先砍日志行数（从最早的行开始丢），再砍日志块，最后砍细节块。
 * 返回的 `truncated` 告诉面板要不要提示「正文已精简，完整诊断请复制」。
 */
export function fitIssueBody(input: IssueDraftInput): {
    body: string
    truncated: boolean
} {
    const titleBudget = 900
    const budget = ISSUE_URL_BUDGET - titleBudget

    let body = buildIssueBody(input)
    if (encodedLength(body) <= budget) return { body, truncated: false }

    // 1) 日志按行减半，直到装得下（或只剩 20 行）
    const log = input.log
    if (log?.text) {
        const allLines = log.text.split('\n')
        for (let keep = Math.floor(allLines.length / 2); keep >= 20; keep = Math.floor(keep / 2)) {
            const candidate = {
                ...input,
                log: { ...log, text: allLines.slice(-keep).join('\n'), lines: keep, dropped: true },
            }
            body = buildIssueBody(candidate)
            if (encodedLength(body) <= budget) return { body, truncated: true }
        }
        // 2) 日志整块丢掉
        body = buildIssueBody({ ...input, log: null })
        if (encodedLength(body) <= budget) return { body, truncated: true }
    }

    // 3) 再丢报错堆栈细节
    body = buildIssueBody({ ...input, error: null })
    if (encodedLength(body) <= budget) return { body, truncated: true }

    // 4) 兜底：硬截断描述
    return { body: body.slice(0, budget), truncated: true }
}

/** 生成 GitHub new-issue 预填链接。 */
export function buildIssueUrl(input: IssueDraftInput): {
    url: string
    truncated: boolean
} {
    const { body, truncated } = fitIssueBody(input)
    const title = `[${KIND_LABEL[input.kind]}] ${input.title.trim() || '未命名反馈'}`
    const params = new URLSearchParams({
        title,
        body,
        labels: 'feedback',
    })
    return {
        url: `${BrandConfig.repositoryUrl}/issues/new?${params.toString()}`,
        truncated,
    }
}
