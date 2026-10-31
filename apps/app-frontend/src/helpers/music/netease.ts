/**
 * 网易云客户端（应用内自建）
 * ------------------------------------------------------------
 * 所有 API 请求都经 Rust 命令 `netease_request`（`apps/app/src/netease.rs`）转发：
 * webview 自己的 fetch 会被 CORS/CSP 拦，而 `@tauri-apps/plugin-http` **不把
 * `Set-Cookie` 交给 JS**（实测响应头里没有 set-cookie）。走 Rust 就都绕开了。
 *
 * 登录：**不自己调扫码接口**。2026-11-01 起改为打开网易云官网登录页
 * （Rust 命令 `netease_open_login`），用户在官网窗口里扫码，登录成功后从
 * WebView 的 cookie 存储读 `MUSIC_U`（HttpOnly，JS 读不到）——
 * 见 `netease_read_cookie`。自己调 `/api/login/qrcode/*` 会被 8821 拒，别再加回来。
 *
 * 接口结论沿用后台已验证的实现（`HaYenai-服务器端/lib/netease.cjs`）：
 *   · 账号密码登录（`/api/login/cellphone`、`/api/login`）返回 `401 无权限访问. ENC`
 *     —— 网易要 weapi/eapi 的 AES+RSA 链，还得过短信二次验证。
 *   · 网关会把「接口不存在」包成 200 + `code: 404`，必须按 code 判断。
 *   · 「粘贴 Cookie」永远保留：官网窗口加载不出来时这是唯一稳路。
 *
 * cookie 只落 localStorage（与启动器其他本地状态同处），不进日志、不进 IPC 明文。
 */

import { invoke } from '@tauri-apps/api/core'

const BASE = 'https://music.163.com'

const DESKTOP_UA =
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36'

const headersFor = (cookie?: string): Record<string, string> => ({
    'User-Agent': DESKTOP_UA,
    Referer: `${BASE}/`,
    Origin: BASE,
    Accept: 'application/json, text/plain, */*',
    ...(cookie ? { Cookie: cookie } : {}),
})

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

/** Rust 侧 `netease_request` 的返回结构（见 apps/app/src/netease.rs） */
type NeteaseRaw = { status: number; body: string; set_cookie: string[] }

async function raw(
    pathname: string,
    options: { method?: string; body?: string; cookie?: string; headers?: Record<string, string> } = {},
): Promise<CallResult> {
    const headers: [string, string][] = Object.entries({
        ...headersFor(options.cookie),
        ...(options.headers ?? {}),
        ...(options.body ? { 'Content-Type': 'application/x-www-form-urlencoded' } : {}),
    })
    let res: NeteaseRaw
    try {
        res = await invoke<NeteaseRaw>('netease_request', { path: pathname, headers })
    } catch (error) {
        const message = String((error as Error)?.message ?? error)
        if (message.includes('timed out') || message.includes('deadline')) throw new Error('请求网易云超时（20s）')
        throw new Error(`请求网易云失败：${message}`)
    }
    // 多条 Set-Cookie 用 "; " 拼起来交给 normalizeCookie：它会把 Expires/Path 这些
    // 属性剥掉，只留 k=v。不在这里做拼接规则，避免和那边重复实现。
    return { status: res.status, text: res.body, setCookie: (res.set_cookie ?? []).join('; ') }
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

/* ---------------- 收藏 ---------------- */

/**
 * 「我喜欢的音乐」里的曲目 id 集合。
 *
 * 为什么不用 `/api/likelist`：实测返回 **code 404「接口未找到」**（网关没开这个路由）。
 * 但「我喜欢的音乐」是一个普通歌单（`specialType === 5`），`/api/v6/playlist/detail`
 * 能正常读到它的 `trackIds` —— 用它当收藏集，可靠且不需要额外权限。
 */
export async function likedSongIds(): Promise<Set<number>> {
    const lists = await myPlaylists()
    const liked = lists.find((p) => p.specialType === 5)
    if (!liked) return new Set()
    const detail = await playlistDetail(liked.id)
    return new Set(detail.trackIds)
}

/**
 * 收藏 / 取消收藏一首。
 *
 * 实测：`GET /api/song/like?trackId=&like=` 返回 `{ code: 200, playlistId }`。
 * （`/api/radio/like` 会被风控打成 `-460 网络环境存在风险`，别用那个。）
 * 调用方负责先做乐观更新、失败时回滚。
 */
export async function setSongLiked(trackId: number, liked: boolean): Promise<void> {
    const s = loadSession()
    if (!s) throw new Error('尚未登录网易云')
    const json = await callJson<{ code?: number; message?: string }>(
        `/api/song/like?trackId=${Number(trackId)}&like=${liked ? 'true' : 'false'}&timestamp=${Date.now()}`,
        { cookie: s.cookie },
    )
    if (json.code !== 200) throw new Error(json.message || '收藏操作失败')
}

/* ---------------- 扫码相关（已下线） ----------------
   2026-11-01：登录改成「拉官网登录页 → 从 WebView 的 cookie 存储里读 MUSIC_U」
   （Rust 命令 `netease_open_login` / `netease_read_cookie`）。
   自己调 `/api/login/qrcode/*` 那条路即使能拿到 Set-Cookie，手机确认后仍会被 8821 拒，
   代码已删，别再加回来 —— 官网登录页才是官方维护的正常入口。
   ---------------- */
