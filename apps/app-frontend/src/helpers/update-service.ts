/**
 * Bridge to the Rust update-service plugin.
 *
 * The service address is resolved in Rust (DNS SRV + a health probe); the webview
 * never contacts it, which is why no `http:` host is whitelisted for it. This
 * module only wraps the three commands and keeps a single shared snapshot that
 * components read.
 *
 * Subscription rules:
 * * `subscribe()` is ref-counted, so mounting several indicator instances (or
 *   remounting on navigation) registers exactly one Tauri listener.
 * * `unsubscribe()` removes the listener when the last consumer goes away.
 * * Nothing here throws: a missing plugin or a shape change degrades to
 *   `unknown` so the title bar still renders.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { type DeepReadonly, readonly, type Ref, ref } from 'vue'

import {
    normalizeStatus,
    statusChanged,
    UNKNOWN_STATUS,
    type UpdateServiceStatus,
} from './update-service-format'

export const UPDATE_SERVICE_STATUS_EVENT = 'update-service-status'

/** The plugin name is hyphenated: Tauri identifiers reject underscores. */
const PLUGIN = 'plugin:update-service'

const status = ref<UpdateServiceStatus>({ ...UNKNOWN_STATUS })

let listener: UnlistenFn | null = null
let listenerPending: Promise<void> | null = null
let subscribers = 0

/** The shared snapshot. Read-only: only this module writes to it. */
export function useUpdateServiceStatus(): DeepReadonly<Ref<UpdateServiceStatus>> {
    return readonly(status) as DeepReadonly<Ref<UpdateServiceStatus>>
}

function apply(next: UpdateServiceStatus): UpdateServiceStatus {
    if (statusChanged(status.value, next)) {
        status.value = next
    }
    return status.value
}

/** Reads the in-memory snapshot from Rust. No network request is triggered. */
export async function fetchUpdateServiceStatus(): Promise<UpdateServiceStatus> {
    try {
        return apply(normalizeStatus(await invoke(`${PLUGIN}|status`)))
    } catch (error) {
        console.warn('[update-service] status command unavailable', error)
        return status.value
    }
}

/**
 * Asks Rust for an immediate probe.
 *
 * The command returns the pre-probe snapshot; the result arrives later as a
 * `update-service-status` event, so the retry button never blocks on the network.
 */
export async function refreshUpdateService(): Promise<UpdateServiceStatus> {
    try {
        return apply(normalizeStatus(await invoke(`${PLUGIN}|refresh`)))
    } catch (error) {
        console.warn('[update-service] refresh command unavailable', error)
        return status.value
    }
}

/** Resolved base URL, or `null` when nothing has been resolved yet. */
export async function getUpdateServiceBaseUrl(): Promise<string | null> {
    try {
        const value = await invoke<string | null>(`${PLUGIN}|base_url`)
        return typeof value === 'string' && value.length > 0 ? value : null
    } catch (error) {
        console.warn('[update-service] base_url command unavailable', error)
        return null
    }
}

/**
 * Announcements from `{srv base}api/announcements`.
 *
 * Fetched by Rust on purpose: the resolved host:port is not in the webview's
 * `http:` capability list and must not be, so the webview cannot reach it
 * directly. `null` means "offline or nothing published" — callers render their
 * empty state and never see an error string.
 */
export async function fetchAnnouncements(): Promise<unknown[] | null> {
    try {
        const value = await invoke<unknown[] | null>(`${PLUGIN}|announcements`)
        return Array.isArray(value) ? value : null
    } catch (error) {
        console.warn('[update-service] announcements command unavailable', error)
        return null
    }
}

/**
 * The updater manifest from `{srv base}latest?channel=…`, or `null`.
 *
 * Used for the "latest release / beta" hint in the update settings; the actual
 * update check goes through the updater in Rust.
 */
export async function fetchLatestManifest(
    channel: 'release' | 'beta',
): Promise<Record<string, unknown> | null> {
    try {
        const value = await invoke<unknown>(`${PLUGIN}|latest`, { channel })
        return value && typeof value === 'object' ? (value as Record<string, unknown>) : null
    } catch (error) {
        console.warn('[update-service] latest command unavailable', error)
        return null
    }
}

/**
 * Subscribes to status events. Safe to call repeatedly: the underlying Tauri
 * listener is created once and shared by all callers.
 */
export async function subscribeUpdateServiceStatus(): Promise<void> {
    subscribers += 1
    if (listener || listenerPending) {
        await listenerPending
        return
    }

    listenerPending = (async () => {
        try {
            listener = await listen(UPDATE_SERVICE_STATUS_EVENT, (event) => {
                apply(normalizeStatus(event.payload))
            })
        } catch (error) {
            console.warn('[update-service] status event unavailable', error)
        } finally {
            listenerPending = null
        }
    })()

    await listenerPending
    // Pick up the state Rust already has, so the dot is not stuck on "unknown"
    // until the next sampling tick.
    await fetchUpdateServiceStatus()
}

/** Drops one subscription; the listener is removed with the last one. */
export function unsubscribeUpdateServiceStatus(): void {
    subscribers = Math.max(0, subscribers - 1)
    if (subscribers > 0) return
    listener?.()
    listener = null
}
