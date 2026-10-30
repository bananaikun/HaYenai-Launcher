import assert from 'node:assert/strict'
import { describe, it } from 'node:test'

import {
    defaultWallpaperConfig,
    encodeWallpaperRelPath,
    formatWallpaperBytes,
    wallpaperAssetUrl,
    wallpaperBaseUrl,
    wallpaperPosterUrl,
    wallpaperRenderability,
    type WallpaperGpuCaps,
    type WallpaperMeta,
} from './wallpaper.ts'

/* ------------------------------------------------------------------ *
 * 取流地址：形状必须与 Rust 侧 `theseus::wallpaper::protocol::resolve` 对齐
 * ------------------------------------------------------------------ */

describe('wallpaper asset urls', () => {
    it('builds the base url the webview resolves to the custom protocol', () => {
        // Windows 上自定义协议就是 `http://<scheme>.localhost`，尾斜杠不能少：
        // 库按 `{基址}/{相对路径}` 拼每个资源。
        assert.equal(wallpaperBaseUrl('local-river-0123456789ab'), 'http://hayenai-we.localhost/local-river-0123456789ab/')
    })

    it('keeps path separators and encodes each segment', () => {
        assert.equal(
            encodeWallpaperRelPath('materials/sub dir/a b.json'),
            'materials/sub%20dir/a%20b.json',
        )
        // `#` 与 `?` 在 URL 里是分隔符，不编码就会把后续路径当成 fragment/query 丢掉。
        assert.equal(encodeWallpaperRelPath('a#b/c?.json'), 'a%23b/c%3F.json')
    })

    it('drops empty segments instead of producing `//`', () => {
        assert.equal(encodeWallpaperRelPath('/materials//x.json'), 'materials/x.json')
    })

    it('composes relative paths under the package base', () => {
        assert.equal(
            wallpaperAssetUrl('local-x-000000000000', 'scene.json'),
            'http://hayenai-we.localhost/local-x-000000000000/scene.json',
        )
    })
})

describe('wallpaper poster url', () => {
    it('uses the in-package preview, which the protocol can serve', () => {
        assert.equal(
            wallpaperPosterUrl({ id: 'local-x-000000000000', preview: 'preview.jpg' }),
            'http://hayenai-we.localhost/local-x-000000000000/preview.jpg',
        )
    })

    it('returns null when the package has no preview', () => {
        assert.equal(wallpaperPosterUrl({ id: 'local-x-000000000000', preview: null }), null)
        assert.equal(wallpaperPosterUrl(null), null)
    })
})

/* ------------------------------------------------------------------ *
 * 能不能画：类型范围 + 设备能力 + 贴图格式
 * ------------------------------------------------------------------ */

const fullCaps: WallpaperGpuCaps = { webgl2: true, etc2: true, bptc: true }
const dx11Caps: WallpaperGpuCaps = { webgl2: true, etc2: false, bptc: false }
const noGpuCaps: WallpaperGpuCaps = { webgl2: false, etc2: false, bptc: false }

function metaOf(patch: Partial<WallpaperMeta>): Pick<WallpaperMeta, 'type' | 'texAudit'> {
    const audit = { etc2: false, dxt: false, bptc: false, total: 0, unreadable: 0, formats: {} }
    return {
        type: 'scene',
        texAudit: { ...audit, ...(patch.texAudit ?? {}) },
        ...(patch.type ? { type: patch.type } : {}),
    }
}

describe('wallpaper renderability', () => {
    it('renders a plain scene on a full-capability device', () => {
        assert.equal(wallpaperRenderability(metaOf({}), fullCaps), 'ok')
    })

    it('falls back for scene without hardware WebGL2', () => {
        assert.equal(wallpaperRenderability(metaOf({}), noGpuCaps), 'weak-gpu')
    })

    it('flags ETC2 textures only when the device lacks the constants', () => {
        const etc2 = metaOf({ texAudit: { etc2: true } as never })
        assert.equal(wallpaperRenderability(etc2, dx11Caps), 'weak-gpu')
        // 同一张包换到支持 ETC2 的机器上就不该报警 —— 这是「包」与「机器」两件事。
        assert.equal(wallpaperRenderability(etc2, fullCaps), 'ok')
    })

    it('flags BPTC the same way', () => {
        const bptc = metaOf({ texAudit: { bptc: true } as never })
        assert.equal(wallpaperRenderability(bptc, dx11Caps), 'weak-gpu')
        assert.equal(wallpaperRenderability(bptc, fullCaps), 'ok')
    })

    it('treats video as renderable without WebGL2 (native <video>)', () => {
        assert.equal(wallpaperRenderability(metaOf({ type: 'video' }), noGpuCaps), 'ok')
    })

    it('reports unsupported types regardless of device', () => {
        for (const type of ['web', 'application', 'other'] as const) {
            assert.equal(wallpaperRenderability(metaOf({ type }), fullCaps), 'unsupported')
        }
    })
})

/* ------------------------------------------------------------------ *
 * 展示与默认值
 * ------------------------------------------------------------------ */

describe('wallpaper formatting', () => {
    it('formats byte counts with the same units the cards show', () => {
        assert.equal(formatWallpaperBytes(0), '0 KB')
        assert.equal(formatWallpaperBytes(512), '512 B')
        assert.equal(formatWallpaperBytes(1536), '1.5 KB')
        assert.equal(formatWallpaperBytes(22.5 * 1024 * 1024), '22.5 MB')
        assert.equal(formatWallpaperBytes(Number.NaN), '0 KB')
    })

    it('defaults match the agreed budget (30fps / medium / AA off / 0.8)', () => {
        const config = defaultWallpaperConfig()
        assert.deepEqual(config.quality, {
            fps: 30,
            postProcessing: 'medium',
            antialiasing: false,
            textureScale: 0.8,
        })
        assert.equal(config.selected, null)
    })
})
