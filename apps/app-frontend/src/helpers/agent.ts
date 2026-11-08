import { Channel, invoke } from '@tauri-apps/api/core'

/** 与 Rust 侧 `ChatMessage` 一一对应。字段名不能改，serde 按名匹配。 */
export interface AgentMessage {
    role: 'system' | 'user' | 'assistant' | 'tool'
    content?: string
    tool_calls?: AgentToolCall[]
    tool_call_id?: string | null
    opaque?: unknown
}

export interface AgentToolCall {
    id: string
    name: string
    args: unknown
}

export interface AgentStreamEvent {
    /** `delta` = 正文增量；`tool` = 即将调用的工具名 */
    kind: string
    text: string
}

export interface AgentToolDefinition {
    type: 'function'
    function: { name: string; description: string; parameters: unknown }
}

export interface AgentTurnRequest {
    presetId?: string | null
    providerId?: string | null
    modelId?: string | null
    allowWrite: boolean
    messages: AgentMessage[]
}

/** 拿工具清单。只读/可写两档返回的东西不一样，UI 靠它显示"当前能做哪些事"。 */
export const agentTools = (allowWrite: boolean) =>
    invoke<AgentToolDefinition[]>('agent_tools', { allowWrite })

export const agentRunTool = (name: string, args: unknown, allowWrite: boolean) =>
    invoke<unknown>('agent_run_tool', { name, args, allowWrite })

export function agentTurn(
    request: AgentTurnRequest,
    onEvent?: (event: AgentStreamEvent) => void,
): Promise<AgentMessage> {
    if (!onEvent) {
        return invoke<AgentMessage>('agent_turn', { request })
    }
    const channel = new Channel<AgentStreamEvent>()
    channel.onmessage = onEvent
    return invoke<AgentMessage>('agent_turn', { request, onEvent: channel })
}

/** Tauri 的命令错误是 `{ field_name, message }` 对象，不是 `Error` —— 直接 String() 会得到 `[object Object]`。 */
export function agentErrorMessage(error: unknown): string {
    if (typeof error === 'string') return error
    if (error && typeof error === 'object') {
        const message = (error as { message?: unknown }).message
        if (typeof message === 'string' && message.trim()) return message
        const text = (error as { error?: unknown }).error
        if (typeof text === 'string' && text.trim()) return text
    }
    try {
        return JSON.stringify(error)
    } catch {
        return String(error)
    }
}
