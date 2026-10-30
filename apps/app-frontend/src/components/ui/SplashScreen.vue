<script setup lang="ts">
/**
 * Cold-start screen: particle field, rounded app icon, progress line.
 *
 * The component does not remove itself. It reports `ready` once the minimum
 * display time has elapsed and the loading state is idle; `App.vue` then runs the
 * pixel-grid reveal, which is the only thing that unmounts this canvas. That
 * keeps the animation loop's lifetime tied to a single owner.
 */
import { defineMessages, injectLoadingState, useVIntl } from '@modrinth/ui'
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import appIcon from '@/assets/hayenai.png'
import { prefersReduced } from '@/components/ui/splash/aeroCore'
import ParticlesBackground from '@/components/ui/splash/ParticlesBackground.vue'
import { loading_listener } from '@/helpers/events.js'

const emit = defineEmits<{ ready: []; 'gpu-ready': [] }>()

/**
 * Loaded lazily: the WebGPU renderer plus its shader module is a few hundred
 * kilobytes, and the splash must not delay the first paint with it. The static
 * gradient below is painted immediately, so a slow or failed chunk load shows a
 * background instead of a blank window.
 */
const AeroShards = defineAsyncComponent({
    loader: () => import('@/components/ui/splash/AeroShards.vue'),
    // A missing chunk is just another reason the GPU path is unavailable; the
    // splash keeps running on the 2D particles instead of showing nothing.
    onError: (error, _retry, fail) => {
        degrade(error instanceof Error ? error : new Error(String(error)))
        fail()
    },
})

/** Never flash the screen for less than this, even on a warm start. */
const MIN_DISPLAY_MS = 1000

/**
 * Background renderer for the splash.
 *
 * `gpu` is the intended path (AeroShards, WebGPU). It falls back to `particles`
 * (WebGL2) and then to `static`, and each step only ever happens once: a
 * recovered GPU must not yank the background back mid-splash, which would read
 * as a flicker.
 */
type SplashRenderer = 'gpu' | 'particles' | 'static'

const progress = ref(0)
const message = ref<string>()
const mountedAt = Date.now()
let readyEmitted = false

/** Start optimistic: AeroShards reports back if WebGPU is unusable. */
const renderer = ref<SplashRenderer>(
    prefersReduced()
        ? 'static'
        : typeof navigator !== 'undefined' && 'gpu' in navigator
          ? 'gpu'
          : 'particles',
)

/**
 * QA switch: `?splash=fallback` forces the particles path and `?splash=static`
 * the gradient, so the degradation chain can be checked without unplugging a GPU
 * or editing code.
 */
const forcedRenderer = (() => {
    if (typeof location === 'undefined') return null
    const value = new URLSearchParams(location.search).get('splash')
    if (value === 'fallback' || value === 'particles') return 'particles' as const
    if (value === 'static') return 'static' as const
    return null
})()
if (forcedRenderer) renderer.value = forcedRenderer

const showGpu = computed(() => renderer.value === 'gpu')

/**
 * `gpu-ready` fires exactly once, from whichever path puts pixels on screen
 * first:
 *
 * * AeroShards' `ready` (device up, first frame scheduled);
 * * `degrade()` — a fallback renderer is about to take over, so the parent must
 *   not wait for a GPU that will never answer;
 * * component mount — when the very first decision is `static` (reduced motion
 *   or `?splash=static`), nothing else will ever emit.
 */
let gpuReadyEmitted = false
function emitGpuReady() {
    if (gpuReadyEmitted) return
    gpuReadyEmitted = true
    emit('gpu-ready')
}

/** Single, one-way degradation: gpu → particles → static. */
function degrade(error?: Error) {
    if (error) console.warn('[splash] falling back from WebGPU:', error.message)
    if (renderer.value === 'gpu') {
        renderer.value = 'particles'
    } else if (renderer.value === 'particles') {
        renderer.value = 'static'
    }
    // The fallback is the thing that will paint; release the gate now rather
    // than waiting for a ready signal the failed renderer will never send.
    emitGpuReady()
}

const loading = injectLoadingState()
const { formatMessage } = useVIntl()

const messages = defineMessages({
    updatingAppDirectory: {
        id: 'app.splash.updating-app-directory',
        defaultMessage: 'Updating app directory...',
    },
    checkingForUpdates: {
        id: 'app.splash.checking-for-updates',
        defaultMessage: 'Checking for updates...',
    },
    preparing: {
        id: 'app.splash.preparing',
        defaultMessage: 'Preparing your launcher...',
    },
})

/** Emits `ready` once, when both the timer and the loader allow it. */
function emitReady() {
    if (readyEmitted) return
    readyEmitted = true
    loading.setEnabled(true)
    emit('ready')
}

function scheduleReady() {
    const elapsed = Date.now() - mountedAt
    const delay = Math.max(0, MIN_DISPLAY_MS - elapsed)
    window.setTimeout(() => {
        if (!loading.pending.value) emitReady()
    }, delay)
}

/** Creeping progress while the app has no real fraction to report. */
function creep() {
    if (progress.value >= 95 || readyEmitted) return
    window.setTimeout(() => {
        progress.value += 2
        creep()
    }, 24)
}

watch(
    [loading.barEnabled, loading.pending],
    ([barEnabled, pending]) => {
        if (barEnabled) return
        if (pending) {
            progress.value = 0
            creep()
            return
        }
        scheduleReady()
    },
    { immediate: true },
)

onMounted(() => {
    // A static background never reports ready — it is not animated. Everything
    // else reports from the component that actually paints.
    if (renderer.value === 'static') emitGpuReady()
})

onBeforeUnmount(() => {
    // The reveal covered us; nothing left to time out.
    readyEmitted = true
})

loading_listener((event) => {
    if (event.event.type === 'directory_move') {
        progress.value = 100 * (event.fraction ?? 1)
        message.value = formatMessage(messages.updatingAppDirectory)
    } else if (event.event.type === 'checking_for_updates') {
        progress.value = 100 * (event.fraction ?? 1)
        message.value = formatMessage(messages.checkingForUpdates)
    }
})
</script>

<template>
    <div class="splash" data-tauri-drag-region>
        <!--
			AeroShards (WebGPU) is the intended background; a failed adapter, a
			failed device request or a runtime GPU error degrades to the WebGL2
			particles once, and a failed WebGL2 context to the static gradient.
		-->
        <AeroShards
            v-if="showGpu"
            :shard-color="'#7aa2ff'"
            :accent-color="'#9fd8ff'"
            :count="900"
            :speed="0.8"
            :glow="0.35"
            :depth-of-field="0.75"
            markers="plus"
            :mouse-interaction="true"
            :pulse-on-click="true"
            :hold-to-gather="true"
            :parallax="0.5"
            :dpr="2"
            @ready="emitGpuReady"
            @error="degrade"
        />
        <ParticlesBackground
            v-else-if="renderer === 'particles'"
            :particle-count="260"
            :particle-colors="['#7aa2ff', '#9fd8ff', '#ffffff']"
            :glow="0.6"
            :twinkle="0.35"
            drift="forward"
            :connections="true"
            hover-effect="parallax"
            :speed="0.08"
            @ready="emitGpuReady"
            @unsupported="degrade()"
        />

        <div class="splash__content" data-tauri-drag-region>
            <img
                :src="appIcon"
                alt="HaYenai"
                class="splash__icon"
                draggable="false"
                data-tauri-drag-region
            />
            <p class="splash__caption" data-tauri-drag-region>
                {{ message ?? formatMessage(messages.preparing) }}
            </p>
            <div class="splash__track" role="progressbar" :aria-valuenow="Math.round(progress)">
                <div class="splash__bar" :style="{ width: `${Math.min(100, progress)}%` }" />
            </div>
        </div>
    </div>
</template>

<style scoped>
.splash {
    position: fixed;
    inset: 0;
    z-index: 10001;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    /* Static gradient doubles as the fallback when WebGL2 is unavailable. */
    background:
        radial-gradient(120% 90% at 50% 40%, rgb(122 162 255 / 22%) 0%, transparent 62%),
        radial-gradient(100% 80% at 50% 100%, rgb(159 216 255 / 14%) 0%, transparent 70%),
        var(--surface-1, #14141a);
}

.splash__content {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 0 1.5rem;
}

.splash__icon {
    width: min(9rem, 30vh);
    height: min(9rem, 30vh);
    object-fit: contain;
    border-radius: 22%;
    /* A soft halo instead of a drop shadow keeps the icon flat, WE-style. */
    filter: drop-shadow(0 0 3rem rgb(122 162 255 / 35%));
    user-select: none;
    -webkit-user-drag: none;
    animation: splash-icon-in 520ms cubic-bezier(0.22, 1, 0.36, 1) both;
}

.splash__caption {
    margin: 0;
    font-size: 0.8125rem;
    letter-spacing: 0.02em;
    color: var(--color-text-tertiary, #9aa3b2);
}

.splash__track {
    width: min(14rem, 60vw);
    height: 2px;
    overflow: hidden;
    border-radius: 999px;
    background: rgb(255 255 255 / 10%);
}

.splash__bar {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #7aa2ff, #9fd8ff);
    transition: width 180ms ease-out;
}

@keyframes splash-icon-in {
    from {
        opacity: 0;
        transform: scale(0.96);
    }
    to {
        opacity: 1;
        transform: scale(1);
    }
}

@media (prefers-reduced-motion: reduce) {
    .splash__icon {
        animation: none;
    }

    .splash__bar {
        transition: none;
    }
}
</style>
