/**
 * 磁悬条 · 音乐状态
 *
 * 数据面照搬后台管理那套：直接打 Meting 兼容音源（`helpers/music/music-source.ts`），
 * 不需要服务端中转，也不占主人服务器带宽。歌词走音源返回的 `lrc` 地址。
 *
 * 为什么是模块单例而不是 provide/inject：
 * 磁悬条只是「一个显示面」，将来可能还有别的面（音乐页、壁纸随节奏）要读同一份状态；
 * 用模块单例就不必让它们共用同一条组件祖先。
 *
 * ⚠ `hookAudio()` 会把 `<audio>` 接到 Web Audio 图上（为了频谱/振幅），
 * 一旦接管就不可逆，且**跨域资源必须带 crossOrigin**，否则静默变哑 —— 见 music-analyser.ts 头注。
 */
import { reactive } from 'vue'

import { hookAudio } from '@/helpers/music/music-analyser'
import { METING_SOURCES, resolveSongs, type RawSong } from '@/helpers/music/music-source'

export type LyricLine = { at: number; text: string }

const SPEC_KEY = 'hayenai:music-spec'
/**
 * 默认歌单 / 默认播放器偏好 —— 主人 2026-10-31 拍板：「默认就按我现在用的这套」。
 * 原来这里是「网易云热歌榜 + 音量 0.8 + 顺序播放 + 不自动播」，现在固化成他实际在用的：
 * 自己的歌单 18376629143（41 首）、音量 1、随机、开机自动播、显示歌词。
 */
const DEFAULT_SPEC = '18376629143'

export type PlayMode = 'loop' | 'single' | 'random'

export const music = reactive({
    songs: [] as RawSong[],
    volume: 1,
    muted: false,
    mode: 'random' as PlayMode,
    /** 功能开关：歌词是否显示、打开启动器是否自动播放 */
    settings: { lyrics: true, autoplay: true } as { lyrics: boolean; autoplay: boolean },
    index: 0,
    /** 当前队列的来源标签（「网易云 · web」/「Meting · 18376629143」）。
        只作展示，不持久化 —— 重启后队列会按 musicSpec 重新拉，标签跟着那次拉的结果走。 */
    queueLabel: '',
    playing: false,
    loading: false,
    error: '',
    currentTime: 0,
    duration: 0,
    lyrics: [] as LyricLine[],
    activeLyric: '',
})

let audio: HTMLAudioElement | null = null
let analyserHooked = false
let lyricAbort: AbortController | null = null

function readSpec(): string {
    try {
        return (localStorage.getItem(SPEC_KEY) || DEFAULT_SPEC).trim() || DEFAULT_SPEC
    } catch {
        return DEFAULT_SPEC
    }
}

export function musicSpec(): string {
    return readSpec()
}

export function setMusicSpec(spec: string): void {
    try {
        localStorage.setItem(SPEC_KEY, spec.trim())
    } catch {
        // 隐私模式下写不进去，本次会话仍可用
    }
}

export function currentSong(): RawSong | undefined {
    return music.songs[music.index]
}

function ensureAudio(): HTMLAudioElement {
    if (audio) return audio
    // 关窗/切后台时补存一次进度（节流会漏掉最后几秒）
    window.addEventListener('pagehide', () => saveProgress(true))
    document.addEventListener('visibilitychange', () => {
        if (document.hidden) saveProgress(true)
    })
    const el = new Audio()
    // 音源与 CDN 都带 CORS 头；不设这一项 Web Audio 会静默输出静音
    el.crossOrigin = 'anonymous'
    el.preload = 'metadata'
    el.addEventListener('timeupdate', onTimeUpdate)
    el.addEventListener('loadedmetadata', () => {
        music.duration = Number.isFinite(el.duration) ? el.duration : 0
    })
    el.addEventListener('ended', () => onEnded())
    el.volume = music.volume
    el.muted = music.muted
    el.addEventListener('play', () => {
        music.playing = true
    })
    el.addEventListener('pause', () => {
        music.playing = false
    })
    el.addEventListener('error', () => {
        // 单曲取流失败就跳过，别把整条播放链卡死
        music.error = '当前歌曲取流失败，已切下一首'
        next()
    })
    audio = el
    return el
}

function onTimeUpdate(): void {
    const el = audio
    if (!el) return
    music.currentTime = el.currentTime
    saveProgress()
    const lines = music.lyrics
    if (!lines.length) return
    // 从后往前找第一个已到时间的行；歌词通常几十行，线性扫也够快
    for (let i = lines.length - 1; i >= 0; i--) {
        if (el.currentTime >= lines[i].at) {
            music.activeLyric = lines[i].text
            return
        }
    }
    music.activeLyric = ''
}

/** 解析 LRC。一行可能带多个时间戳（合并句），全部展开。 */
export function parseLrc(raw: string): LyricLine[] {
    const out: LyricLine[] = []
    const stamp = /\[(\d{1,2}):(\d{2})(?:[.:](\d{1,3}))?\]/g
    for (const line of raw.split(/\r?\n/)) {
        const text = line.replace(stamp, '').trim()
        stamp.lastIndex = 0
        let match: RegExpExecArray | null
        while ((match = stamp.exec(line))) {
            const min = Number(match[1] ?? 0)
            const sec = Number(match[2] ?? 0)
            const fracRaw = match[3] ?? '0'
            const frac = Number(fracRaw.padEnd(3, '0').slice(0, 3)) / 1000
            if (!text) continue
            out.push({ at: min * 60 + sec + frac, text })
        }
    }
    return out.sort((a, b) => a.at - b.at)
}

async function loadLyrics(url: string | undefined): Promise<void> {
    music.lyrics = []
    music.activeLyric = ''
    if (!url) return
    lyricAbort?.abort()
    lyricAbort = new AbortController()
    try {
        const res = await fetch(url, { signal: lyricAbort.signal })
        if (!res.ok) return
        const raw = await res.text()
        music.lyrics = parseLrc(raw)
    } catch (error) {
        if ((error as Error).name !== 'AbortError') {
            // 歌词拿不到不该打断播放：静音失败即可
            music.lyrics = []
        }
    }
}

function applySong(index: number, autoplay: boolean): void {
    const song = music.songs[index]
    if (!song?.url) return
    music.index = index
    music.currentTime = 0
    music.duration = 0
    const el = ensureAudio()
    el.src = song.url
    void loadLyrics(song.lrc)
}

export function playIndex(index: number): void {
    if (!music.songs[index]) return
    applySong(index, true)
    void play()
}

export async function play(): Promise<void> {
    const el = ensureAudio()
    try {
        if (!analyserHooked) {
            // 必须在用户手势里调用：AudioContext 在无手势时是 suspended
            hookAudio(el)
            analyserHooked = true
        }
        await el.play()
        music.playing = true
        music.error = ''
    } catch (error) {
        music.playing = false
        music.error = `播放失败：${(error as Error).message || '未知原因'}`
    }
}

export function pause(): void {
    audio?.pause()
    music.playing = false
}

export function toggle(): void {
    if (music.playing) pause()
    else void play()
}

/** 单曲循环：`<audio loop>` 由我们自己管，因为切歌也要按模式走 */
function onEnded(): void {
    if (music.mode === 'single') {
        const el = ensureAudio()
        el.currentTime = 0
        void el.play()
        return
    }
    if (music.mode === 'random' && music.songs.length > 1) {
        let pick = music.index
        while (pick === music.index) pick = Math.floor(Math.random() * music.songs.length)
        playIndex(pick)
        return
    }
    next()
}

export function cycleMode(): void {
    const order: PlayMode[] = ['loop', 'single', 'random']
    music.mode = order[(order.indexOf(music.mode) + 1) % order.length]
    persistSettings()
}

export function setVolume(value: number): void {
    music.volume = Math.max(0, Math.min(1, value))
    const el = ensureAudio()
    el.volume = music.volume
    if (music.volume > 0 && music.muted) {
        music.muted = false
        el.muted = false
    }
    persistSettings()
}

export function toggleMute(): void {
    music.muted = !music.muted
    ensureAudio().muted = music.muted
    persistSettings()
}

/* ---------------- 播放进度（2026-10-31 主人要求） ----------------
   以前每次开启动器都从歌单第一首 0:00 重新放。现在把「哪首歌 + 播到第几秒」
   存 localStorage，下次进来接着放：
   - 只在 `timeupdate` 里**节流 5 秒**写一次，且 `hidden` / `pagehide` 时立刻补一次
     （节流是为了不每 250ms 写一次 localStorage）；
   - 歌曲用 `id`（缺省 `url`）标识，**不用下标** —— 歌单顺序变了也能对上；
   - 恢复时若那首歌已不在歌单里，退回第一首，不报错。
   契约：恢复「位置」不强制「播放」；播不播由 `settings.autoplay` 决定。 */
const PROGRESS_KEY = 'hayenai:music-progress'
const PROGRESS_THROTTLE_MS = 5000

type SavedProgress = { key: string; position: number; index: number; savedAt: number }

function songKey(song: { id?: number | string; url?: string } | undefined): string {
    if (!song) return ''
    if (song.id !== undefined && song.id !== null && `${song.id}`.length) return `id:${song.id}`
    return song.url ? `url:${song.url}` : ''
}

function readProgress(): SavedProgress | null {
    try {
        const raw = localStorage.getItem(PROGRESS_KEY)
        if (!raw) return null
        const parsed = JSON.parse(raw) as Partial<SavedProgress>
        if (typeof parsed.key !== 'string' || !parsed.key) return null
        const position = Number(parsed.position)
        if (!Number.isFinite(position) || position < 0) return null
        return {
            key: parsed.key,
            position,
            index: Number.isFinite(Number(parsed.index)) ? Number(parsed.index) : 0,
            savedAt: Number.isFinite(Number(parsed.savedAt)) ? Number(parsed.savedAt) : 0,
        }
    } catch {
        return null
    }
}

let lastSavedAt = 0

function saveProgress(force = false): void {
    const el = audio
    const song = music.songs[music.index]
    if (!el || !song) return
    const now = Date.now()
    if (!force && now - lastSavedAt < PROGRESS_THROTTLE_MS) return
    lastSavedAt = now
    try {
        localStorage.setItem(
            PROGRESS_KEY,
            JSON.stringify({
                key: songKey(song),
                position: el.currentTime || 0,
                index: music.index,
                savedAt: now,
            } satisfies SavedProgress),
        )
    } catch {
        // 写不进去就算了，下次还是从头放
    }
}

/** 把上次的进度套到当前音轨上（元数据好了才设，否则 currentTime 会被重置）。 */
function restoreProgress(): boolean {
    const saved = readProgress()
    if (!saved) return false
    const key = songKey(music.songs[music.index])
    if (key !== saved.key) return false
    const el = ensureAudio()
    const apply = () => {
        if (saved.position > 0 && Number.isFinite(el.duration) && el.duration > 0) {
            el.currentTime = Math.min(saved.position, Math.max(0, el.duration - 1))
        }
    }
    if (el.readyState >= 1) apply()
    else el.addEventListener('loadedmetadata', apply, { once: true })
    return true
}

const SETTINGS_KEY = 'hayenai:music-settings'

/** 改功能开关并落盘（设置 → 音乐与歌词） */
export function updateSettings(patch: Partial<{ lyrics: boolean; autoplay: boolean }>): void {
    music.settings = { ...music.settings, ...patch }
    persistSettings()
}

function persistSettings(): void {
    try {
        localStorage.setItem(
            SETTINGS_KEY,
            JSON.stringify({
                volume: music.volume,
                muted: music.muted,
                mode: music.mode,
                settings: music.settings,
            }),
        )
    } catch {
        // 隐私模式：不落盘，本次会话仍生效
    }
}

function restoreSettings(): void {
    try {
        const raw = localStorage.getItem(SETTINGS_KEY)
        if (!raw) return
        const saved = JSON.parse(raw) as {
            volume?: number
            muted?: boolean
            mode?: PlayMode
            settings?: { lyrics?: boolean; autoplay?: boolean }
        }
        if (saved.settings) {
            // 缺字段时回落到**当前默认**（跟着 DEFAULT 走，别再写死第二份）
            music.settings = {
                lyrics: saved.settings.lyrics ?? music.settings.lyrics,
                autoplay: saved.settings.autoplay ?? music.settings.autoplay,
            }
        }
        if (typeof saved.volume === 'number') music.volume = Math.max(0, Math.min(1, saved.volume))
        if (typeof saved.muted === 'boolean') music.muted = saved.muted
        if (saved.mode === 'loop' || saved.mode === 'single' || saved.mode === 'random') {
            music.mode = saved.mode
        }
    } catch {
        // 坏数据就当没存过
    }
}

restoreSettings()

export function next(): void {
    if (!music.songs.length) return
    playIndex((music.index + 1) % music.songs.length)
}

export function prev(): void {
    if (!music.songs.length) return
    playIndex((music.index - 1 + music.songs.length) % music.songs.length)
}

export function seek(seconds: number): void {
    if (!audio) return
    audio.currentTime = Math.max(0, Math.min(seconds, music.duration || seconds))
}

/**
 * 用一份现成的曲目列表替换播放队列（网易云歌单走这条）。
 *
 * 与 `loadPlaylist()` 的区别：那边自己去音源解析 spec，这边是「外面已经拿到
 * 曲目元数据了，只差播放」。所以这里**不再请求任何接口**，只做三件事：
 * 换队列、定位到起始曲、按需起播；进度沿用同一套 `restoreProgress()` 约定
 * （换队列等于换内容，进度按 key 对不上就从头开始）。
 *
 * `url` 为空的曲目直接丢掉 —— 与 `loadPlaylist()` 里 `filter(song => song.url)` 同一口径，
 * 免得播放列表里混进点了没反应的条目。
 */
export function setQueue(next: RawSong[], startIndex = 0, autoplay = true, label = ''): void {
    const list = (next || []).filter((song) => song && song.url)
    if (!list.length) {
        music.error = '这个歌单里没有可播放的曲目'
        return
    }
    music.songs = list
    music.error = ''
    music.queueLabel = label
    const at = Math.max(0, Math.min(list.length - 1, Math.trunc(startIndex) || 0))
    music.index = at
    applySong(at, autoplay)
    if (autoplay) void play()
}

/** 拉歌单。失败时把「哪个源、什么错」原样留给界面，不做二次包装。 */
let autoplayed = false

export async function loadPlaylist(spec = readSpec()): Promise<void> {
    if (music.loading) return
    music.loading = true
    music.error = ''
    try {
        const resolved = await resolveSongs(METING_SOURCES[0], spec)
        music.songs = resolved.songs.filter((song) => song.url)
        music.queueLabel = `Meting · ${spec}`
        music.index = 0
        if (!music.songs.length) {
            music.error = '歌单里没有可播放的曲目'
            return
        }
        // 上次播到哪首/第几秒：能对上就接着放那首，对不上退回第一首
        const saved = readProgress()
        const resumeIndex =
            saved && music.songs.findIndex((song) => songKey(song) === saved.key) >= 0
                ? music.songs.findIndex((song) => songKey(song) === saved.key)
                : 0
        applySong(resumeIndex, false)
        const restored = restoreProgress()
        // 只在第一次成功拉到歌单时自动开播；之后换歌单不会突然出声
        if (!autoplayed && music.settings.autoplay && music.songs.length) {
            autoplayed = true
            void play()
        }
        // `restored` 时进度条已经在 restoreProgress 里摆回上次的位置了（只是不放声）
        void restored
    } catch (error) {
        music.error = (error as Error).message || '取歌单失败'
    } finally {
        music.loading = false
    }
}

export function formatClock(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds <= 0) return '0:00'
    const total = Math.floor(seconds)
    const m = Math.floor(total / 60)
    const s = total % 60
    return `${m}:${s.toString().padStart(2, '0')}`
}
