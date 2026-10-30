<script setup lang="ts">
import { defineMessages, useVIntl } from '@modrinth/ui'
import { computed } from 'vue'

import { refreshUpdateService, useUpdateServiceStatus } from '@/helpers/update-service'
import {
    formatLastOk,
    formatLatency,
    type UpdateServiceFailure,
} from '@/helpers/update-service-format'

/**
 * Title-bar indicator for the update service.
 *
 * Three visual states: green (online), grey (offline/unknown, breathing while a
 * probe is in flight) and a `--` latency placeholder until a sample exists. The
 * tooltip carries the base URL, the last success time and a failure *category*
 * — never a raw error string.
 *
 * The root node deliberately has no `data-tauri-drag-region`: the title bar is
 * `decorations: false` and self-drawn, so a child that claims the drag region
 * makes the whole bar undraggable. The parent supplies `pointer-events-auto`.
 *
 * This component registers no listener of its own: `App.vue` owns the single
 * subscription (started from `runWhenIdle`) and this reads the shared snapshot,
 * so navigating in and out of pages cannot stack Tauri listeners.
 */
const { formatMessage } = useVIntl()
const status = useUpdateServiceStatus()

const messages = defineMessages({
    checking: {
        id: 'app.update-service.state.checking',
        defaultMessage: 'Checking the update service',
    },
    online: {
        id: 'app.update-service.state.online',
        defaultMessage: 'Update service reachable',
    },
    offline: {
        id: 'app.update-service.state.offline',
        defaultMessage: 'Update service unreachable',
    },
    unknown: {
        id: 'app.update-service.state.unknown',
        defaultMessage: 'Update service status unknown',
    },
    address: {
        id: 'app.update-service.address',
        defaultMessage: 'Address',
    },
    lastSuccess: {
        id: 'app.update-service.last-success',
        defaultMessage: 'Last success',
    },
    latency: {
        id: 'app.update-service.latency',
        defaultMessage: 'Latency',
    },
    reason: {
        id: 'app.update-service.reason',
        defaultMessage: 'Reason',
    },
    failureCertificate: {
        id: 'app.update-service.failure.certificate',
        defaultMessage: 'Certificate not trusted',
    },
    failureTimeout: {
        id: 'app.update-service.failure.timeout',
        defaultMessage: 'Timed out',
    },
    failureUnreachable: {
        id: 'app.update-service.failure.unreachable',
        defaultMessage: 'Not reachable',
    },
    retry: {
        id: 'app.update-service.retry',
        defaultMessage: 'Check again',
    },
})

const failureLabels: Record<UpdateServiceFailure, () => string> = {
    certificate: () => formatMessage(messages.failureCertificate),
    timeout: () => formatMessage(messages.failureTimeout),
    unreachable: () => formatMessage(messages.failureUnreachable),
}

const stateLabel = computed(() => {
    switch (status.value.state) {
        case 'online':
            return formatMessage(messages.online)
        case 'offline':
            return formatMessage(messages.offline)
        case 'unknown':
            return formatMessage(messages.checking)
        default:
            return formatMessage(messages.unknown)
    }
})

const latencyLabel = computed(() => formatLatency(status.value.rttMs))
const lastOkLabel = computed(() => formatLastOk(status.value.lastOkAt))
const failureLabel = computed(() =>
    status.value.lastFailure ? failureLabels[status.value.lastFailure]() : null,
)

const dotClass = computed(() => {
    if (status.value.state === 'online') return 'bg-green'
    if (status.value.state === 'offline') return 'bg-[var(--color-text-tertiary)]'
    return 'bg-[var(--color-text-tertiary)] animate-pulse'
})

async function probeNow() {
    await refreshUpdateService()
}
</script>

<template>
    <button
        type="button"
        class="connection-status flex h-full shrink-0 cursor-pointer items-center gap-1.5 border-none bg-transparent px-2 text-[11px] leading-none text-[var(--color-text-default)] outline-none hover:brightness-110"
        :aria-label="`${stateLabel}. ${latencyLabel}`"
        @click.stop="probeNow"
    >
        <span class="size-2 shrink-0 rounded-full" :class="dotClass" aria-hidden="true" />
        <span class="tabular-nums">{{ latencyLabel }}</span>
        <span class="sr-only">{{ stateLabel }}</span>

        <span class="connection-status-tooltip" role="tooltip">
            <span class="block font-semibold text-[var(--color-text-primary)]">{{
                stateLabel
            }}</span>
            <span class="block"
                >{{ formatMessage(messages.address) }}: {{ status.baseUrl ?? '--' }}</span
            >
            <span class="block">{{ formatMessage(messages.lastSuccess) }}: {{ lastOkLabel }}</span>
            <span class="block">{{ formatMessage(messages.latency) }}: {{ latencyLabel }}</span>
            <span v-if="failureLabel" class="block"
                >{{ formatMessage(messages.reason) }}: {{ failureLabel }}</span
            >
            <span class="mt-1 block text-[var(--color-text-tertiary)]">{{
                formatMessage(messages.retry)
            }}</span>
        </span>
    </button>
</template>

<style scoped>
.connection-status {
    position: relative;
}

/* The tooltip is CSS-only so the indicator adds no listeners and no teleport
   target to the title bar. */
.connection-status-tooltip {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 60;
    display: none;
    width: max-content;
    max-width: 22rem;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-md, 0.5rem);
    background: var(--surface-5, #1f1f26);
    box-shadow: 0 8px 24px rgb(0 0 0 / 35%);
    text-align: left;
    font-size: 11px;
    line-height: 1.45;
    pointer-events: none;
}

.connection-status:hover .connection-status-tooltip,
.connection-status:focus-visible .connection-status-tooltip {
    display: block;
}
</style>
