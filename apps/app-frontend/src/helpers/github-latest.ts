/**
 * 各更新通道的「最新版本号」，来源是本仓库的 GitHub Releases。
 *
 * 为什么不用 `fetchLatestManifest()`：那个走 Rust 的 update-service 插件，
 * 解析的是**旧 HaYenai 更新服务器**的 DNS SRV 地址。更新源换成 GitHub 之后
 * 那条链路已经不存在了，于是两个通道都恒显示「最新版本暂不可用」。
 *
 * 口径（与发布脚本 `make_latest` 的语义对齐）：
 * - `release`：最新的**正式** release（非 draft、非 prerelease）；
 * - `beta`：最新的**任意** release（含 prerelease）；没有预发布时就是正式版，
 *   因为对 beta 通道来说「能拿到的最新构建」本来就是它。
 *
 * 和 `fetchGithubReleases()` 一样：**永不抛错**，返回空对象表示拿不到。
 */
const RELEASES_API =
    'https://api.github.com/repos/bananaikun/HaYenai-Launcher/releases?per_page=20'

type ReleaseLike = {
    tag_name?: unknown
    draft?: unknown
    prerelease?: unknown
}

const cleanVersion = (value: unknown): string | null => {
    if (typeof value !== 'string') return null
    const trimmed = value.trim().replace(/^v/i, '')
    return trimmed.length ? trimmed : null
}

export async function fetchLatestChannelVersions(): Promise<
    Partial<Record<'release' | 'beta', string>>
> {
    try {
        const response = await fetch(RELEASES_API, {
            headers: { Accept: 'application/vnd.github+json' },
        })
        if (!response.ok) return {}
        const payload: unknown = await response.json()
        if (!Array.isArray(payload)) return {}

        let stable: string | null = null
        let newest: string | null = null
        for (const entry of payload as ReleaseLike[]) {
            if (!entry || typeof entry !== 'object' || entry.draft === true) continue
            const version = cleanVersion(entry.tag_name)
            if (!version) continue
            if (!newest) newest = version
            if (!stable && entry.prerelease !== true) stable = version
            if (stable && newest) break
        }

        const out: Partial<Record<'release' | 'beta', string>> = {}
        if (stable) out.release = stable
        if (newest) out.beta = newest
        return out
    } catch {
        return {}
    }
}
