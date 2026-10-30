/**
 * 公告来源：本仓库的 GitHub Releases。
 *
 * 为什么不用更新服务器：那份 `/api/announcements` 是上游 Axolotl 的后台，我们
 * 已经没有它了；更新源既然换成 GitHub，发版说明（release notes）就是最自然的
 * 「作者本人的公告」。
 *
 * 契约与旧的 `fetchAnnouncements()` 一致：**永不抛错**，返回 `null` 表示
 * 「离线 / 没发过版 / 被限流」，调用方照常渲染空状态。返回的条目仍是更新服务那套
 * `{id,title,body,level,publishedAt}`，交给 `toAnnouncementEntries` 翻译，
 * 渲染层一行都不用改。
 *
 * 未鉴权的 GitHub API 是每小时 60 次/IP；调用方有 24 小时本地缓存，正常使用远达不到。
 */
const RELEASES_API =
    'https://api.github.com/repos/bananaikun/HaYenai-Launcher/releases?per_page=20'

type ReleaseLike = {
    id?: unknown
    tag_name?: unknown
    name?: unknown
    body?: unknown
    published_at?: unknown
    draft?: unknown
    prerelease?: unknown
}

export async function fetchGithubReleases(): Promise<unknown[] | null> {
    try {
        const response = await fetch(RELEASES_API, {
            headers: { Accept: 'application/vnd.github+json' },
        })
        if (!response.ok) return null

        const payload: unknown = await response.json()
        if (!Array.isArray(payload)) return null

        const mapped: unknown[] = []
        for (const entry of payload as ReleaseLike[]) {
            // 草稿不对外，预发布也别推给所有人
            if (!entry || typeof entry !== 'object') continue
            if (entry.draft === true || entry.prerelease === true) continue
            if (typeof entry.id !== 'number' && typeof entry.id !== 'string') continue

            const publishedAt =
                typeof entry.published_at === 'string' ? entry.published_at : null
            if (!publishedAt || !Number.isFinite(Date.parse(publishedAt))) continue

            const title =
                (typeof entry.name === 'string' && entry.name.trim()) ||
                (typeof entry.tag_name === 'string' ? entry.tag_name : '') ||
                `Release ${String(entry.id)}`
            const body = typeof entry.body === 'string' ? entry.body : ''
            if (!body.trim()) continue

            mapped.push({
                id: `gh-${String(entry.id)}`,
                title,
                body,
                level: 'info',
                publishedAt,
            })
        }

        return mapped
    } catch {
        // 离线 / DNS 失败 / JSON 坏掉都走这里：公告是锦上添花，不该冒泡到 UI
        return null
    }
}
