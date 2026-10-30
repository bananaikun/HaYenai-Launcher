<script setup lang="ts">
/**
 * Pixel-grid reveal used to swap the splash for the app.
 *
 * A full-screen grid of tiles shrinks away from the centre outwards with a
 * spring, uncovering whatever is already mounted underneath. It is deliberately
 * a *cover* that disappears rather than a two-content cross-fader: the app is
 * live beneath it, so the swap can never show an empty frame, and the app is
 * never remounted just to animate it.
 *
 * Falls back to a 150 ms opacity fade when motion is reduced or WebGL-free
 * browsers are in play — the reveal is decoration, not structure.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'

const props = withDefaults(
    defineProps<{
        /** Column/row count of the grid is derived from `pixelSize`. */
        pixelSize?: number
        gap?: number
        pixelRadius?: number
        /** Starting scale of every tile. */
        pixelScale?: number
        /** How long one tile takes to shrink, in ms. */
        pixelDuration?: number
        /** Total reveal budget, in ms. */
        duration?: number
        /** 0 = linear, 1 = very bouncy. */
        bounce?: number
        pattern?: 'center' | 'random' | 'sweep'
    }>(),
    {
        pixelSize: 56,
        gap: 2,
        pixelRadius: 4,
        pixelScale: 0.15,
        pixelDuration: 380,
        duration: 420,
        bounce: 0.35,
        pattern: 'center',
    },
)

const emit = defineEmits<{ done: [] }>()

const layer = ref<HTMLElement>()
const reducedMotion = ref(false)
const tiles = ref<{ id: number; x: number; y: number; size: number; delay: number; order: number }[]>(
    [],
)

let raf = 0
let startedAt = 0
let finished = false

/** Builds the tile grid for the current viewport. */
function buildGrid() {
    const width = window.innerWidth
    const height = window.innerHeight
    const size = Math.max(16, props.pixelSize)
    const columns = Math.ceil(width / size)
    const rows = Math.ceil(height / size)
    const centerX = (columns - 1) / 2
    const centerY = (rows - 1) / 2
    const maxDistance = Math.hypot(centerX, centerY) || 1

    const next: typeof tiles.value = []
    let order = 0
    for (let row = 0; row < rows; row += 1) {
        for (let column = 0; column < columns; column += 1) {
            const distance = Math.hypot(column - centerX, row - centerY) / maxDistance
            let delayRatio: number
            if (props.pattern === 'random') delayRatio = Math.random()
            else if (props.pattern === 'sweep') delayRatio = column / Math.max(1, columns - 1)
            else delayRatio = distance

            next.push({
                id: row * columns + column,
                x: column * size,
                y: row * size,
                size,
                // Stagger: the last tile starts once the budget minus one tile's
                // duration has elapsed, so the whole reveal fits `duration`.
                delay: Math.max(0, props.duration - props.pixelDuration) * delayRatio,
                order: order++,
            })
        }
    }
    tiles.value = next
}

/** Spring-ish ease-out with overshoot controlled by `bounce`. */
function easeOutBack(progress: number): number {
    const overshoot = 1 + props.bounce * 2.2
    const p = progress - 1
    return 1 + p * p * ((overshoot + 1) * p + overshoot)
}

/** Runs the reveal; resolves by emitting `done` once every tile is gone. */
function run(timestamp: number) {
    if (finished) return
    if (startedAt === 0) startedAt = timestamp
    const elapsed = timestamp - startedAt

    const nodes = layer.value?.children
    if (nodes) {
        for (let index = 0; index < nodes.length; index += 1) {
            const tile = tiles.value[index]
            const element = nodes[index] as HTMLElement
            if (!tile) continue
            const local = Math.min(1, Math.max(0, (elapsed - tile.delay) / props.pixelDuration))
            // Shrink from `pixelScale` to 0: the tile disappears into its centre.
            const scale = props.pixelScale * (1 - easeOutBack(local))
            element.style.transform = `scale(${Math.max(0, scale)})`
            element.style.opacity = local >= 1 ? '0' : '1'
        }
    }

    if (elapsed >= props.duration) {
        finished = true
        emit('done')
        return
    }
    raf = requestAnimationFrame(run)
}

onMounted(() => {
    reducedMotion.value = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reducedMotion.value) {
        // Fade-out only; no grid, no rAF.
        window.setTimeout(() => emit('done'), 150)
        return
    }
    buildGrid()
    raf = requestAnimationFrame(run)
})

onBeforeUnmount(() => {
    if (raf) cancelAnimationFrame(raf)
})
</script>

<template>
    <div
        v-if="!reducedMotion"
        ref="layer"
        class="pixel-swap"
        :class="{ 'pixel-swap--fading': reducedMotion }"
        aria-hidden="true"
    >
        <span
            v-for="tile in tiles"
            :key="tile.id"
            class="pixel-swap__tile"
            :style="{
                left: `${tile.x}px`,
                top: `${tile.y}px`,
                width: `${tile.size - gap}px`,
                height: `${tile.size - gap}px`,
                borderRadius: `${pixelRadius}px`,
                transform: `scale(${pixelScale})`,
            }"
        />
    </div>
    <div v-else class="pixel-swap-fade" aria-hidden="true" />
</template>

<style scoped>
.pixel-swap {
    position: fixed;
    inset: 0;
    z-index: 10002;
    overflow: hidden;
    pointer-events: none;
    /* No background here on purpose. The container used to paint an opaque
       plane, which stayed on top of the app while the tiles shrank away — the
       reveal was there, it was just hidden behind this element. The tiles are
       the mask; the container only positions them. */
    background: transparent;
}

.pixel-swap__tile {
    position: absolute;
    display: block;
    /* Same base colour as `SplashScreen`'s backdrop (`--surface-1` under its
       two radial gradients), so the swap does not flash a differently-toned
       plane on frame one. */
    background: var(--surface-1, #14141a);
    will-change: transform, opacity;
    transform-origin: center;
}

.pixel-swap-fade {
    position: fixed;
    inset: 0;
    z-index: 10002;
    pointer-events: none;
    background: var(--surface-1, #14141a);
    animation: pixel-swap-fade-out 150ms ease-out forwards;
}

@keyframes pixel-swap-fade-out {
    from {
        opacity: 1;
    }
    to {
        opacity: 0;
    }
}
</style>
