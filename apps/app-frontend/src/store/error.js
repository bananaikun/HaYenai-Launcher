import { defineStore } from 'pinia'

import { findMinecraftAuthError } from '@/components/ui/minecraft-auth-error-modal/minecraft-auth-errors'
import { isOfflineAccountRestrictedError } from '@/composables/useAntiPiracyStatus'

export const useError = defineStore('errorsStore', {
    state: () => ({
        errorModal: null,
        minecraftAuthErrorModal: null,
        minecraftLaunchErrorHandler: null,
        antiPiracyNoticeModal: null,
        // 反馈面板的「最近一次报错」：只要弹过一次错就记下来，用户不必复现。
        lastError: null,
    }),
    actions: {
        setErrorModal(ref) {
            this.errorModal = ref
        },
        setMinecraftAuthErrorModal(ref) {
            this.minecraftAuthErrorModal = ref
        },
        setAntiPiracyNoticeModal(ref) {
            this.antiPiracyNoticeModal = ref
        },
        showAntiPiracyNotice() {
            this.antiPiracyNoticeModal?.show()
        },
        setMinecraftLaunchErrorHandler(handler) {
            this.minecraftLaunchErrorHandler = handler
        },
        /** 留一份报错快照给反馈面板用（ErrorModal 关掉之后就查不到了）。 */
        rememberError(error, context) {
            const message =
                typeof error === 'string' ? error : (error?.message ?? String(error ?? ''))
            if (!message) return
            this.lastError = {
                message: String(message).slice(0, 4000),
                stack: typeof error?.stack === 'string' ? error.stack.slice(0, 8000) : undefined,
                context: context ? String(context).slice(0, 400) : undefined,
                at: Date.now(),
            }
        },
        showError(error, context, closable = true, source = null) {
            this.rememberError(error, context)
            if (isOfflineAccountRestrictedError(error)) {
                this.showAntiPiracyNotice()
                return
            }
            if (this.minecraftLaunchErrorHandler?.(error, context)) return
            if (
                error.message &&
                (error.message.includes('Minecraft authentication error:') ||
                    findMinecraftAuthError(error.message)) &&
                this.minecraftAuthErrorModal
            ) {
                this.minecraftAuthErrorModal.show(error)
                return
            }
            this.errorModal.show(error, context, closable, source)
        },
    },
})

export const handleSevereError = (err, context) => {
    const error = useError()
    error.showError(err, context)
    console.error(err)
}
