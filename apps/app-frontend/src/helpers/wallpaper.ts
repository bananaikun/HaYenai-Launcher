/**
 * Wallpaper Engine 壁纸的前端封装。
 *
 * 这一层只做四件事，任何一件都不该在别处重复实现：
 *
 * 1. **命令**：`wallpaper_*` 六个 Tauri 命令的类型化包装（Rust 侧已就绪，
 *    见 `apps/app/src/api/wallpaper.rs`）。配置一律落在
 *    `$APPDATA/<id>/wallpapers/config.json`，**不写 settings 表** —— 那张表的
 *    UPDATE 是几十个位置参数，加字段的代价在架构文档 §5.1 里有记录。
 * 2. **取流地址**：`http://hayenai-we.localhost/<id>/<rel>`（Windows 形态，
 *    协议注册在 `apps/app/src/main.rs`，逻辑在 `theseus::wallpaper::protocol`）。
 *    包 id 带内容哈希 ⇒ 同一条 URL 的字节永不改变，服务端回 immutable。
 * 3. **共享状态**：背景层（`App.vue`）与设置页（外观 → 壁纸）读同一份
 *    `wallpaperState`，所以「选中后背景立刻切换」不需要事件总线或重启。
 * 4. **设备能力**：WebGL2 / ETC2 / BPTC 探一次就缓存。包能不能画，与包好不好
 *    是两件事 —— 前者问设备，后者问 `texAudit`。
 */

import { invoke } from '@tauri-apps/api/core'

/** 壁纸命令注册在 `wallpaper` 插件里，必须带 `plugin:` 前缀。 */
const PLUGIN = 'plugin:wallpaper'
import { reactive } from 'vue'

/** `project.json.type` 归一化后的类型，与 Rust 侧 `WallpaperType` 一一对应。 */
export type WallpaperKind = 'scene' | 'video' | 'web' | 'application' | 'other'

/** 后处理档，对应 `webwallgl` 的 `quality.postProcessing`。 */
export type WallpaperPostProcessing = 'high' | 'medium' | 'low' | 'off'

/** WE 用户属性只出现标量：bool → boolean，slider → number，其余 → string。 */
export type WallpaperPropertyValue = boolean | number | string

/** `.tex` 体检结果。只回答「这个包需要 GPU 的哪项能力」，不回答「画不画得出」。 */
export interface WallpaperTexAudit {
    /** 命中 ETC2（format 3/5）。ANGLE-D3D11 上没有对应扩展 ⇒ 会被静默拒收。 */
    etc2: boolean
    /** 命中 DXT1/DXT3/DXT5（S3TC / BC1-3）。 */
    dxt: boolean
    /** 命中 BPTC（BC7）。 */
    bptc: boolean
    /** 扫到的 `.tex` 总数。 */
    total: number
    /** 文件头读不出来的张数（旧容器 / 损坏 / 库不认的格式值）。 */
    unreadable: number
    /** 格式名 → 张数。 */
    formats: Record<string, number>
}

/** 一个已安装壁纸的元数据，字段名与 Rust 侧 `WallpaperMeta` 的序列化结果一致。 */
export interface WallpaperMeta {
    id: string
    type: WallpaperKind
    title: string
    description: string
    tags: string[]
    workshopId: string
    /** 入口文件：场景是 `scene.json`，视频是那个 mp4 的相对路径。 */
    file: string
    /** 预览图相对路径；包内没有就是 `null`。 */
    preview: string | null
    entries: number
    bytes: number
    /** Unix 毫秒。 */
    installedAt: number
    hasProperties: boolean
    texAudit: WallpaperTexAudit
    /** `file` 指向的文件不存在时为 `true`。 */
    damaged: boolean
}

/** 渲染档位。默认值来自架构文档 §0。 */
export interface WallpaperQuality {
    fps: number
    postProcessing: WallpaperPostProcessing
    /**
     * 抗锯齿模式。2026-10-30 20:00 由布尔改成四档，与 WE 客户端一致：
     * `off` / `fxaa`（帧末后处理抗锯齿）/ `msaa2` / `msaa4`（多重采样）。
     * 老配置里的 `antialiasing: true` 由 `normalizeWallpaperQuality()` 迁移成 `fxaa`。
     */
    antiAliasing: WallpaperAntiAliasing
    /** 粒子质量（webwallgl 支持，WE 客户端的「阴影/体积光/反射」是 3D 特性，2D 合成做不到）。 */
    particles: WallpaperParticles
    /** 贴图倍率，0.25 ~ 1。 */
    textureScale: number
}

/** `wallpapers/config.json` 的内容。 */
export interface WallpaperConfig {
    selected: string | null
    properties: Record<string, Record<string, WallpaperPropertyValue>>
    quality: WallpaperQuality
}

/** `wallpaper_import` 的返回值。 */
export interface WallpaperImportOutcome {
    meta: WallpaperMeta
    /** 这个包本来就在（内容哈希撞上了），本次没有多占磁盘。 */
    alreadyInstalled: boolean
    /** 中文单句提示：ETC2 贴图、暂不支持的类型、入口文件缺失。 */
    warnings: string[]
}

/* ------------------------------------------------------------------ *
 * Steam 库（Wallpaper Engine 的工坊目录）
 * ------------------------------------------------------------------ */

/**
 * 一个工坊目录在磁盘上的状态。
 *
 * 注意这三个值与「有没有导入过」是**两件独立的事**：前者说的是 Steam 那边
 * 的目录完不完整，后者由 `importedId` 回答。
 *
 * `unknown` 是 Rust 侧的第三种答案（`appworkshop_431960.acf` 读不到）——
 * 它既不是「装好了」也不是「没下载」，所以同样不给导入按钮。
 */
export type WallpaperSteamItemState = 'installed' | 'orphan' | 'missing' | 'unknown'

/** `wallpaper_steam_scan` 返回的一个条目。字段名与 Rust 侧一一对应。 */
export interface WallpaperSteamItem {
    /** 工坊 id，十进制字符串（Steam 的 URL 与目录名都用它）。 */
    workshopId: string
    /** 工坊目录的绝对路径。 */
    dir: string
    /** `project.json` 里的标题；读不到时退回目录名。 */
    title: string
    kind: WallpaperKind
    /** 包内预览图的相对文件名；没有就是 `null`。 */
    preview: string | null
    /** 目录体积（字节）。 */
    bytes: number
    /** Unix 毫秒；读不到 `project.json` 时为 `null`。 */
    installedAt: number | null
    /** 目录完整度。除 `installed` 之外的都不能导入。 */
    itemState: WallpaperSteamItemState
    /** 已经导入过的话，是本地壁纸包的 id；否则 `null`。 */
    importedId: string | null
}

/** 被跳过的一个目录与原因（Rust 侧 `SteamSkipped`）。 */
export interface WallpaperSteamSkippedEntry {
    dir: string
    reason: string
}

/**
 * `wallpaper_steam_scan` 的**原始**返回值。
 *
 * `skipped` 在这里是联合类型，只为了兼容两侧的契约差异：Rust 目前发的是
 * `SteamSkipped[]`（带原因，能进 tooltip），早期约定是纯数字。下面的
 * `scanSteamWallpapers()` 会把它归一成一个数字 + 一份原因清单，页面只认归一后的形状。
 */
export interface WallpaperSteamScanRaw {
    items: WallpaperSteamItem[]
    skipped: number | WallpaperSteamSkippedEntry[]
    roots?: string[]
    steamRoot?: string | null
}

/** 归一后的扫描结果。页面用这一份，不直接碰原始形状。 */
export interface WallpaperSteamScanResult {
    items: WallpaperSteamItem[]
    /** 跳过的目录数。大于 0 时页面要给一条非阻断提示。 */
    skipped: number
    /** 中文单句的跳过原因，可以进 tooltip。可能比 `skipped` 短。 */
    skippedReasons: string[]
    /** 实际扫过的 `workshop/content/431960` 目录。 */
    roots: string[]
    /** Steam 安装根目录；读不到就是 `null`。 */
    steamRoot: string | null
}

/** 工坊 id → Steam 的社区文件页（客户端协议，不是 https）。 */
export function steamWorkshopPageUrl(workshopId: string): string {
    return `steam://url/CommunityFilePage/${encodeURIComponent(workshopId)}`
}

/**
 * 工坊预览图的取流地址。
 *
 * 与本地包不同，工坊目录**不在** `packages/` 下，所以协议侧给了一条独立的
 * `/steam/<workshopId>/<相对文件名>` 前缀。段级编码与 `wallpaperAssetUrl` 同源。
 *
 * 注意这条路径只对**本次会话扫到过**的工坊目录有效（Rust 侧靠一张进程内的
 * 目录表解析，见 `steam::remembered_dir`）—— 所以调它之前必须先 `scan`。
 */
export function steamWallpaperPreviewUrl(
    workshopId: string,
    preview: string | null,
): string | null {
    if (!preview) return null
    return `${WALLPAPER_PROTOCOL_ORIGIN}/steam/${encodeURIComponent(workshopId)}/${encodeWallpaperRelPath(preview)}`
}

/**
 * 这个工坊条目现在能不能导入。
 *
 * 「已经导入过」与「目录不完整」都拒绝 —— 前者是幂等的重复劳动（65 个包、
 * 1.8 GB，重复导入不是零代价），后者根本没有可读的内容。
 */
export function canImportSteamWallpaper(
    item: Pick<WallpaperSteamItem, 'itemState' | 'importedId'>,
): boolean {
    return item.importedId === null && item.itemState === 'installed'
}

/** 扫一遍本机的 Steam 库。没有 Wallpaper Engine 时 Rust 侧报错，由调用方兜底。 */
export async function scanSteamWallpapers(): Promise<WallpaperSteamScanResult> {
    const raw = await invoke<WallpaperSteamScanRaw>(`${PLUGIN}|wallpaper_steam_scan`)
    const entries = Array.isArray(raw.skipped) ? raw.skipped : []
    return {
        items: Array.isArray(raw.items) ? raw.items : [],
        skipped: Array.isArray(raw.skipped) ? raw.skipped.length : (raw.skipped ?? 0),
        skippedReasons: entries
            .map((entry) => entry.reason)
            .filter((reason): reason is string => typeof reason === 'string' && reason.length > 0),
        roots: Array.isArray(raw.roots) ? raw.roots : [],
        steamRoot: raw.steamRoot ?? null,
    }
}

/** 从 Steam 库导入一个工坊条目。返回值与手动导入完全一致。 */
export async function importSteamWallpaper(workshopId: string): Promise<WallpaperImportOutcome> {
    return await invoke<WallpaperImportOutcome>(`${PLUGIN}|wallpaper_steam_import`, { workshopId })
}

/** 取流协议名（自定义协议，不开端口）。 */
export const WALLPAPER_PROTOCOL_SCHEME = 'hayenai-we'
/** Windows 上 WebView 把自定义协议映射成的域名。 */
export const WALLPAPER_PROTOCOL_HOST = 'hayenai-we.localhost'
/** 协议源。macOS / Linux 的 `hayenai-we://localhost` 形态由 Rust 侧一并接收。 */
export const WALLPAPER_PROTOCOL_ORIGIN = `http://${WALLPAPER_PROTOCOL_HOST}`

/** 贴图倍率在 URL 上的参数名（库只认页面 URL，不认挂载选项）。 */
const TEXTURE_SCALE_PARAM = 'resources'

/**
 * 默认档位：30fps / medium / AA 关 / **1.0 倍率**。
 *
 * 倍率原来是 0.8 —— 也就是「按原图 80% 的分辨率去装配贴图」，再叠上库自己的
 * autoQuality 只降不升，最后的画面就是一层糊。主人一直说「这不是模糊，是缩略图」，
 * 指的就是这个：它压根不是 poster 兜底，而是**真的用低分辨率渲的**。
 * 1.0 会走到库的 `native` 分支，贴图按原始尺寸装配。
 */
export type WallpaperAntiAliasing = 'off' | 'fxaa' | 'msaa2' | 'msaa4'
export type WallpaperParticles = 'off' | 'low' | 'medium' | 'high'

/** 四档预设，对应 WE 客户端的「低 / 中 / 高 / 超高」。 */
export type WallpaperQualityPreset = 'low' | 'medium' | 'high' | 'ultra'

export const WALLPAPER_QUALITY_PRESETS: Record<WallpaperQualityPreset, WallpaperQuality> = {
    low: {
        fps: 30,
        postProcessing: 'off',
        antiAliasing: 'off',
        particles: 'low',
        textureScale: 0.5,
    },
    medium: {
        fps: 30,
        postProcessing: 'medium',
        antiAliasing: 'fxaa',
        particles: 'medium',
        textureScale: 0.75,
    },
    high: {
        fps: 60,
        postProcessing: 'high',
        antiAliasing: 'msaa2',
        particles: 'high',
        textureScale: 1,
    },
    ultra: {
        fps: 60,
        postProcessing: 'high',
        antiAliasing: 'msaa4',
        particles: 'high',
        textureScale: 1,
    },
}

export const DEFAULT_WALLPAPER_QUALITY: WallpaperQuality = {
    /* 2026-11-01 主人拍板：「默认为我 WE 现在的配置」——
       帧率 60 / 抗锯齿 MSAA ×2 / 后处理启用(high) / 纹理分辨率自动(1.0) / 粒子 high。
       这正好等于「高」档预设，所以直接从这里派生，别再各写一份数字（两张表迟早对不上）。

       ⚠ 之前这里写的是 30fps / medium，理由是「Radeon 760M 实测只有 36fps」。
       但那是**在抗锯齿被 bug 关掉、且壁纸后处理跑 high** 的前提下测的，
       而且主人要的是跟 WE 一致 —— 性能不行他会自己在设置里往下调。
       真正的性能开关在「设置 → 壁纸」的品质档，不在代码里替他做决定。 */
    ...WALLPAPER_QUALITY_PRESETS.high,
}

/**
 * 把可能来自旧版本 / 手改过的画质对象收成合法值。
 *
 * 迁移规则：旧版 `antialiasing: boolean` → 新 `antiAliasing`：
 * `true` 落 `fxaa`（旧版一直是 fxaa 语义），`false` 落 `off`。
 */
export function normalizeWallpaperQuality(input: unknown): WallpaperQuality {
    const raw = (input ?? {}) as Record<string, unknown>
    const legacy = typeof raw.antialiasing === 'boolean' ? raw.antialiasing : undefined

    const antiAliasing: WallpaperAntiAliasing =
        raw.antiAliasing === 'off' ||
        raw.antiAliasing === 'fxaa' ||
        raw.antiAliasing === 'msaa2' ||
        raw.antiAliasing === 'msaa4'
            ? raw.antiAliasing
            : legacy === undefined
              ? DEFAULT_WALLPAPER_QUALITY.antiAliasing
              : legacy
                ? 'fxaa'
                : 'off'

    const particles: WallpaperParticles =
        raw.particles === 'off' ||
        raw.particles === 'low' ||
        raw.particles === 'medium' ||
        raw.particles === 'high'
            ? raw.particles
            : DEFAULT_WALLPAPER_QUALITY.particles

    const postProcessing: WallpaperPostProcessing =
        raw.postProcessing === 'off' ||
        raw.postProcessing === 'low' ||
        raw.postProcessing === 'medium' ||
        raw.postProcessing === 'high'
            ? raw.postProcessing
            : DEFAULT_WALLPAPER_QUALITY.postProcessing

    const fpsRaw = Number(raw.fps)
    const fps = Number.isFinite(fpsRaw) ? Math.min(240, Math.max(15, Math.round(fpsRaw))) : DEFAULT_WALLPAPER_QUALITY.fps

    const scaleRaw = Number(raw.textureScale)
    const textureScale = Number.isFinite(scaleRaw)
        ? Math.min(1, Math.max(0.25, scaleRaw))
        : DEFAULT_WALLPAPER_QUALITY.textureScale

    return { fps, postProcessing, antiAliasing, particles, textureScale }
}

/** 默认配置。读不到 `config.json` 时用它，语义与 Rust 侧的 `Default` 相同。 */
export function defaultWallpaperConfig(): WallpaperConfig {
    return {
        selected: null,
        properties: {},
        quality: { ...DEFAULT_WALLPAPER_QUALITY },
    }
}

/* ------------------------------------------------------------------ *
 * 取流地址
 * ------------------------------------------------------------------ */

/**
 * 把包内相对路径编码成 URL 路径片段。
 *
 * 逐段编码而不是整串 `encodeURIComponent`：包内路径是有层级的
 * （`materials/foo.json`），整串编码会把 `/` 变成 `%2F`，协议侧的
 * `safe_rel` 就把它当成一个文件名了。空段（`a//b`）直接丢掉。
 */
export function encodeWallpaperRelPath(rel: string): string {
    return rel
        .split('/')
        .filter((segment) => segment.length > 0)
        .map((segment) => encodeURIComponent(segment))
        .join('/')
}

/** 一个包的取流基址，**带尾斜杠** —— 库按 `{基址}/{相对路径}` 拼每个资源。 */
export function wallpaperBaseUrl(id: string): string {
    return `${WALLPAPER_PROTOCOL_ORIGIN}/${encodeURIComponent(id)}/`
}

/** 包内某个文件的取流地址。 */
export function wallpaperAssetUrl(id: string, rel: string): string {
    return `${wallpaperBaseUrl(id)}${encodeWallpaperRelPath(rel)}`
}

/**
 * 海报地址：包内 `preview` 的取流地址。
 *
 * 为什么不用 `wallpapers/posters/<id>.jpg`：那份副本是给「包被删了一半」这类
 * 场景留的兜底，而取流协议只映射 `packages/<id>/<rel>`，海报目录不在协议里。
 * 预览图本来就在包内，直接用协议取，少一条路径、少一次 IO。
 */
export function wallpaperPosterUrl(
    meta: Pick<WallpaperMeta, 'id' | 'preview'> | null | undefined,
): string | null {
    if (!meta || !meta.preview) return null
    return wallpaperAssetUrl(meta.id, meta.preview)
}

/* ------------------------------------------------------------------ *
 * 贴图倍率：库只认页面 URL 上的 `resources` 参数
 * ------------------------------------------------------------------ */

/**
 * 把贴图倍率写进当前页 URL 的 query。
 *
 * `webwallgl@2.1.0` 的 `resourceScaleFor()` 无条件先读 `location.search` 里的
 * `resources`（`resourceOverride`），挂载选项里**没有**对应项；实例上沾边的
 * `setRenderDpr` 会连带改画布 backing store，不是同一个东西。所以想让「0.8 倍率」
 * 真的生效，只能改 URL。
 *
 * `history.replaceState` 幂等、不导航、不进历史。库只在**场景装配那一刻**读它，
 * 因此必须在每次 `mount()` 之前重新写一遍 —— 中途用户点了导航，router 会把
 * query 抹掉（history 模式下 router 自己 push/replace 会覆盖我们这一笔）。
 *
 * 值域照抄库的字面量：它把 `"native"` / `"off"` / `"0"` 当 native，
 * 把 `"auto"` 当「交回库自己算」，数字串才走倍率分支。
 */
export function applyWallpaperTextureScale(scale: number): void {
    try {
        const url = new URL(window.location.href)
        const value = textureScaleLiteral(scale)
        if (value === null) url.searchParams.delete(TEXTURE_SCALE_PARAM)
        else url.searchParams.set(TEXTURE_SCALE_PARAM, value)
        if (url.href === window.location.href) return
        window.history.replaceState(window.history.state, '', url.href)
    } catch {
        // 某些沙箱（禁 history 的 file:// 页面）会拒 replaceState。
        // 退回库自己的默认倍率，画面照常，不致命。
    }
}

/** 把倍率翻成库认的字面量。`>= 1` 落 native，非有限数落 `null`（删参数）。 */
function textureScaleLiteral(scale: number): string | null {
    if (!Number.isFinite(scale) || scale <= 0) return null
    if (scale >= 0.999) return 'native'
    return String(Math.round(scale * 100) / 100)
}

/** 停用壁纸时清掉那一笔，别把参数永久留在用户地址栏里。 */
export function clearWallpaperTextureScale(): void {
    applyWallpaperTextureScale(0)
}

/* ------------------------------------------------------------------ *
 * 设备能力：探一次，缓存整场会话
 * ------------------------------------------------------------------ */

/** 这台机器能不能画某种贴图 / 有没有硬件 WebGL2。 */
export interface WallpaperGpuCaps {
    /** 有硬件 WebGL2（`failIfMajorPerformanceCaveat: true` 也拿得到上下文）。 */
    webgl2: boolean
    /** 拿得到 ETC2 常量（`webwallgl` 对 format 3/5 取常量前**不查扩展**）。 */
    etc2: boolean
    /** 拿得到 BPTC 常量（format 12）。 */
    bptc: boolean
    /**
     * 这次探测本身跑完了吗。
     *
     * `false` = 探测**没跑成**（抛异常、没有 `document`、创建 canvas 失败）。
     * 此时上面三个字段全是没意义的 `false`，**不能**拿来判断「这台机器画不出来」——
     * 那是「不知道」，不是「不行」。缺省（`undefined`）按 `true` 处理，
     * 老的构造与单测因此不受影响。
     */
    probed?: boolean
}

let cachedCaps: WallpaperGpuCaps | null = null

/**
 * 探一次本机的 WebGL2 能力。
 *
 * `failIfMajorPerformanceCaveat: true` 是关键：拿不到上下文就意味着这台机器在走
 * 软件渲染（SwiftShader 之类），此时**根本不建**渲染上下文，直接走海报
 * （架构文档 §4.1 第 10 条）。探完立刻用 `WEBGL_lose_context` 主动丢弃上下文，
 * 免得为一次探测常驻一份 GL 状态。
 *
 * **探测失败不缓存**：抛异常时返回 `probed: false`，下一次调用会重试。缓存住一个
 * 「不知道」会让整场会话都把 49 个场景包判成画不出来 —— 用户看到的正是
 * 「添加了但点不动」。
 */
export function probeWallpaperGpu(): WallpaperGpuCaps {
    if (cachedCaps) return cachedCaps

    let caps: WallpaperGpuCaps = { webgl2: false, etc2: false, bptc: false, probed: false }
    let canvas: HTMLCanvasElement | null = null
    try {
        canvas = document.createElement('canvas')
        canvas.width = 1
        canvas.height = 1
        // `getContext` 返回 null 是一个**明确的**答案（这台机器没有硬件 WebGL2），
        // 只有抛异常才是「不知道」。
        caps = { webgl2: false, etc2: false, bptc: false, probed: true }
        const gl = canvas.getContext('webgl2', {
            failIfMajorPerformanceCaveat: true,
        }) as (WebGL2RenderingContext & Record<string, unknown>) | null

        if (gl) {
            caps.webgl2 = true
            // 这两个常量在类型声明里是 number，但**运行时真的可能是 undefined**：
            // ANGLE-D3D11（WebView2 的形态）只给 S3TC，ETC2 / BPTC 的常量根本不存在。
            caps.etc2 =
                typeof gl.COMPRESSED_RGBA8_ETC2_EAC === 'number' ||
                gl.getExtension('WEBGL_compressed_texture_etc') !== null
            caps.bptc =
                typeof gl.COMPRESSED_RGBA_BPTC_UNORM_EXT === 'number' ||
                typeof gl.COMPRESSED_RGBA_BPTC_UNORM === 'number' ||
                gl.getExtension('EXT_texture_compression_bptc') !== null
            gl.getExtension('WEBGL_lose_context')?.loseContext()
        }
    } catch {
        // 探测本身失败是「不知道」，不是「不行」：不缓存，也不据此禁用任何东西。
        // 调用方拿到 `probed: false` 就会乐观放行，真画不出来由 WallpaperLayer 降级到海报。
        caps = { webgl2: false, etc2: false, bptc: false, probed: false }
    } finally {
        canvas = null
    }

    // 只有探到底了才记缓存；`probed: false` 留空，下次重新试。
    if (caps.probed) cachedCaps = caps
    return caps
}

/** 单测/调试用：丢掉缓存，下一次重新探测。 */
export function resetWallpaperGpuProbe(): void {
    cachedCaps = null
}

/* ------------------------------------------------------------------ *
 * 命令封装
 * ------------------------------------------------------------------ */

/**
 * 从 Tauri 的拒绝值里取一句用户可读的话。
 *
 * Rust 侧所有壁纸命令都返回 `{ fieldName, message }`（单句中文，不含路径与堆栈），
 * 直接 `String(error)` 只会得到 `[object Object]`。取不到就返回 `null`，
 * 由调用方用自己那条 i18n 兜底文案 —— 这一层不猜语言。
 */
export function wallpaperErrorMessage(error: unknown): string | null {
    if (typeof error === 'string' && error.length > 0) return error
    if (error && typeof error === 'object') {
        const message = (error as { message?: unknown }).message
        if (typeof message === 'string' && message.length > 0) return message
    }
    return null
}

/** 列出所有已安装的壁纸。损坏的也会出来（`damaged: true`）。 */
export async function listWallpapers(): Promise<WallpaperMeta[]> {
    return await invoke<WallpaperMeta[]>(`${PLUGIN}|wallpaper_list`)
}

/** 导入一个包。`path` 是 `.mpkg` / `.pkg` 文件或目录。 */
export async function importWallpaper(path: string): Promise<WallpaperImportOutcome> {
    return await invoke<WallpaperImportOutcome>(`${PLUGIN}|wallpaper_import`, { path })
}

/** 删除一个包。目录本来就不在也算成功（幂等）。 */
export async function removeWallpaper(id: string): Promise<void> {
    await invoke(`${PLUGIN}|wallpaper_remove`, { id })
}

/** 选中（传 id）或取消选中（传 `null`）。写 `wallpapers/config.json`。 */
export async function selectWallpaper(id: string | null): Promise<void> {
    await invoke(`${PLUGIN}|wallpaper_select`, { id })
}

/** 读 `wallpapers/config.json`。文件不存在时 Rust 侧返回默认值。 */
export async function getWallpaperConfig(): Promise<WallpaperConfig> {
    const config = await invoke<WallpaperConfig>(`${PLUGIN}|wallpaper_config_get`)
    /* 2026-10-30 20:15：读出来就归一化一次。
       旧版 `quality.antialiasing` 是布尔、且没有 `particles` 字段；
       不迁移的话 `qualityPatch()` 会拿到 `undefined` 然后静默用库默认（AA off）。
       这里一次性收成新结构，调用方就不必各自兼容。 */
    const normalized = normalizeWallpaperQuality(config.quality)

    /* 2026-11-01 修「每次切壁纸画质就重置」。

       旧指纹迁移是这轮事故的放大器：Rust 侧 `WallpaperQuality` 少了 `particles`、
       且 `antialiasing` 拼写对不上前端的 `antiAliasing`，**存盘时这两个键都被丢掉**。
       于是用户按 WE 设成 60/high/1.0 之后，落盘的正好是「旧默认指纹」，
       每次读配置都被判定成"从没改过" → 覆盖回 30fps/medium。
       （Rust 侧字段已补齐，存盘不再丢数据；这里把误判的指纹一起换掉。）

       现在的判据只认一件事：**这份配置是不是被那个会丢字段的旧 store 写出来的**
       —— 缺 `particles` 或 缺 `antiAliasing` 就必然是。那种值不可能是用户选的
       （UI 上根本没有对应的形状），一律覆盖成新默认。
       迁移完这两个键就存在了，下次不会再触发，天然幂等。 */
    const raw = (config.quality ?? {}) as Record<string, unknown>
    const writtenByLossyStore = raw.particles === undefined || raw.antiAliasing === undefined
    if (writtenByLossyStore) {
        const migrated: WallpaperConfig = {
            ...config,
            quality: { ...DEFAULT_WALLPAPER_QUALITY },
        }
        // 写失败不阻断读取：本次仍按新默认跑，下次再试
        await setWallpaperConfig(migrated).catch(() => undefined)
        return migrated
    }

    return { ...config, quality: normalized }
}

/** 覆盖写 `wallpapers/config.json`（临时文件 + 原子 rename）。 */
export async function setWallpaperConfig(config: WallpaperConfig): Promise<void> {
    await invoke(`${PLUGIN}|wallpaper_config_set`, { config })
}

/* ------------------------------------------------------------------ *
 * 共享状态
 * ------------------------------------------------------------------ */

interface WallpaperState {
    /** 至少成功读过一次列表。 */
    loaded: boolean
    loading: boolean
    /** 已安装的包，按安装时间倒序。 */
    items: WallpaperMeta[]
    config: WallpaperConfig
    /** 最近一次读取失败的用户可读原因；成功后清空。 */
    error: string | null
}

/**
 * 背景层与设置页共用的一份状态。
 *
 * 反应式对象放在模块作用域：`App.vue` 与外观页不需要父子关系也能看到同一份
 * 「当前选中的是谁」，这是「选中后背景立即切换」的全部机制。
 */
export const wallpaperState = reactive<WallpaperState>({
    loaded: false,
    loading: false,
    items: [],
    config: defaultWallpaperConfig(),
    error: null,
})

let inflightRefresh: Promise<void> | null = null

/** 当前选中的包；配置里指向一个不存在的包时视为未选中。 */
export function selectedWallpaper(): WallpaperMeta | null {
    const id = wallpaperState.config.selected
    if (!id) return null
    return wallpaperState.items.find((item) => item.id === id) ?? null
}

/**
 * 重新读列表与配置。
 *
 * 两件事都做，且**并发调用合并成一次**：`App.vue` 的启动钩子与外观页的
 * `onMounted` 会先后触发，没必要扫两遍目录。
 */
export async function refreshWallpapers(): Promise<void> {
    if (inflightRefresh) return await inflightRefresh

    inflightRefresh = (async () => {
        wallpaperState.loading = true
        try {
            const [items, config] = await Promise.all([listWallpapers(), getWallpaperConfig()])
            wallpaperState.items = sortWallpapers(items)
            wallpaperState.config = config
            wallpaperState.error = null
            wallpaperState.loaded = true
            await pruneMissingSelection()
        } catch (error) {
            wallpaperState.error = wallpaperErrorMessage(error)
        } finally {
            wallpaperState.loading = false
            inflightRefresh = null
        }
    })()

    return await inflightRefresh
}

/** 只在第一次需要时读一次（背景层启动用，不覆盖设置页后续的刷新）。 */
export async function ensureWallpapersLoaded(): Promise<void> {
    if (wallpaperState.loaded) return
    await refreshWallpapers()
}

/**
 * 配置里选中的包被用户手动删了 ⇒ 静默把 `selected` 清空。
 *
 * 架构文档 §5.5 的约定：这种情况退纯色、**不弹错误框**。写盘走
 * `wallpaper_select(null)`，失败也不重试 —— 下次启动还会再走一遍这条路径。
 */
async function pruneMissingSelection(): Promise<void> {
    const id = wallpaperState.config.selected
    if (!id) return
    if (wallpaperState.items.some((item) => item.id === id)) return

    wallpaperState.config = { ...wallpaperState.config, selected: null }
    try {
        await selectWallpaper(null)
    } catch {
        // 用户目录只读之类的极端情况：配置留在内存里已经生效，画面不会黑。
    }
}

/** 安装时间倒序（新装的排前面）；同时间按标题，保证列表不抖。 */
function sortWallpapers(items: WallpaperMeta[]): WallpaperMeta[] {
    return [...items].sort((left, right) => {
        if (right.installedAt !== left.installedAt) return right.installedAt - left.installedAt
        return left.title.localeCompare(right.title)
    })
}

/** 选中某个壁纸：写配置 + 更新内存里的那一份。 */
export async function selectWallpaperAndRefresh(id: string | null): Promise<void> {
    await selectWallpaper(id)
    wallpaperState.config = { ...wallpaperState.config, selected: id }
}

/** 删掉一个包并刷新列表（Rust 侧会顺手把配置里的痕迹清掉）。 */
export async function removeWallpaperAndRefresh(id: string): Promise<void> {
    await removeWallpaper(id)
    await refreshWallpapers()
}

/** 导入并刷新列表。返回导入结果（含 warnings，交给调用方决定怎么提示）。 */
export async function importWallpaperAndRefresh(path: string): Promise<WallpaperImportOutcome> {
    const outcome = await importWallpaper(path)
    await refreshWallpapers()
    return outcome
}

/* ------------------------------------------------------------------ *
 * 展示用的小工具
 * ------------------------------------------------------------------ */

/** 字节数 → `1.2 MB`。列表卡片与当前壁纸卡片共用，口径保持一致。 */
export function formatWallpaperBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 KB'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    let value = bytes
    let unit = 0
    while (value >= 1024 && unit < units.length - 1) {
        value /= 1024
        unit += 1
    }
    const digits = unit === 0 || value >= 100 ? 0 : 1
    return `${value.toFixed(digits)} ${units[unit]}`
}

/**
 * 这个包能不能在这台机器上画出来。
 *
 * 三种答案：
 * - `ok`：类型支持 +（场景）有硬件 WebGL2 + 没有本机解不了的贴图格式；
 * - `unsupported`：类型本身不在第一期范围（web / application / other）；
 * - `weak-gpu`：格式在本机画不出来（ETC2 / BPTC 缺常量）或没有硬件 WebGL2。
 *
 * `damaged` 单独由调用方判断 —— 「包坏了」与「机器不行」是两条不同的提示。
 *
 * **能力未知时一律乐观放行**（`caps.probed === false`）：探测没跑成 ≠ 这台机器
 * 画不出来。把它当成 `weak-gpu` 会让所有场景包在列表里变成不可点击的灰卡片，
 * 而用户根本不知道为什么。真画不出来由 `WallpaperLayer` 在装配失败时降级到海报，
 * 并且会带一行非阻断提示 —— 那是**运行时的真答案**，比一次探测的猜测可靠。
 */
export function wallpaperRenderability(
    meta: Pick<WallpaperMeta, 'type' | 'texAudit'>,
    caps: WallpaperGpuCaps = probeWallpaperGpu(),
): 'ok' | 'unsupported' | 'weak-gpu' {
    // web 类（Wallpaper Engine 的网页工程）用 iframe 承载，不吃 GPU 贴图能力，
    // 所以不需要探针结论，直接算可用。
    if (meta.type === 'web') return 'ok'
    if (meta.type !== 'scene' && meta.type !== 'video') return 'unsupported'
    if (meta.type === 'scene') {
        // 只有 `probed === false` 才算「不知道」；缺省（undefined）按「探过」处理。
        if (caps.probed === false) return 'ok'
        if (!caps.webgl2) return 'weak-gpu'
        if (meta.texAudit.etc2 && !caps.etc2) return 'weak-gpu'
        if (meta.texAudit.bptc && !caps.bptc) return 'weak-gpu'
    }
    return 'ok'
}

/** 能力探测是否「明确给出了答案」。`false` = 不知道，别拿它做禁用依据。 */
export function wallpaperCapsKnown(caps: WallpaperGpuCaps): boolean {
    return caps.probed !== false
}
