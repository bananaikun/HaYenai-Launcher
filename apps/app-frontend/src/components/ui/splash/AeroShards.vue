<script setup lang="ts">
/**
 * AeroShards: glass-shard field rendered with WebGPU.
 *
 * Thin Vue wrapper around `aeroShardsEngine.ts`, which holds the ported renderer.
 * Its job is the part React did implicitly: own the container element, translate
 * props into clamped `AeroSettings`, and give the caller exactly one failure
 * signal so the splash can fall back to the 2D particles exactly once.
 *
 * WebGPU can be present but unusable, so the preflight is explicit:
 * 1. no `navigator.gpu` at all (old WebView2 / disabled flag);
 * 2. `requestAdapter()` resolves to `null` (driver blocklist, hardware
 *    acceleration off, software-only adapter);
 * 3. `requestDevice()` fails inside the renderer's `init()` (missing limits or
 *    features) — reported through `onError` like any other GPU error.
 *
 * All three paths end in `emit('error')` and a stopped loop; nothing throws into
 * the app.
 */
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { clamp, MARKER_STYLES, MATERIALS, parseColor, PLACEMENTS, prefersReduced } from './aeroCore'
import { type AeroHandle, type AeroSettings, createAeroShards } from './aeroShardsEngine'

const props = withDefaults(
    defineProps<{
        shardColor?: string
        accentColor?: string
        backgroundColor?: string
        lightMode?: boolean
        placement?: 'full' | 'left' | 'right' | 'center'
        material?: 'glass' | 'chrome'
        count?: number
        shardSize?: number
        spread?: number
        twist?: number
        billow?: number
        speed?: number
        scale?: number
        depthOfField?: number
        glow?: number
        grain?: number
        vignette?: number
        intro?: boolean
        mouseInteraction?: boolean
        mouseStrength?: number
        markers?: 'plus' | 'dots' | 'none'
        pulseOnClick?: boolean
        holdToGather?: boolean
        parallax?: number
        paused?: boolean
        dpr?: number
        /** Skip the WebGPU preflight, for tests that stub the renderer. */
        skipPreflight?: boolean
    }>(),
    {
        shardColor: '#8a6bbd',
        accentColor: '#a855f7',
        backgroundColor: '',
        lightMode: false,
        placement: 'full',
        material: 'glass',
        count: 600,
        shardSize: 1,
        spread: 1,
        twist: 1,
        billow: 1,
        speed: 1,
        scale: 1,
        depthOfField: 0.75,
        glow: 0.35,
        grain: 0.06,
        vignette: 0.3,
        intro: true,
        mouseInteraction: true,
        mouseStrength: 0.6,
        markers: 'plus',
        pulseOnClick: true,
        holdToGather: true,
        parallax: 0.5,
        paused: false,
        dpr: 2,
        skipPreflight: false,
    },
)

const emit = defineEmits<{ ready: []; error: [error: Error] }>()

/**
 * The minimal `navigator.gpu` surface needed for the preflight.
 *
 * Declared locally so the launcher does not need `@webgpu/types` in its
 * `tsconfig.types`: only `requestAdapter` is used here, and the renderer's own
 * typings come from vgpu.
 */
type AdapterProbe = { destroy?: () => void }
type GpuNavigator = Navigator & {
    gpu?: {
        requestAdapter(options?: {
            powerPreference?: 'high-performance' | 'low-power'
        }): Promise<AdapterProbe | null>
    }
}

const host = ref<HTMLDivElement>()
let handle: AeroHandle | null = null
let disposed = false
/** Guards against a late preflight resolving after unmount. */
let failed = false

/** Props to the renderer's settings shape; mirrors the upstream clamping. */
function resolveSettings(): AeroSettings {
    return {
        shard: parseColor(props.shardColor, [0.54, 0.42, 0.74]) ?? [0.54, 0.42, 0.74],
        accent: parseColor(props.accentColor, [0.66, 0.33, 0.97]) ?? [0.66, 0.33, 0.97],
        paper: parseColor(props.backgroundColor, null),
        lightMode: props.lightMode,
        placement: Math.max(0, PLACEMENTS.indexOf(props.placement)),
        material: MATERIALS[props.material] ?? MATERIALS.glass,
        count: Math.round(clamp(props.count, 100, 6000)),
        shardSize: clamp(props.shardSize, 0.2, 4),
        spread: clamp(props.spread, 0, 3),
        twist: clamp(props.twist, 0, 3),
        billow: clamp(props.billow, 0, 3),
        speed: clamp(props.speed, 0, 4),
        scale: clamp(props.scale, 0.4, 2.5),
        depthOfField: clamp(props.depthOfField, 0, 1),
        glow: clamp(props.glow, 0, 2),
        grain: clamp(props.grain, 0, 0.5),
        vignette: clamp(props.vignette, 0, 1),
        intro: props.intro,
        mouseInteraction: props.mouseInteraction,
        mouseStrength: clamp(props.mouseStrength, 0, 1),
        markers: MARKER_STYLES[props.markers] ?? -1,
        pulseOnClick: props.pulseOnClick,
        holdToGather: props.holdToGather,
        parallax: clamp(props.parallax, 0, 1),
        paused: props.paused,
        dpr: Math.max(0.5, props.dpr),
    }
}

/** Reports the failure once and stops everything. */
function fail(error: Error) {
    if (failed || disposed) return
    failed = true
    handle?.dispose()
    handle = null
    console.warn('[aero-shards] WebGPU path unavailable:', error.message)
    emit('error', error)
}

/**
 * Proves a usable adapter exists before the renderer allocates anything.
 *
 * `navigator.gpu` being defined says nothing about the driver: a blocked or
 * software-only adapter resolves to `null` here, and that must fall back rather
 * than show an empty canvas.
 */
async function hasUsableAdapter(): Promise<boolean> {
    const gpu = typeof navigator === 'undefined' ? undefined : (navigator as GpuNavigator).gpu
    if (!gpu) return false
    try {
        const adapter = await gpu.requestAdapter({ powerPreference: 'high-performance' })
        if (adapter) {
            // Release the probe immediately: the renderer asks for its own.
            adapter.destroy?.()
            return true
        }
        return false
    } catch (error) {
        console.warn('[aero-shards] requestAdapter threw:', error)
        return false
    }
}

onMounted(async () => {
    if (prefersReduced()) {
        fail(new Error('Reduced motion is enabled'))
        return
    }
    const element = host.value
    if (!element) {
        fail(new Error('Missing container'))
        return
    }

    if (!props.skipPreflight && !(await hasUsableAdapter())) {
        if (disposed) return
        fail(new Error('WebGPU is not available'))
        return
    }
    if (disposed) return

    try {
        handle = createAeroShards(element, resolveSettings(), fail)
        emit('ready')
    } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)))
    }
})

// Settings are hot-applied: the renderer reads them per frame, so changing a
// slider must not rebuild the device.
watch(
    () => ({ ...props }),
    () => {
        if (!handle) return
        try {
            handle.update(resolveSettings())
        } catch (error) {
            fail(error instanceof Error ? error : new Error(String(error)))
        }
    },
    { deep: true },
)

onBeforeUnmount(() => {
    disposed = true
    handle?.dispose()
    handle = null
})
</script>

<template>
    <div ref="host" class="aero-shards" aria-hidden="true" />
</template>

<style scoped>
.aero-shards {
    position: absolute;
    inset: 0;
    overflow: hidden;
    /* The engine listens on `window`, so the layer itself must not swallow
       clicks meant for the splash content above it. */
    pointer-events: none;
}

/* The canvas is created by the engine; size it from here so the CSS owns layout. */
.aero-shards :deep(canvas) {
    display: block;
    width: 100%;
    height: 100%;
}
</style>
