/**
 * 生成 Wallpaper Engine 壁纸的**最小 fixture**，用来在没有真实工坊包时验证导入与
 * 渲染两条路径。
 *
 * 这台机器上没有装 Wallpaper Engine，也没有任何 `.mpkg` 样本，所以「装得进来」与
 * 「画得出来」这两件事必须能自己造输入：
 *
 * | 产物 | 验证什么 |
 * | --- | --- |
 * | `scene-loose/` | 松散工程形态：`project.json` + `scene.json`（零图层，只有清屏色） |
 * | `scene.mpkg` | `PKGM0019` 容器形态：同样三个条目打成一个包 |
 * | `video-loose/` | 视频形态的 `project.json`；`clip.mp4` 要自己放（见下） |
 *
 * 为什么「零图层场景」也有意义：它照样要走完 project.json → scene.json → 装配 →
 * 首帧那条链，画面上应该是一块纯色（`general.clearcolor`）。链路上任何一环断了
 * （协议 404、`file` 指错、贴图/着色器前置校验不过）都出不来首帧 —— 这正是
 * 我们要验的「渲染路径通不通」，而不是「壁纸好不好看」。
 *
 * 用法：
 *
 * ```bash
 * node scripts/wallpaper-fixture/make-fixture.mjs            # 输出到 tmp/wallpaper-fixture（tmp/ 已在 .gitignore 里）
 * node scripts/wallpaper-fixture/make-fixture.mjs --out DIR
 * node scripts/wallpaper-fixture/make-fixture.mjs --video C:/path/to/clip.mp4
 * ```
 *
 * 视频文件**不做合成**：一个合法的 mp4 需要 H.264 编码器，本仓库不引这个依赖。
 * `--video` 会把现成的 mp4 拷成 `video-loose/clip.mp4`；不给就只生成清单，
 * 让用户自己丢一个（任意 `.mp4` / `.webm`，几秒钟的就行）。
 */

import { createHash } from 'node:crypto'
import { copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { crc32, deflateSync } from 'node:zlib'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(HERE, '..', '..')

/** 场景逻辑分辨率。与 WE 默认工程一致，方便对照。 */
const SCENE_WIDTH = 1920
const SCENE_HEIGHT = 1080

/* ------------------------------------------------------------------ *
 * PNG：8×8 纯色，手搓一份带 CRC 的合法 PNG（不引图片库）
 * ------------------------------------------------------------------ */

/** 一个字节的 PNG chunk：长度 + 类型 + 数据 + CRC32(类型 + 数据)。 */
function pngChunk(type, data) {
    const length = Buffer.alloc(4)
    length.writeUInt32BE(data.length, 0)
    const typeBytes = Buffer.from(type, 'ascii')
    const payload = Buffer.concat([typeBytes, data])
    const crc = Buffer.alloc(4)
    crc.writeUInt32BE(crc32(payload) >>> 0, 0)
    return Buffer.concat([length, payload, crc])
}

/**
 * 造一张 `size × size` 的纯色 PNG。
 *
 * 每行是 `1 字节滤波类型(0) + width × 3 字节 RGB`，整块用 zlib 压成一个 IDAT。
 */
function solidPng(size, [red, green, blue]) {
    const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

    const ihdr = Buffer.alloc(13)
    ihdr.writeUInt32BE(size, 0)
    ihdr.writeUInt32BE(size, 4)
    ihdr.writeUInt8(8, 8) // 位深
    ihdr.writeUInt8(2, 9) // 颜色类型：真彩色 RGB
    ihdr.writeUInt8(0, 10) // 压缩方式
    ihdr.writeUInt8(0, 11) // 滤波方式
    ihdr.writeUInt8(0, 12) // 非隔行

    const stride = size * 3 + 1
    const raw = Buffer.alloc(stride * size)
    for (let row = 0; row < size; row += 1) {
        const offset = row * stride
        raw[offset] = 0
        for (let column = 0; column < size; column += 1) {
            const pixel = offset + 1 + column * 3
            raw[pixel] = red
            raw[pixel + 1] = green
            raw[pixel + 2] = blue
        }
    }

    return Buffer.concat([
        signature,
        pngChunk('IHDR', ihdr),
        pngChunk('IDAT', deflateSync(raw, { level: 9 })),
        pngChunk('IEND', Buffer.alloc(0)),
    ])
}

/* ------------------------------------------------------------------ *
 * 清单
 * ------------------------------------------------------------------ */

/** `project.json`：字段与 WE 工坊包一致，只保留启动器真正读的那几个。 */
function projectJson({ title, type, file, preview }) {
    return {
        description: `HaYenai fixture · ${title}`,
        file,
        general: {
            properties: {
                schemecolor: {
                    order: 0,
                    text: 'Accent',
                    type: 'color',
                    value: '1.00 0.42 0.62',
                },
                speed: {
                    order: 1,
                    text: 'Speed',
                    type: 'slider',
                    value: 1,
                    min: 0,
                    max: 2,
                },
            },
        },
        preview,
        tags: ['Fixture'],
        title,
        type,
        visibility: 'private',
    }
}

/**
 * `scene.json`：零图层的正交场景。
 *
 * 每个字段都是库的解析器真的会读的（`general.clearenabled` / `clearcolor` /
 * `orthogonalprojection`）。`objects: []` 是刻意的 —— 这样这张壁纸的整帧输出
 * 就是清屏色，肉眼一眼能判「首帧到底有没有画出来」。
 */
function sceneJson() {
    return {
        camera: {
            center: '0.00 0.00 0.00',
            eye: '0.00 0.00 1.00',
            fov: 50,
            farz: 100,
            nearz: 0.1,
            up: '0.00 1.00 0.00',
        },
        general: {
            ambientcolor: '0.00 0.00 0.00',
            clearcolor: '0.12 0.16 0.24',
            clearenabled: true,
            cameraparallax: false,
            cameraparallaxamount: 0.1,
            cameraparallaxdelay: 0,
            cameraparallaxmouseinfluence: 0.1,
            camerashake: false,
            fov: 50,
            farz: 100,
            nearz: 0.1,
            hdr: false,
            orthogonalprojection: { height: SCENE_HEIGHT, width: SCENE_WIDTH },
            skylightcolor: '0.00 0.00 0.00',
        },
        objects: [],
    }
}

/* ------------------------------------------------------------------ *
 * PKGM0019 容器
 * ------------------------------------------------------------------ */

/**
 * 打包成 `PKGM0019`。
 *
 * 格式（与 Rust 侧 `wallpaper::pkg` 逐字节一致，见该文件 `parse_header`）：
 *
 * ```text
 * u32 magicLen | magic("PKGM0019") | u32 count |
 * count × ( u32 nameLen | name | u32 offset | u32 size )
 * 数据区（条目不压缩、紧挨着排在头后面）
 * ```
 *
 * 所有整数小端。⚠ `offset` 是**相对数据区起点**（即头部末尾）的偏移，
 * 不是相对文件开头 —— Rust 侧记的是 `data_base = 头部读完的位置`，再用
 * `data_base + offset` 定位。这一条弄反了包能"打开"但每个条目读到的都是错位的字节。
 */
function packPkg(entries) {
    const MAGIC = 'PKGM0019'
    const magic = Buffer.from(MAGIC, 'ascii')

    const headerSize =
        4 + magic.length + 4 + entries.reduce((total, entry) => total + 4 + Buffer.byteLength(entry.name, 'utf8') + 8, 0)

    let dataOffset = 0
    const table = []
    const bodies = []
    for (const entry of entries) {
        const name = Buffer.from(entry.name, 'utf8')
        const record = Buffer.alloc(4 + name.length + 8)
        record.writeUInt32LE(name.length, 0)
        name.copy(record, 4)
        record.writeUInt32LE(dataOffset, 4 + name.length)
        record.writeUInt32LE(entry.body.length, 8 + name.length)
        table.push(record)
        bodies.push(entry.body)
        dataOffset += entry.body.length
    }

    const magicLen = Buffer.alloc(4)
    magicLen.writeUInt32LE(magic.length, 0)
    const count = Buffer.alloc(4)
    count.writeUInt32LE(entries.length, 0)

    const out = Buffer.concat([magicLen, magic, count, ...table, ...bodies])
    if (out.length !== headerSize + dataOffset) {
        throw new Error(`header size mismatch: ${out.length} != ${headerSize + dataOffset}`)
    }
    return out
}

/** 用**独立实现**把包读回来，逐条目比对字节。fixture 自己错了一切都白搭。 */
function readPkg(buffer) {
    const magicLen = buffer.readUInt32LE(0)
    const magic = buffer.subarray(4, 4 + magicLen).toString('ascii')
    if (magic !== 'PKGM0019' && magic !== 'PKGV0024') {
        throw new Error(`bad magic: ${magic}`)
    }
    let cursor = 4 + magicLen
    const count = buffer.readUInt32LE(cursor)
    cursor += 4

    // 第一遍只读条目表（offset 相对数据区，数据区起点要等整张表读完才知道）。
    const records = []
    for (let index = 0; index < count; index += 1) {
        const nameLen = buffer.readUInt32LE(cursor)
        cursor += 4
        const name = buffer.subarray(cursor, cursor + nameLen).toString('utf8')
        cursor += nameLen
        const offset = buffer.readUInt32LE(cursor)
        cursor += 4
        const size = buffer.readUInt32LE(cursor)
        cursor += 4
        records.push({ name, offset, size })
    }

    const dataBase = cursor
    const entries = records.map((record) => ({
        body: buffer.subarray(dataBase + record.offset, dataBase + record.offset + record.size),
        name: record.name,
    }))
    return { dataBase, entries, magic }
}

/* ------------------------------------------------------------------ *
 * 主流程
 * ------------------------------------------------------------------ */

async function writeTree(root, files) {
    await rm(root, { force: true, recursive: true })
    await mkdir(root, { recursive: true })
    for (const [name, body] of files) {
        const target = join(root, name)
        await mkdir(dirname(target), { recursive: true })
        await writeFile(target, body)
    }
}

async function main() {
    const args = process.argv.slice(2)
    const outIndex = args.indexOf('--out')
    const videoIndex = args.indexOf('--video')
    const outDir = resolve(
        outIndex >= 0 && args[outIndex + 1] ? args[outIndex + 1] : join(REPO_ROOT, 'tmp', 'wallpaper-fixture'),
    )
    const videoSource = videoIndex >= 0 ? args[videoIndex + 1] : undefined

    await mkdir(outDir, { recursive: true })

    const preview = solidPng(8, [31, 41, 61])
    const sceneFiles = [
        ['project.json', Buffer.from(JSON.stringify(projectJson({
            file: 'scene.json',
            preview: 'preview.png',
            title: 'HaYenai Fixture · Solid Scene',
            type: 'scene',
        }), null, 2))],
        ['scene.json', Buffer.from(JSON.stringify(sceneJson(), null, 2))],
        ['preview.png', preview],
    ]

    // 1) 松散工程目录
    const sceneLoose = join(outDir, 'scene-loose')
    await writeTree(sceneLoose, sceneFiles)

    // 2) 容器形态：同样三个条目打成 PKGM0019
    const mpkgPath = join(outDir, 'scene.mpkg')
    await writeFile(mpkgPath, packPkg(sceneFiles.map(([name, body]) => ({ body, name }))))

    // 3) 视频形态（清单 + 预览图，视频文件由 --video 拷入）
    const videoLoose = join(outDir, 'video-loose')
    await writeTree(videoLoose, [
        ['project.json', Buffer.from(JSON.stringify(projectJson({
            file: 'clip.mp4',
            preview: 'preview.png',
            title: 'HaYenai Fixture · Video',
            type: 'video',
        }), null, 2))],
        ['preview.png', preview],
    ])
    if (videoSource) {
        await copyFile(videoSource, join(videoLoose, 'clip.mp4'))
    }

    /* ---- 自检：把包读回来，逐字节比对 ---- */
    const parsed = readPkg(await readFile(mpkgPath))
    if (parsed.entries.length !== sceneFiles.length) {
        throw new Error(`entry count mismatch: ${parsed.entries.length} != ${sceneFiles.length}`)
    }
    for (const [name, body] of sceneFiles) {
        const entry = parsed.entries.find((candidate) => candidate.name === name)
        if (!entry) throw new Error(`missing entry ${name}`)
        if (!entry.body.equals(body)) throw new Error(`entry ${name} differs from source bytes`)
    }

    const sha = createHash('sha1').update(await readFile(mpkgPath)).digest('hex').slice(0, 12)
    const sizes = await Promise.all(
        (await readdir(outDir, { withFileTypes: true }))
            .filter((entry) => entry.isFile())
            .map(async (entry) => `${entry.name} = ${(await stat(join(outDir, entry.name))).size} B`),
    )

    console.log(`fixture written to ${outDir}`)
    console.log(`  scene-loose/   project.json + scene.json + preview.png (松散工程形态)`)
    console.log(`  scene.mpkg     PKGM0019, ${parsed.entries.length} entries, sha1=${sha}`)
    console.log(`  video-loose/   project.json + preview.png${videoSource ? ' + clip.mp4' : '（缺 clip.mp4，用 --video 提供）'}`)
    for (const line of sizes) console.log(`  ${line}`)
    console.log('\n自检通过：容器 magic、条目数、每个条目的字节都与源文件一致。')
}

await main()
