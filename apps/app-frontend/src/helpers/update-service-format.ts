/**
 * Pure formatting/classification helpers for the update-service indicator.
 *
 * Kept in their own module so they can be unit tested under `node --test`
 * without a Tauri runtime.
 */

export type UpdateServiceState = 'unknown' | 'online' | 'offline'

/**
 * Why the last probe failed. Mirrors `theseus::update_service::ProbeFailure`.
 *
 * Only a category travels over IPC: the raw TLS/DNS message is unreadable in a
 * tooltip and would leak network details the user does not need.
 */
export type UpdateServiceFailure = 'certificate' | 'timeout' | 'unreachable'

export type UpdateServiceStatus = {
    state: UpdateServiceState
    rttMs: number | null
    baseUrl: string | null
    lastOkAt: number | null
    lastFailure: UpdateServiceFailure | null
}

export const UNKNOWN_STATUS: UpdateServiceStatus = Object.freeze({
    state: 'unknown',
    rttMs: null,
    baseUrl: null,
    lastOkAt: null,
    lastFailure: null,
})

const STATES: readonly UpdateServiceState[] = ['unknown', 'online', 'offline']
const FAILURES: readonly UpdateServiceFailure[] = ['certificate', 'timeout', 'unreachable']

function asState(value: unknown): UpdateServiceState {
    return STATES.includes(value as UpdateServiceState) ? (value as UpdateServiceState) : 'unknown'
}

function asFailure(value: unknown): UpdateServiceFailure | null {
    return FAILURES.includes(value as UpdateServiceFailure) ? (value as UpdateServiceFailure) : null
}

function asCount(value: unknown): number | null {
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null
}

function asUrl(value: unknown): string | null {
    if (typeof value !== 'string') return null
    const trimmed = value.trim()
    return trimmed.length > 0 ? trimmed : null
}

/**
 * Coerces whatever came back from the backend into a status object.
 *
 * A missing or malformed field becomes "unknown" rather than throwing: the
 * indicator is decoration, and a shape change must never take the title bar
 * down with it.
 */
export function normalizeStatus(payload: unknown): UpdateServiceStatus {
    if (!payload || typeof payload !== 'object') return { ...UNKNOWN_STATUS }
    const record = payload as Record<string, unknown>

    return {
        state: asState(record.state),
        rttMs: asCount(record.rttMs),
        baseUrl: asUrl(record.baseUrl),
        lastOkAt: asCount(record.lastOkAt),
        lastFailure: asFailure(record.lastFailure),
    }
}

/** Latency label: `123 ms`, or `--` when there is no sample yet. */
export function formatLatency(rttMs: number | null): string {
    if (rttMs === null) return '--'
    return `${Math.round(rttMs)} ms`
}

/**
 * Whether two snapshots differ enough to be worth re-rendering.
 *
 * Mirrors the backend's publish threshold so a late event does not repaint the
 * title bar for a couple of milliseconds of jitter.
 */
export function statusChanged(
    previous: UpdateServiceStatus,
    next: UpdateServiceStatus,
    thresholdMs = 5,
): boolean {
    if (previous.state !== next.state) return true
    if (previous.baseUrl !== next.baseUrl) return true
    if (previous.lastFailure !== next.lastFailure) return true
    if (previous.rttMs === null || next.rttMs === null) return previous.rttMs !== next.rttMs
    return Math.abs(previous.rttMs - next.rttMs) >= thresholdMs
}

/** `HH:MM:SS` in local time, or `--` without a timestamp. */
export function formatLastOk(lastOkAt: number | null, locale?: string): string {
    if (lastOkAt === null) return '--'
    const date = new Date(lastOkAt * 1000)
    if (Number.isNaN(date.getTime())) return '--'
    return date.toLocaleTimeString(locale, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
    })
}
