export type RemoteAnnouncement = {
    id: string
    title: string
    summary: string | null
    content: string
    type: 'modal' | 'notification'
    priority: 'low' | 'normal' | 'high' | 'critical'
    starts_at: string
    ends_at: string | null
    published_at: string
    action_url: string | null
    action_label: string | null
}

export function safeAnnouncementUrl(value: unknown): string | null {
    if (typeof value !== 'string') return null
    try {
        const url = new URL(value)
        return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password
            ? url.href
            : null
    } catch {
        return null
    }
}

export function parseAnnouncements(value: unknown): RemoteAnnouncement[] | null {
    if (!Array.isArray(value) || value.length > 200) return null
    const items: RemoteAnnouncement[] = []
    const ids = new Set<string>()
    for (const entry of value) {
        if (!entry || typeof entry !== 'object') continue
        const item = entry as Record<string, unknown>
        if (
            typeof item.id !== 'string' ||
            item.id.length > 100 ||
            ids.has(item.id) ||
            typeof item.title !== 'string' ||
            !item.title.trim() ||
            item.title.length > 120 ||
            typeof item.content !== 'string' ||
            item.content.length > 20000 ||
            (item.summary != null &&
                (typeof item.summary !== 'string' || item.summary.length > 300)) ||
            !['modal', 'notification'].includes(String(item.type)) ||
            !['low', 'normal', 'high', 'critical'].includes(String(item.priority)) ||
            typeof item.starts_at !== 'string' ||
            !Number.isFinite(Date.parse(item.starts_at)) ||
            typeof item.published_at !== 'string' ||
            !Number.isFinite(Date.parse(item.published_at)) ||
            (item.ends_at != null &&
                (typeof item.ends_at !== 'string' || !Number.isFinite(Date.parse(item.ends_at)))) ||
            (item.action_label != null &&
                (typeof item.action_label !== 'string' || item.action_label.length > 80)) ||
            (item.action_url != null && !safeAnnouncementUrl(item.action_url))
        )
            continue
        ids.add(item.id)
        items.push({
            ...item,
            summary: item.summary ?? null,
            ends_at: item.ends_at ?? null,
            action_label: item.action_label ?? null,
            action_url: item.action_url ?? null,
        } as RemoteAnnouncement)
    }
    const rank = { low: 0, normal: 1, high: 2, critical: 3 }
    return items.sort(
        (first, second) =>
            rank[second.priority] - rank[first.priority] ||
            Date.parse(second.published_at) - Date.parse(first.published_at),
    )
}

export function isAnnouncementActive(item: RemoteAnnouncement, now = Date.now()) {
    return Date.parse(item.starts_at) <= now && (!item.ends_at || Date.parse(item.ends_at) > now)
}
export function announcementKey(item: RemoteAnnouncement) {
    return item.id + ':' + item.published_at
}

/**
 * Priorities the launcher's feed understands, keyed by the update service's
 * `level` vocabulary.
 */
const LEVEL_TO_PRIORITY = {
    info: 'normal',
    warn: 'high',
    critical: 'critical',
} as const

/**
 * `critical` announcements interrupt; everything else is a passive notification.
 */
const LEVEL_TO_TYPE = {
    info: 'notification',
    warn: 'notification',
    critical: 'modal',
} as const

/** Accepts a unix timestamp (seconds) or an ISO string; returns an ISO string. */
function toIsoTimestamp(value: unknown): string | null {
    if (typeof value === 'number' && Number.isFinite(value)) {
        return new Date(value * 1000).toISOString()
    }
    if (typeof value === 'string') {
        const parsed = Date.parse(value)
        return Number.isFinite(parsed) ? new Date(parsed).toISOString() : null
    }
    return null
}

/**
 * Converts the update service's announcement entries into the shape
 * {@link parseAnnouncements} validates.
 *
 * The service speaks `{id, title, body, level, publishedAt, dismissible}`; the
 * launcher's feed model predates it and uses `content` / `priority` / `type` /
 * `starts_at`. Keeping the translation in one tested function means the network
 * layer stays free of UI-shaped fields.
 *
 * Entries that cannot be mapped are dropped rather than defaulted: a malformed
 * announcement should not turn into an empty banner with a title.
 */
export function toAnnouncementEntries(value: unknown): unknown[] | null {
    if (!Array.isArray(value)) return null

    const mapped: unknown[] = []
    for (const entry of value) {
        if (!entry || typeof entry !== 'object') continue
        const item = entry as Record<string, unknown>
        const publishedAt = toIsoTimestamp(item.publishedAt)
        if (
            typeof item.id !== 'string' ||
            typeof item.title !== 'string' ||
            typeof item.body !== 'string' ||
            !publishedAt
        ) {
            continue
        }

        const level = item.level === 'warn' || item.level === 'critical' ? item.level : 'info'

        mapped.push({
            id: item.id,
            title: item.title,
            summary: null,
            content: item.body,
            type: LEVEL_TO_TYPE[level],
            priority: LEVEL_TO_PRIORITY[level],
            starts_at: publishedAt,
            ends_at: null,
            published_at: publishedAt,
            action_url: null,
            action_label: null,
        })
    }

    return mapped
}

export const REMOTE_ANNOUNCEMENTS_UPDATED_EVENT = 'axolotl:remote-announcements-updated'
export const OPEN_REMOTE_ANNOUNCEMENT_CENTER_EVENT = 'axolotl:open-remote-announcement-center'
