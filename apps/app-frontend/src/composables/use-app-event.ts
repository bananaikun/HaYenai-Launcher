import { onUnmounted } from 'vue'

import { instance_listener } from '@/helpers/events.js'

export interface InstanceAppEvent {
    event: string
    instance_id?: string
}

export function useAppEvent(type: 'instance', callback: (event: InstanceAppEvent) => void) {
    if (type !== 'instance') return
    let unlisten: (() => void) | undefined
    let disposed = false
    void instance_listener(callback).then((dispose) => {
        // Unmounted before the subscription settled: dispose right away instead
        // of leaving a backend listener pointing at a dead component.
        if (disposed) {
            dispose()
            return
        }
        unlisten = dispose
    })
    onUnmounted(() => {
        disposed = true
        unlisten?.()
        unlisten = undefined
    })
}
