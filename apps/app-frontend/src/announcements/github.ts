/**
 * 「更新公告」的数据源：本仓库的 GitHub Releases（发版说明就是公告）。
 *
 * 原来的 `announcements/catalog.ts` 是**上游 Axolotl 的更新日志**（1.10.0-beta.1、
 * 1.9.8…那一串），已经跟这个启动器没有关系了。主人 2026-10-31 要求：
 * 移除原启动器的更新公告，换成自己的。
 *
 * 与 `helpers/github-releases.ts` 的分工：那边产出的是「远程公告中心」要的
 * `{id,title,body,level,publishedAt}`；这里产出的是「更新公告面板」要的
 * `{version, publishedAt, name, body, url}` —— 两个面板形状不同，别硬塞一份。
 *
 * 语义：草稿不出现；预发布照常出现（它是真实发过的包）；没有正文的版本跳过
 * （空折叠面板没有意义）。
 */
const RELEASES_API =
    'https://api.github.com/repos/bananaikun/HaYenai-Launcher/releases?per_page=20'

export type ReleaseAnnouncement = {
    /** 归一化后的版本号（去掉开头的 v）。 */
    version: string
    /** `YYYY-MM-DD`，用于列表右侧的日期。 */
    publishedAt: string
    /** release 的 name，缺省用 tag。 */
    name: string
    /** Markdown 正文。 */
    body: string
    /** release 页面地址，用于「查看完整更新日志」。 */
    url: string
    prerelease: boolean
}

type ReleaseLike = {
    tag_name?: unknown
    name?: unknown
    body?: unknown
    published_at?: unknown
    created_at?: unknown
    html_url?: unknown
    draft?: unknown
    prerelease?: unknown
}

let cache: ReleaseAnnouncement[] | null = null
let inflight: Promise<ReleaseAnnouncement[]> | null = null

const asString = (value: unknown): string => (typeof value === 'string' ? value : '')

function normalize(entry: ReleaseLike): ReleaseAnnouncement | null {
    const tag = asString(entry.tag_name).trim()
    if (!tag) return null
    const version = tag.replace(/^v/i, '')
    const body = asString(entry.body).trim()
    if (!version || !body) return null
    const stamp = asString(entry.published_at) || asString(entry.created_at)
    const parsed = Date.parse(stamp)
    return {
        version,
        publishedAt: Number.isFinite(parsed) ? new Date(parsed).toISOString().slice(0, 10) : '',
        name: asString(entry.name).trim() || `HaYenai ${version}`,
        body,
        url: asString(entry.html_url),
        prerelease: entry.prerelease === true,
    }
}

/** 拉一次并缓存（公告是静态内容，一次会话拉一次足够）。失败返回空数组，不抛错。 */
export function fetchReleaseAnnouncements(): Promise<ReleaseAnnouncement[]> {
    if (cache) return Promise.resolve(cache)
    if (inflight) return inflight
    inflight = (async () => {
        try {
            const response = await fetch(RELEASES_API, {
                headers: { Accept: 'application/vnd.github+json' },
            })
            if (!response.ok) return []
            const payload: unknown = await response.json()
            if (!Array.isArray(payload)) return []
            const list: ReleaseAnnouncement[] = []
            for (const entry of payload as ReleaseLike[]) {
                if (!entry || typeof entry !== 'object' || entry.draft === true) continue
                const item = normalize(entry)
                if (item) list.push(item)
            }
            cache = list
            return list
        } catch {
            return []
        } finally {
            inflight = null
        }
    })()
    return inflight
}
