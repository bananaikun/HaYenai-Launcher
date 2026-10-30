const trimTrailingSlash = (url: string) => url.replace(/\/$/, '')

/**
 * Single source of truth for every user-visible piece of branding.
 *
 * Nothing outside this object may hardcode the product name, the project URLs,
 * the deep-link scheme or the update-service host: build-time guards
 * (`scripts/axolotl/brand-guard.mjs`) check that this stays the only place.
 */
export const BrandConfig = Object.freeze({
    productName: 'HaYenai',
    shortProductName: 'HaYenai',
    /** HaYenai has no separate marketing site; the repository is the site. */
    website: 'https://github.com/Bananaikun/HaYenai',
    repositoryUrl: 'https://github.com/Bananaikun/HaYenai',
    supportUrl: 'https://github.com/Bananaikun/HaYenai/issues',
    /** Upstream project this launcher is a modified version of (GPL-3.0). */
    upstreamName: 'Axolotl Launcher',
    upstreamUrl: 'https://github.com/Mystic-Stars/Axolotl',
    qqGroupNumber: '737601250',
    /** Empty on purpose: the UI must not render an empty anchor. */
    qqChannelUrl: '',
    /** Empty on purpose: the UI must not render an empty anchor. */
    sponsorUrl: '',
    bundleIdentifier: 'com.hayenai.launcher',
    deepLinkScheme: 'hayenai',
    /**
     * Update service fallback base. The Rust side resolves the real host and
     * port from SRV at runtime (`theseus::update_service`); this constant is
     * only what the browser-side code falls back to when the plugin is absent.
     */
    updateServiceFallbackBase: 'https://github.com/bananaikun/HaYenai/releases/',
    /** 隐私政策（随仓库发布；后续可换成更新服务上的 /privacy 页面） */
    privacyUrl: 'https://github.com/Bananaikun/HaYenai/blob/main/docs/PRIVACY.md',
    userAgent: (version: string, os: string) => `hayenai/${version} (${os})`,
    capabilities: Object.freeze({
        publicModrinthApi: true,
        privateModrinthServices: false,
        ghsTelemetry: false,
    }),
})

/**
 * Joins a path onto a base URL while keeping exactly one slash between them.
 * @param base Base URL, with or without a trailing slash.
 * @param path Relative path, with or without a leading slash.
 */
export function joinUrlPath(base: string, path: string): string {
    return `${base.replace(/\/+$/, '')}/${path.replace(/^\/+/, '')}`
}

const siteUrl = trimTrailingSlash(import.meta.env.MODRINTH_URL || 'https://modrinth.com')
const officialLabrinthBaseUrl = trimTrailingSlash(
    import.meta.env.MODRINTH_API_BASE_URL || 'https://api.modrinth.com',
)
type DownloadSourceMode = 'auto' | 'official_only' | 'mirror_preferred' | 'official_preferred'

// The Modrinth API always uses the official source; Modrinth download mirror
// routing is handled by the Rust download layer.
export function setModrinthSourceMode(_sourceMode: DownloadSourceMode) {}

export function setModrinthMirrorEnabled(_enabled: boolean) {}

export function getOfficialLabrinthBaseUrl() {
    return officialLabrinthBaseUrl
}

export function getLabrinthBaseUrl() {
    return officialLabrinthBaseUrl
}

export const config = {
    siteUrl,
    labrinthBaseUrl: getLabrinthBaseUrl,
}
