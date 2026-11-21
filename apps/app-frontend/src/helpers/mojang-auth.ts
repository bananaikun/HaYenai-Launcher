import { invoke } from '@tauri-apps/api/core'

import { ensureFallenAuthProxyArgs, removeFallenAuthProxyArgs } from '@/helpers/java-arguments'
import { type AppSettings, get, update } from '@/helpers/settings'

function sameArgs(left: string[], right: string[]) {
    return left.length === right.length && left.every((arg, index) => arg === right[index])
}

export async function setMojangAuthUseMirror(useMirror: boolean) {
    await invoke('plugin:auth|set_mojang_auth_use_mirror', { useMirror, automatic: false })
}

/**
 * Authentication traffic only leaves Mojang's endpoints when the user picked the
 * third-party mirror explicitly. The automatic modes never fall back to it: a
 * silent fallback would hand a one-time login token to another host without the
 * user ever asking for that.
 */
export async function reconcileMojangAuthSource(settings: AppSettings): Promise<boolean> {
    const useMirror = settings.mojang_auth_source === 'mirror_preferred'

    await setMojangAuthUseMirror(useMirror)

    const nextArgs = useMirror
        ? ensureFallenAuthProxyArgs(settings.extra_launch_args)
        : removeFallenAuthProxyArgs(settings.extra_launch_args)

    if (sameArgs(nextArgs, settings.extra_launch_args)) return false

    settings.extra_launch_args = nextArgs
    return true
}

export async function reconcileMojangAuthSourceAtStartup(): Promise<void> {
    const settings = await get()
    if (await reconcileMojangAuthSource(settings)) {
        await update({ extra_launch_args: settings.extra_launch_args })
    }
}
