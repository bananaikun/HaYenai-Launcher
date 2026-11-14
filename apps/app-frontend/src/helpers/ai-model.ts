/**
 * 「用哪个模型」是**全局**设置，不属于任何人格方案。
 *
 * 2026-11-14 主人重新定义：人格方案只保存「性格 / 人设 / 规则」，
 * 它是「这个 AI 说话什么调调」，不是「用哪个厂商哪个模型」。
 * 所以模型单独存在这里，供 Agent 页、房间机器人、后台 AI 共用一份。
 */

const STORAGE_KEY = 'hayenai.ai.model'

export interface ModelRef {
    providerId: string
    modelId: string
}

export function readGlobalModel(): ModelRef | null {
    try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return null
        const parsed = JSON.parse(raw) as Partial<ModelRef>
        if (!parsed.providerId || !parsed.modelId) return null
        return { providerId: parsed.providerId, modelId: parsed.modelId }
    } catch {
        return null
    }
}

export function writeGlobalModel(ref: ModelRef | null): void {
    if (!ref?.providerId || !ref.modelId) {
        localStorage.removeItem(STORAGE_KEY)
        return
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(ref))
}

/** `providerId::modelId` —— Combobox 的取值形态。 */
export function encodeModel(ref: ModelRef | null): string {
    return ref ? `${ref.providerId}::${ref.modelId}` : ''
}

export function decodeModel(value: string): ModelRef | null {
    const [providerId = '', modelId = ''] = (value ?? '').split('::')
    return providerId && modelId ? { providerId, modelId } : null
}

interface ProviderLike {
    providerId: string
    label: string
    models: { id: string; name?: string; enabled?: boolean }[]
}

/**
 * 把已添加的模型摊平成一张表：**按厂商名 A–Z，同厂商内按模型名 A–Z**。
 *
 * 不用分两级下拉（先厂商再模型）是因为主人明确要「一个下拉里就能选到」；
 * 分组只体现在标签的 `厂商 · 模型` 前缀和排序上。
 */
export function buildModelOptions(providers: ProviderLike[]): { value: string; label: string }[] {
    const collator = new Intl.Collator('zh-Hans-CN', { numeric: true, sensitivity: 'base' })
    return providers
        .flatMap((provider) =>
            provider.models
                .filter((model) => model.enabled !== false)
                .map((model) => ({
                    provider: provider.label,
                    name: model.name || model.id,
                    value: `${provider.providerId}::${model.id}`,
                })),
        )
        .sort(
            (a, b) => collator.compare(a.provider, b.provider) || collator.compare(a.name, b.name),
        )
        .map(({ provider, name, value }) => ({ value, label: `${provider} · ${name}` }))
}
