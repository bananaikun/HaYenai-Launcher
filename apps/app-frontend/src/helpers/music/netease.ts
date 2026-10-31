/**
 * 网易云客户端（应用内自建，B 方案）
 * ------------------------------------------------------------
 * 数据全部直连 `music.163.com`，走 Tauri 的 `plugin-http`（Rust 侧发请求）：
 * webview 自己的 fetch 会被 CORS 与 CSP 拦，而且读不到 `Set-Cookie`，登录态根本拿不到。
 *
 * 接口与风控结论照抄后台已验证的实现（`HaYenai-服务器端/lib/netease.cjs`），
 * 那边踩过的坑这里一条都没重新踩：
 *   · 账号密码登录（`/api/login/cellphone`、`/api/login`）返回 `401 无权限访问. ENC`
 *     —— 网易要 weapi/eapi 的 AES+RSA 链，还得过短信二次验证。所以只做**扫码**。
 *   · `/api/login/qrcode/*` 有单独风控：`Referer` 必须是 `…/login`、UA 必须是移动端，
 *     否则手机扫了码轮询永远停在 801，最后 8821。见 `QR_HEADERS`。
 *   · 签发 unikey 那次的会话 cookie 要带进轮询，否则同样一直 801。
 *   · 网关会把「接口不存在」包成 200 + `code: 404`，必须按 code 判断。
 *   · 扫码整体已被网易收紧（8821 = 要行为验证码），所以界面**必须**保留
 *     「粘贴 Cookie」这条稳的路，不能只有扫码。
 *
 * cookie 只落 localStorage（与启动器其他本地状态同处），不进日志、不进 IPC 明文。
 */

import { fetch as tauriFetch } from '@tauri-apps/plugin-http'

const BASE = 'https://music.163.com'
const TIMEOUT_MS = 15_000

const DESKTOP_UA =
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36'
const MOBILE_UA =
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1'

const headersFor = (cookie?: string): Record<string, string> => ({
    'User-Agent': DESKTOP_UA,
    Referer: `${BASE}/`,
    Origin: BASE,
    Accept: 'application/json, text/plain, */*',
    ...(cookie ? { Cookie: cookie } : {}),
})

/** 扫码接口专用：移动端 UA + `/login` 的 Referer（见文件头「风控」） */
const QR_HEADERS: Record<string, string> = {
    'User-Agent': MOBILE_UA,
    Referer: `${BASE}/login`,
    Origin: BASE,
    Accept: 'application/json, text/plain, */*',
}

export const NETESE_SESSION_KEY = 'hayenai:netease-session'

export type NeteaseSession = {
    cookie: string
    uid: number | null
    nickname: string
    avatarUrl: string
    vipType: number
    loginAt: number
    expireAt: number
}

export type NeteaseLoginState = {
    loggedIn: boolean
    expired: boolean
    uid?: number | null
    nickname?: string
    avatarUrl?: string
    vipType?: number
    loginAt?: number
    expireAt?: number
}

export type NeteasePlaylist = {
    id: number
    name: string
    cover: string
    trackCount: number
    /** 5 = 我喜欢的音乐（官方固定标记） */
    specialType: number
    creator: string
    subscribed: boolean
}

export type NeteaseSong = {
    id: number
    name: string
    artist: string
    cover: string
    album: string
    /** 0 免费 / 1 VIP / 4 需购专辑 / 8 低音质免费 */
    fee: number
    durationMs: number
}

/* ---------------- 会话 ---------------- */

let session: NeteaseSession | null = null
let loaded = false

function loadSession(): NeteaseSession | null {
    if (loaded) return session
    loaded = true
    try {
        const raw = localStorage.getItem(NETESE_SESSION_KEY)
        if (!raw) return null
        const parsed = JSON.parse(raw) as Partial<NeteaseSession>
        session = typeof parsed.cookie === 'string' && parsed.cookie ? (parsed as NeteaseSession) : null
    } catch {
        session = null
    }
    return session
}

function persist(next: NeteaseSession | null): void {
    session = next
    loaded = true
    try {
        if (next) localStorage.setItem(NETESE_SESSION_KEY, JSON.stringify(next))
        else localStorage.removeItem(NETESE_SESSION_KEY)
    } catch {
        // 写不进去只影响下次启动，本次会话照常
    }
}

/** 给界面看的脱敏视图：永远不含 cookie */
export function loginState(): NeteaseLoginState {
    const s = loadSession()
    if (!s) return { loggedIn: false, expired: false }
    const expired = !!s.expireAt && Date.now() > s.expireAt
    return {
        loggedIn: !expired,
        expired,
        uid: s.uid,
        nickname: s.nickname,
        avatarUrl: s.avatarUrl,
        vipType: s.vipType,
        loginAt: s.loginAt,
        expireAt: s.expireAt,
    }
}

/** 把 Set-Cookie / cookie 字段规整成 `k=v; k=v`（丢掉 Expires/Path 这些属性） */
export function normalizeCookie(raw: string): string {
    const out = new Map<string, string>()
    for (const part of String(raw ?? '').split(/[;\n]/)) {
        const s = part.trim()
        if (!s) continue
        const eq = s.indexOf('=')
        if (eq <= 0) continue
        const k = s.slice(0, eq).trim()
        const v = s.slice(eq + 1).trim()
        if (/^(expires|max-age|path|domain|secure|httponly|samesite|version|comment)$/i.test(k)) continue
        if (!v) continue
        out.set(k, v)
    }
    return [...out.entries()].map(([k, v]) => `${k}=${v}`).join('; ')
}

/* ---------------- 请求基础 ---------------- */

type CallResult = { status: number; text: string; setCookie: string }

async function raw(
    pathname: string,
    options: { method?: string; body?: string; cookie?: string; headers?: Record<string, string> } = {},
): Promise<CallResult> {
    const controller = new AbortController()
    const timer = window.setTimeout(() => controller.abort(), TIMEOUT_MS)
    try {
        const res = await tauriFetch(BASE + pathname, {
            method: options.method ?? 'GET',
            headers: {
                ...headersFor(options.cookie),
                ...(options.headers ?? {}),
                ...(options.body ? { 'Content-Type': 'application/x-www-form-urlencoded' } : {}),
            },
            body: options.body,
            signal: controller.signal,
        })
        const text = await res.text()
        return { status: res.status, text, setCookie: readSetCookie(res) }
    } catch (error) {
        if ((error as Error)?.name === 'AbortError') throw new Error('请求网易云超时（15s）')
        throw new Error(`请求网易云失败：${(error as Error)?.message ?? '未知原因'}`)
    } finally {
        window.clearTimeout(timer)
    }
}

/**
 * 读 `Set-Cookie`。plugin-http 把 Rust 侧的响应头原样带回来，
 * 但不同版本对多值头的合并方式不一样，所以两种形态都兜一下。
 */
function readSetCookie(res: Response): string {
    const headers = res.headers as Headers & { getSetCookie?: () => string[] }
    if (typeof headers.getSetCookie === 'function') {
        const list = headers.getSetCookie()
        if (list?.length) return list.join('; ')
    }
    const flat = headers.get('set-cookie')
    if (!flat) return ''
    // `Expires=Wed, 21 Oct 2026 …` 里的逗号会把多个 cookie 粘在一起，这里按 `, k=` 拆
    return flat
}

async function callJson<T = Record<string, unknown>>(
    pathname: string,
    options: { method?: string; body?: string; cookie?: string; headers?: Record<string, string> } = {},
): Promise<T> {
    const { status, text } = await raw(pathname, options)
    let json: unknown
    try {
        json = JSON.parse(text)
    } catch {
        throw new Error(`网易云返回了非 JSON 内容（HTTP ${status}）`)
    }
    const payload = json as { code?: number; message?: string }
    // 网关把「接口不存在」包成 200 + code 404 —— 只看 HTTP 状态会误判为成功
    if (payload.code === 404) throw new Error(`网易云接口不存在（${pathname.split('?')[0]}），官方可能已调整`)
    return json as T
}

/* ---------------- 公开元数据（无需登录） ---------------- */

export async function playlistDetail(id: string | number): Promise<{
    id: number
    name: string
    cover: string
    creator: { uid: number; nickname: string } | null
    trackCount: number
    trackIds: number[]
}> {
    const pid = String(id ?? '').trim()
    if (!/^\d+$/.test(pid)) throw new Error(`歌单 ID 必须是纯数字：${pid || '(空)'}`)
    const json = await callJson<{ playlist?: Record<string, any> }>(`/api/v6/playlist/detail?id=${pid}&n=1000&s=8`)
    const p = json.playlist
    if (!p) throw new Error(`歌单不存在或被设为私密（id=${pid}）`)
    return {
        id: p.id,
        name: p.name || '未命名歌单',
        cover: p.coverImgUrl || '',
        creator: p.creator ? { uid: p.creator.userId, nickname: p.creator.nickname } : null,
        trackCount: p.trackCount ?? (p.trackIds || []).length,
        trackIds: (p.trackIds || []).map((t: { id: number }) => t.id),
    }
}

export async function songsDetail(ids: number[]): Promise<NeteaseSong[]> {
    const list = (ids || []).map((i) => ({ id: Number(i) })).filter((x) => Number.isFinite(x.id))
    if (!list.length) return []
    const json = await callJson<{ songs?: Record<string, any>[] }>(
        `/api/v3/song/detail?c=${encodeURIComponent(JSON.stringify(list))}`,
    )
    return (json.songs || []).map((s) => ({
        id: s.id,
        name: s.name || '未知曲目',
        artist: (s.ar || s.artists || []).map((a: { name: string }) => a.name).join(' / ') || '未知歌手',
        cover: s.al?.picUrl || s.album?.picUrl || s.al?.picUrl || '',
        album: s.al?.name || s.album?.name || '',
        fee: s.fee ?? 0,
        durationMs: s.dt || 0,
    }))
}

/** 歌单元数据 + 全部曲目（每 100 首一批） */
export async function playlistSongs(id: string | number): Promise<{
    id: number
    name: string
    cover: string
    creator: { uid: number; nickname: string } | null
    trackCount: number
    songs: NeteaseSong[]
}> {
    const meta = await playlistDetail(id)
    const ids = meta.trackIds.slice(0, 1000)
    const songs: NeteaseSong[] = []
    for (let i = 0; i < ids.length; i += 100) {
        songs.push(...(await songsDetail(ids.slice(i, i + 100))))
    }
    return { ...meta, songs }
}

/* ---------------- 账号 / 登录 ---------------- */

async function account(cookie: string): Promise<{
    uid: number
    nickname: string
    avatarUrl: string
    vipType: number
} | null> {
    const json = await callJson<{ code?: number; message?: string; profile?: Record<string, any> }>(
        '/api/nuser/account/get',
        { cookie },
    )
    if (json.code !== 200) throw new Error(json.message || '会话已失效（账号接口返回异常）')
    const p = json.profile
    if (!p) return null
    return {
        uid: p.userId,
        nickname: p.nickname || '',
        avatarUrl: p.avatarUrl || '',
        vipType: p.vipType ?? 0,
    }
}

/** 用 cookie 登录：交给官方接口验证，通过才落盘 */
export async function loginWithCookie(cookie: string): Promise<NeteaseLoginState> {
    const normalized = normalizeCookie(cookie)
    if (!normalized) throw new Error('cookie 为空')
    if (!/(^|;\s*)MUSIC_U=/.test(normalized)) {
        throw new Error(
            'cookie 里没有 MUSIC_U，这不是网易云的登录凭证。请在 music.163.com 登录后，从开发者工具里复制完整 Cookie。',
        )
    }
    const me = await account(normalized)
    if (!me) throw new Error('cookie 无效或已过期，请重新获取')
    persist({
        cookie: normalized,
        uid: me.uid,
        nickname: me.nickname,
        avatarUrl: me.avatarUrl,
        vipType: me.vipType,
        loginAt: Date.now(),
        // 官方不给确切有效期，按 180 天估；到期只提示重新登录，不影响已拉到的数据
        expireAt: Date.now() + 180 * 24 * 3600 * 1000,
    })
    return loginState()
}

export function logoutNetease(): NeteaseLoginState {
    persist(null)
    qrSessionCookie = ''
    return loginState()
}

/** 启动时自检：会话还在就刷新昵称/头像，失效就标记过期（不弹错） */
export async function verifySession(): Promise<NeteaseLoginState> {
    const s = loadSession()
    if (!s) return loginState()
    try {
        const me = await account(s.cookie)
        if (!me) throw new Error('no profile')
        persist({ ...s, ...me, uid: me.uid })
    } catch {
        persist({ ...s, expireAt: 0 })
    }
    return loginState()
}

/** 我的歌单（含「我喜欢的音乐」） */
export async function myPlaylists(): Promise<NeteasePlaylist[]> {
    const s = loadSession()
    if (!s) throw new Error('尚未登录网易云')
    const all: Record<string, any>[] = []
    for (let offset = 0; offset < 1000; offset += 100) {
        const json = await callJson<{
            code?: number
            message?: string
            more?: boolean
            playlist?: Record<string, any>[]
        }>(`/api/user/playlist?uid=${s.uid}&limit=100&offset=${offset}`, { cookie: s.cookie })
        if (json.code !== 200) {
            if (json.code === 301) {
                persist({ ...s, expireAt: 0 })
                throw new Error('登录已失效，请重新扫码或粘贴 Cookie')
            }
            throw new Error(json.message || '拉取歌单失败')
        }
        const batch = json.playlist || []
        all.push(...batch)
        if (!json.more || !batch.length) break
    }
    return all.map((p) => ({
        id: p.id,
        name: p.name || '未命名歌单',
        cover: p.coverImgUrl || '',
        trackCount: p.trackCount ?? 0,
        specialType: p.specialType ?? 0,
        creator: p.creator ? p.creator.nickname : '',
        subscribed: !!p.subscribed,
    }))
}

/* ---------------- 扫码登录 ---------------- */

/** 二维码里编码的内容：官方协议规定为登录页 + codekey */
const QR_CONTENT = (key: string) => `${BASE}/login?codekey=${key}`
export const QR_TTL_SEC = 180

/**
 * 签发 unikey 时带回来的会话 cookie。
 * 轮询必须**同一个会话**，否则服务端把 unikey 当成「未签发」，永远停在 801
 * （表现：手机明明扫了也确认了，界面一直说「请扫描二维码」）。
 */
let qrSessionCookie = ''

export async function qrStart(): Promise<{ key: string; url: string; expireIn: number }> {
    const res = await raw(`/api/login/qrcode/unikey?type=1&timestamp=${Date.now()}`, { headers: QR_HEADERS })
    let json: { code?: number; message?: string; unikey?: string }
    try {
        json = JSON.parse(res.text)
    } catch {
        throw new Error('网易云返回了非 JSON 内容，请稍后重试')
    }
    if (json.code !== 200 || !json.unikey) throw new Error(json.message || '生成二维码失败，请稍后重试')
    const key = String(json.unikey)
    qrSessionCookie = normalizeCookie(res.setCookie)
    return { key, url: QR_CONTENT(key), expireIn: QR_TTL_SEC }
}

export type QrPollResult =
    | { status: 'waiting' }
    | { status: 'scanned' }
    | { status: 'expired' }
    | { status: 'ok'; session: NeteaseLoginState }
    | { status: 'error'; code: number; message: string }

export async function qrPoll(key: string): Promise<QrPollResult> {
    const k = String(key ?? '').trim()
    if (!k) throw new Error('缺少二维码 key')

    const res = await raw(`/api/login/qrcode/client/login?key=${encodeURIComponent(k)}&type=1&timestamp=${Date.now()}`, {
        cookie: qrSessionCookie,
        headers: QR_HEADERS,
    })
    let json: { code?: number; message?: string; cookie?: string }
    try {
        json = JSON.parse(res.text)
    } catch {
        throw new Error('轮询返回了非 JSON 内容，请重新获取二维码')
    }

    const code = json.code
    if (code === 801) return { status: 'waiting' }
    if (code === 802) return { status: 'scanned' }
    if (code === 800) return { status: 'expired' }

    if (code === 803) {
        const cookie = normalizeCookie(json.cookie || res.setCookie || '')
        if (!/(^|;\s*)MUSIC_U=/.test(cookie)) {
            // 「扫上了」和「拿到凭证」是两件事，文案必须分开，否则用户反复重扫
            throw new Error('扫码已确认，但没拿到登录凭证（MUSIC_U）。请改用「粘贴 Cookie」登录。')
        }
        return { status: 'ok', session: await loginWithCookie(cookie) }
    }

    // 未知 code 如实交回界面（8821 = 网易要求行为验证码），不要吞成一句笼统错误
    return { status: 'error', code: Number(code) || -1, message: String(json.message || '').slice(0, 120) }
}
