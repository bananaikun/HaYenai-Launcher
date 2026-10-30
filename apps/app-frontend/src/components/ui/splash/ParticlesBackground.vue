<script setup lang="ts">
/**
 * GPU particle field for the splash screen.
 *
 * Written straight against WebGL2 (no dependency): point sprites with additive
 * blending, optional proximity links, pointer parallax and a per-particle
 * twinkle phase. Motion only runs while the page is visible — a splash screen
 * must not burn a core while the user waits.
 *
 * Everything is best-effort: if WebGL2 is unavailable, shaders fail to build or
 * the user asked for reduced motion, the component emits `unsupported`/draws a
 * static frame and the caller keeps its gradient. Nothing here blocks the app
 * or throws into it.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'

const props = withDefaults(
    defineProps<{
        particleCount?: number
        particleColors?: string[]
        glow?: number
        twinkle?: number
        /** Drift bias: outward from the centre, or upward. */
        drift?: 'forward' | 'up'
        connections?: boolean
        hoverEffect?: 'parallax' | 'none'
        speed?: number
    }>(),
    {
        particleCount: 260,
        particleColors: () => ['#7aa2ff', '#9fd8ff', '#ffffff'],
        glow: 0.6,
        twinkle: 0.35,
        drift: 'forward',
        connections: true,
        hoverEffect: 'parallax',
        speed: 0.08,
    },
)

const emit = defineEmits<{ unsupported: []; ready: [] }>()

const canvas = ref<HTMLCanvasElement>()

/** Cap on line segments per frame; the CPU link pass stops here. */
const MAX_LINKS = 900
/** Two particles closer than this (clip space) get a link. */
const LINK_DISTANCE = 0.16
/** Clip-space offset applied per unit of normalised pointer position. */
const PARALLAX = 0.035
/** Floats per particle in the interleaved buffer. */
const FLOATS_PER_PARTICLE = 8

const POINT_VERTEX = `#version 300 es
precision mediump float;
in vec3 aPosition;
in vec3 aColor;
in float aAlpha;
in float aSize;
uniform vec2 uPointer;
out vec3 vColor;
out float vAlpha;
void main() {
  vColor = aColor;
  vAlpha = aAlpha;
  gl_Position = vec4(aPosition.xy + uPointer, 0.0, 1.0);
  gl_PointSize = aSize;
}`

const POINT_FRAGMENT = `#version 300 es
precision mediump float;
in vec3 vColor;
in float vAlpha;
uniform float uGlow;
out vec4 outColor;
void main() {
  // Soft radial sprite: a falloff reads as a glow without a texture upload.
  float distanceFromCenter = length(gl_PointCoord - vec2(0.5)) * 2.0;
  float falloff = smoothstep(1.0, 0.0, distanceFromCenter);
  float core = pow(falloff, 1.0 + (1.0 - uGlow) * 2.0);
  outColor = vec4(vColor * (0.6 + uGlow), core * vAlpha);
}`

const LINK_VERTEX = `#version 300 es
precision mediump float;
in vec2 aPosition;
in float aAlpha;
uniform vec2 uPointer;
out float vAlpha;
void main() {
  vAlpha = aAlpha;
  gl_Position = vec4(aPosition + uPointer, 0.0, 1.0);
}`

const LINK_FRAGMENT = `#version 300 es
precision mediump float;
in float vAlpha;
out vec4 outColor;
void main() {
  outColor = vec4(0.55, 0.72, 1.0, vAlpha);
}`

let gl: WebGL2RenderingContext | null = null
let points: WebGLProgram | null = null
let links: WebGLProgram | null = null
let pointBuffer: WebGLBuffer | null = null
let linkBuffer: WebGLBuffer | null = null
let pointUniforms: Record<string, WebGLUniformLocation | null> = {}
let linkUniforms: Record<string, WebGLUniformLocation | null> = {}
let particleVertexData: Float32Array = new Float32Array(0)
let linkVertexData: Float32Array = new Float32Array(0)
let positions: Float32Array = new Float32Array(0)
let velocities: Float32Array = new Float32Array(0)
let phases: Float32Array = new Float32Array(0)
let linkVertexCount = 0
let particleCount = 0
let raf = 0
let running = false
let disposed = false
let lastTimestamp = 0
let pageVisible = true
let pointerX = 0
let pointerY = 0
let targetPointerX = 0
let targetPointerY = 0

/** Compiles one shader stage; returns null and logs on failure. */
function compileStage(type: number, source: string): WebGLShader | null {
    if (!gl) return null
    const shader = gl.createShader(type)
    if (!shader) return null
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        console.warn('[particles] shader compile failed:', gl.getShaderInfoLog(shader))
        gl.deleteShader(shader)
        return null
    }
    return shader
}

/** Builds a linked program; returns null and logs on failure. */
function buildProgram(vertexSource: string, fragmentSource: string): WebGLProgram | null {
    if (!gl) return null
    const vertex = compileStage(gl.VERTEX_SHADER, vertexSource)
    const fragment = compileStage(gl.FRAGMENT_SHADER, fragmentSource)
    if (!vertex || !fragment) return null
    const program = gl.createProgram()
    if (!program) return null
    gl.attachShader(program, vertex)
    gl.attachShader(program, fragment)
    gl.linkProgram(program)
    gl.deleteShader(vertex)
    gl.deleteShader(fragment)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        console.warn('[particles] program link failed:', gl.getProgramInfoLog(program))
        gl.deleteProgram(program)
        return null
    }
    return program
}

/** #rrggbb (or #rgb) to linear-ish 0..1 components. */
function parseColor(value: string): [number, number, number] {
    const hex = value.replace('#', '')
    const expanded =
        hex.length === 3
            ? hex
                  .split('')
                  .map((part) => part + part)
                  .join('')
            : hex
    const numeric = Number.parseInt(expanded, 16)
    if (expanded.length !== 6 || !Number.isFinite(numeric)) return [1, 1, 1]
    return [((numeric >> 16) & 0xff) / 255, ((numeric >> 8) & 0xff) / 255, (numeric & 0xff) / 255]
}

/** Allocates particle state in clip space. */
function seedParticles() {
    particleCount = Math.max(1, Math.min(1500, Math.round(props.particleCount)))
    positions = new Float32Array(particleCount * 2)
    velocities = new Float32Array(particleCount * 2)
    phases = new Float32Array(particleCount)
    particleVertexData = new Float32Array(particleCount * FLOATS_PER_PARTICLE)
    linkVertexData = new Float32Array(MAX_LINKS * 2 * 3)

    const palette = (props.particleColors.length > 0 ? props.particleColors : ['#ffffff']).map(
        parseColor,
    )

    for (let index = 0; index < particleCount; index += 1) {
        // Uniform disc distribution: sqrt avoids a dense centre.
        const radius = Math.sqrt(Math.random())
        const angle = Math.random() * Math.PI * 2
        const x = Math.cos(angle) * radius
        const y = Math.sin(angle) * radius
        positions[index * 2] = x
        positions[index * 2 + 1] = y

        // Bands of speed and size give a sense of depth without a z buffer.
        const depth = Math.random()
        velocities[index * 2] = (Math.random() - 0.5) * 0.0007
        velocities[index * 2 + 1] =
            (props.drift === 'forward' ? y : 1) * 0.0003 * (0.4 + depth)
        phases[index] = Math.random() * Math.PI * 2

        const color = palette[Math.floor(Math.random() * palette.length) % palette.length]
        const base = index * FLOATS_PER_PARTICLE
        particleVertexData[base] = x
        particleVertexData[base + 1] = y
        particleVertexData[base + 2] = 0
        particleVertexData[base + 3] = color[0]
        particleVertexData[base + 4] = color[1]
        particleVertexData[base + 5] = color[2]
        particleVertexData[base + 6] = 0.3 + depth * 0.55 // alpha
        particleVertexData[base + 7] = 1.6 + depth * 3.4 // size in device pixels
    }
}

/** Advances positions; wrapping keeps the density constant. */
function stepParticles(deltaSeconds: number) {
    const step = deltaSeconds * props.speed * 60
    for (let index = 0; index < particleCount; index += 1) {
        const xIndex = index * 2
        const yIndex = xIndex + 1
        let x = positions[xIndex] + velocities[xIndex] * step
        let y = positions[yIndex] + velocities[yIndex] * step
        if (x > 1.1) x = -1.1
        else if (x < -1.1) x = 1.1
        if (y > 1.1) y = -1.1
        else if (y < -1.1) y = 1.1
        positions[xIndex] = x
        positions[yIndex] = y
    }
}

/** Rebuilds the interleaved vertex data and the link segments. */
function buildGeometry(elapsed: number) {
    const twinkleDepth = Math.max(0, Math.min(1, props.twinkle))

    for (let index = 0; index < particleCount; index += 1) {
        const base = index * FLOATS_PER_PARTICLE
        particleVertexData[base] = positions[index * 2]
        particleVertexData[base + 1] = positions[index * 2 + 1]
        const twinkle =
            1 -
            twinkleDepth +
            twinkleDepth * (0.5 + 0.5 * Math.sin(elapsed * 2.4 + phases[index]))
        // Alpha slot is pre-multiplied by the twinkle factor each frame.
        const depthAlpha = 0.3 + ((particleVertexData[base + 7] - 1.6) / 3.4) * 0.55
        particleVertexData[base + 6] = depthAlpha * twinkle
    }

    linkVertexCount = 0
    if (props.connections) {
        for (
            let left = 0;
            left < particleCount && linkVertexCount < MAX_LINKS * 2;
            left += 1
        ) {
            for (
                let right = left + 1;
                right < particleCount && linkVertexCount < MAX_LINKS * 2;
                right += 1
            ) {
                const dx = positions[left * 2] - positions[right * 2]
                const dy = positions[left * 2 + 1] - positions[right * 2 + 1]
                const distance = Math.hypot(dx, dy)
                if (distance > LINK_DISTANCE) continue
                const alpha = (1 - distance / LINK_DISTANCE) * 0.16
                const cursor = linkVertexCount * 3
                linkVertexData[cursor] = positions[left * 2]
                linkVertexData[cursor + 1] = positions[left * 2 + 1]
                linkVertexData[cursor + 2] = alpha
                linkVertexData[cursor + 3] = positions[right * 2]
                linkVertexData[cursor + 4] = positions[right * 2 + 1]
                linkVertexData[cursor + 5] = alpha
                linkVertexCount += 2
            }
        }
    }
}

/** Points one float attribute of a program at the bound buffer. */
function bindAttribute(
    program: WebGLProgram,
    name: string,
    size: number,
    stride: number,
    offset: number,
) {
    if (!gl) return
    const location = gl.getAttribLocation(program, name)
    if (location < 0) return
    gl.enableVertexAttribArray(location)
    gl.vertexAttribPointer(location, size, gl.FLOAT, false, stride, offset)
}

/** Uploads the current data and draws one frame. */
function draw(timestamp: number) {
    if (!gl || !points || !pointBuffer) return

    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight)
    gl.clearColor(0, 0, 0, 0)
    gl.clear(gl.COLOR_BUFFER_BIT)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE)

    gl.useProgram(points)
    gl.bindBuffer(gl.ARRAY_BUFFER, pointBuffer)
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, particleVertexData)
    const stride = FLOATS_PER_PARTICLE * 4
    bindAttribute(points, 'aPosition', 3, stride, 0)
    bindAttribute(points, 'aColor', 3, stride, 3 * 4)
    bindAttribute(points, 'aAlpha', 1, stride, 6 * 4)
    bindAttribute(points, 'aSize', 1, stride, 7 * 4)
    gl.uniform2f(pointUniforms.uPointer, pointerX * PARALLAX, pointerY * PARALLAX)
    gl.uniform1f(pointUniforms.uGlow, Math.max(0, Math.min(1, props.glow)))
    gl.drawArrays(gl.POINTS, 0, particleCount)

    if (props.connections && links && linkBuffer && linkVertexCount > 0) {
        gl.useProgram(links)
        gl.bindBuffer(gl.ARRAY_BUFFER, linkBuffer)
        gl.bufferSubData(gl.ARRAY_BUFFER, 0, linkVertexData)
        bindAttribute(links, 'aPosition', 2, 3 * 4, 0)
        bindAttribute(links, 'aAlpha', 1, 3 * 4, 2 * 4)
        gl.uniform2f(linkUniforms.uPointer, pointerX * PARALLAX, pointerY * PARALLAX)
        gl.drawArrays(gl.LINES, 0, linkVertexCount)
    }

    // Keep the last timestamp for the frame-time budget.
    lastTimestamp = timestamp
}

/** rAF loop: integrates, rebuilds geometry and draws. */
function frame(timestamp: number) {
    if (!running || disposed) return
    const delta = lastTimestamp === 0 ? 1 / 60 : Math.min(0.05, (timestamp - lastTimestamp) / 1000)
    pointerX += (targetPointerX - pointerX) * 0.06
    pointerY += (targetPointerY - pointerY) * 0.06
    stepParticles(delta)
    buildGeometry(timestamp / 1000)
    draw(timestamp)
    raf = requestAnimationFrame(frame)
}

function start() {
    if (running || disposed || !gl) return
    running = true
    lastTimestamp = 0
    raf = requestAnimationFrame(frame)
}

function stop() {
    running = false
    if (raf) cancelAnimationFrame(raf)
    raf = 0
}

function handleVisibilityChange() {
    pageVisible = document.visibilityState === 'visible'
    if (pageVisible) start()
    else stop()
}

function handlePointerMove(event: PointerEvent) {
    if (props.hoverEffect !== 'parallax') return
    targetPointerX = (event.clientX / window.innerWidth) * 2 - 1
    targetPointerY = -((event.clientY / window.innerHeight) * 2 - 1)
}

function resetPointer() {
    targetPointerX = 0
    targetPointerY = 0
}

/** Releases every GL object and the context itself. */
function teardown() {
    stop()
    document.removeEventListener('visibilitychange', handleVisibilityChange)
    window.removeEventListener('pointermove', handlePointerMove)
    window.removeEventListener('pointerleave', resetPointer)
    window.removeEventListener('blur', stop)
    window.removeEventListener('focus', start)
    if (!gl) return
    if (pointBuffer) gl.deleteBuffer(pointBuffer)
    if (linkBuffer) gl.deleteBuffer(linkBuffer)
    if (points) gl.deleteProgram(points)
    if (links) gl.deleteProgram(links)
    gl.getExtension('WEBGL_lose_context')?.loseContext()
    gl = null
}

onMounted(() => {
    const element = canvas.value
    if (!element) return

    const context = element.getContext('webgl2', {
        alpha: true,
        antialias: false,
        depth: false,
        stencil: false,
        // A software rasteriser would cost more than the decoration is worth.
        failIfMajorPerformanceCaveat: true,
        powerPreference: 'low-power',
    })
    if (!context) {
        emit('unsupported')
        return
    }
    gl = context

    points = buildProgram(POINT_VERTEX, POINT_FRAGMENT)
    links = buildProgram(LINK_VERTEX, LINK_FRAGMENT)
    if (!points) {
        teardown()
        emit('unsupported')
        return
    }
    pointUniforms = {
        uPointer: gl.getUniformLocation(points, 'uPointer'),
        uGlow: gl.getUniformLocation(points, 'uGlow'),
    }
    linkUniforms = links ? { uPointer: gl.getUniformLocation(links, 'uPointer') } : {}

    seedParticles()
    pointBuffer = gl.createBuffer()
    linkBuffer = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, pointBuffer)
    gl.bufferData(gl.ARRAY_BUFFER, particleVertexData.byteLength, gl.DYNAMIC_DRAW)
    gl.bindBuffer(gl.ARRAY_BUFFER, linkBuffer)
    gl.bufferData(gl.ARRAY_BUFFER, linkVertexData.byteLength, gl.DYNAMIC_DRAW)

    document.addEventListener('visibilitychange', handleVisibilityChange)
    window.addEventListener('pointermove', handlePointerMove, { passive: true })
    window.addEventListener('pointerleave', resetPointer)
    // Minimising on Windows hides the document, but a blur without a visibility
    // change also stops being useful; both paths stop the loop.
    window.addEventListener('blur', stop)
    window.addEventListener('focus', start)

    buildGeometry(0)
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
        // One static frame, no animation loop.
        draw(performance.now())
    } else {
        start()
    }

    emit('ready')
})

onBeforeUnmount(() => {
    disposed = true
    teardown()
})
</script>

<template>
    <canvas ref="canvas" class="particles-canvas" aria-hidden="true" />
</template>

<style scoped>
.particles-canvas {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
}
</style>
