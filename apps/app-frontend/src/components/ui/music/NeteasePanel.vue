<!--
  网易云面板（音乐页内的登录 + 我的歌单）
  ------------------------------------------------------------
  数据层在 `helpers/music/netease.ts`（请求经 Rust 命令 `netease_request` 转发）。

  登录方式（2026-11-01 改）：**拉官网的登录页**，在子窗口里用网易云 App 扫码 ——
  扫的是官网自己的码、走官网自己的会话，不存在「模拟客户端」那一层风控
  （自己调 `/api/login/qrcode/*` 实测会被 8821 拒）。
  登录成功后从 WebView 的 cookie 存储里读 `MUSIC_U`（HttpOnly，JS 读不到），
  再用它换账号信息并落盘。

  「粘贴 Cookie」保留：官网窗口万一加载不出来（断网 / CDN 抽风），这是最后一条稳路。
-->
<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import {
    loginState,
    loginWithCookie,
    logoutNetease,
    myPlaylists,
    playlistSongs,
    type NeteaseLoginState,
    type NeteasePlaylist,
    type NeteaseSong,
} from '@/helpers/music/netease'

const emit = defineEmits<{
    close: []
    /** 选了一个歌单：父组件负责换成播放队列 */
    pick: [payload: { playlist: NeteasePlaylist; songs: NeteaseSong[] }]
}>()

useVIntl()

type Channel = 'official' | 'cookie'
/** 官网登录的轮询状态 */
type OfficialStatus = 'idle' | 'opening' | 'waiting' | 'checking' | 'timeout'

const OFFICIAL_POLL_MS = 1500
/** 官网窗口最多等 5 分钟，之后停止轮询（窗口留着，用户还能继续操作） */
const OFFICIAL_TIMEOUT_MS = 5 * 60 * 1000

const state = ref<NeteaseLoginState>(loginState())
const channel = ref<Channel>('official')
const official = ref<OfficialStatus>('idle')
const cookieInput = ref('')
const busy = ref(false)
const error = ref('')
const playlists = ref<NeteasePlaylist[]>([])
const loadingPlaylists = ref(false)
const pickingId = ref<number | null>(null)

let pollTimer: number | null = null
let pollStartedAt = 0

const loggedIn = computed(() => state.value.loggedIn)

function stopPoll() {
    if (pollTimer !== null) {
        window.clearInterval(pollTimer)
        pollTimer = null
    }
}

/**
 * 轮询官网窗口的 cookie：出现 `MUSIC_U=` 就说明登录完成。
 * 单次失败不打断轮询（窗口可能还在加载、cookie 还没落地）。
 */
async function pollCookieOnce() {
    if (official.value !== 'waiting' && official.value !== 'checking') return
    if (Date.now() - pollStartedAt > OFFICIAL_TIMEOUT_MS) {
        stopPoll()
        official.value = 'timeout'
        return
    }
    try {
        const cookie = await invoke<string | null>('netease_read_cookie')
        if (!cookie || !/(^|;\s*)MUSIC_U=/.test(cookie)) return

        official.value = 'checking'
        stopPoll()
        // 拿 cookie 换账号信息（顺带验证它有效），通过才落盘
        state.value = await loginWithCookie(cookie)
        void invoke('netease_close_login').catch(() => undefined)
        official.value = 'idle'
        await loadPlaylists()
    } catch (e) {
        error.value = (e as Error).message || '读取登录状态失败'
    }
}

async function openOfficialLogin() {
    error.value = ''
    official.value = 'opening'
    try {
        await invoke('netease_open_login')
        official.value = 'waiting'
        pollStartedAt = Date.now()
        stopPoll()
        pollTimer = window.setInterval(() => void pollCookieOnce(), OFFICIAL_POLL_MS)
    } catch (e) {
        official.value = 'idle'
        error.value = (e as Error).message || '打开登录窗口失败'
    }
}

async function submitCookie() {
    if (busy.value || !cookieInput.value.trim()) return
    busy.value = true
    error.value = ''
    try {
        state.value = await loginWithCookie(cookieInput.value)
        cookieInput.value = ''
        await loadPlaylists()
    } catch (e) {
        error.value = (e as Error).message || '登录失败'
    } finally {
        busy.value = false
    }
}

async function loadPlaylists() {
    if (!loggedIn.value) return
    loadingPlaylists.value = true
    error.value = ''
    try {
        playlists.value = await myPlaylists()
    } catch (e) {
        error.value = (e as Error).message || '拉取歌单失败'
    } finally {
        loadingPlaylists.value = false
    }
}

async function pickPlaylist(playlist: NeteasePlaylist) {
    if (pickingId.value !== null) return
    pickingId.value = playlist.id
    error.value = ''
    try {
        const detail = await playlistSongs(playlist.id)
        emit('pick', { playlist, songs: detail.songs })
    } catch (e) {
        error.value = (e as Error).message || '拉取歌单曲目失败'
    } finally {
        pickingId.value = null
    }
}

function signOut() {
    state.value = logoutNetease()
    playlists.value = []
    official.value = 'idle'
}

onMounted(() => {
    if (loggedIn.value) void loadPlaylists()
})

onBeforeUnmount(() => {
    // 面板关了不关官网窗口：用户可能正在扫码，只是不能再自动检测
    stopPoll()
})
</script>

<template>
    <section class="net">
        <header class="net-head">
            <span class="net-eyebrow">NETEASE CLOUD MUSIC</span>
            <span class="net-grow" />
            <button type="button" class="net-icon" aria-label="关闭" @click="emit('close')">
                ✕
            </button>
        </header>

        <p v-if="error" class="net-error">{{ error }}</p>

        <template v-if="!loggedIn">
            <div class="net-seg">
                <button
                    type="button"
                    :class="{ on: channel === 'official' }"
                    @click="channel = 'official'"
                >
                    官网登录
                </button>
                <button
                    type="button"
                    :class="{ on: channel === 'cookie' }"
                    @click="channel = 'cookie'"
                >
                    粘贴 Cookie
                </button>
            </div>

            <div v-if="channel === 'official'" class="net-official">
                <p class="net-line">
                    {{
                        official === 'waiting' || official === 'checking'
                            ? '官网登录窗口已打开，用网易云音乐 App 扫码，登录成功会自动带回来'
                            : official === 'timeout'
                              ? '等了 5 分钟还没登录；窗口仍开着，登录后点「继续检测」'
                              : '打开网易云官网登录窗口，扫码登录后 cookie 会自动读回来'
                    }}
                </p>
                <p class="net-hint">
                    用的是网易云自己的登录页与二维码，不经过任何第三方；只读取
                    <code>MUSIC_U</code> 这类登录凭证，存在本机。
                </p>
                <button type="button" class="net-btn" @click="openOfficialLogin">
                    {{
                        official === 'waiting' || official === 'checking'
                            ? '重新打开登录窗口'
                            : official === 'timeout'
                              ? '继续检测'
                              : '打开官网登录'
                    }}
                </button>
            </div>

            <div v-else class="net-cookie">
                <p class="net-hint">
                    在浏览器打开 music.163.com 并登录 → F12 → Network 任意请求 →
                    复制请求头里的整条 <code>Cookie</code>（必须含 <code>MUSIC_U=</code>）。
                </p>
                <textarea
                    v-model="cookieInput"
                    class="net-textarea"
                    rows="4"
                    spellcheck="false"
                    placeholder="MUSIC_U=xxxx; __csrf=xxxx; ..."
                />
                <button
                    type="button"
                    class="net-btn"
                    :disabled="busy || !cookieInput.trim()"
                    @click="submitCookie"
                >
                    用这条 Cookie 登录
                </button>
            </div>
        </template>

        <template v-else>
            <div class="net-account">
                <img v-if="state.avatarUrl" class="net-avatar" :src="state.avatarUrl" alt="" />
                <div class="net-account-text">
                    <strong>{{ state.nickname || '已登录' }}</strong>
                    <span class="net-hint">UID {{ state.uid ?? '—' }}</span>
                </div>
                <button type="button" class="net-btn net-btn--ghost" @click="signOut">退出</button>
            </div>

            <p v-if="loadingPlaylists" class="net-line">正在拉取我的歌单…</p>
            <div v-else class="net-list">
                <button
                    v-for="playlist in playlists"
                    :key="playlist.id"
                    type="button"
                    class="net-item"
                    :disabled="pickingId !== null"
                    @click="pickPlaylist(playlist)"
                >
                    <img v-if="playlist.cover" class="net-item-cover" :src="playlist.cover" alt="" />
                    <span v-else class="net-item-cover net-item-cover--empty">♪</span>
                    <span class="net-item-text">
                        <span class="net-item-name">{{ playlist.name }}</span>
                        <span class="net-hint">{{ playlist.trackCount }} 首</span>
                    </span>
                    <span v-if="pickingId === playlist.id" class="net-hint">载入中…</span>
                </button>
                <p v-if="!playlists.length" class="net-line">这个账号下没有可见的歌单。</p>
            </div>
        </template>
    </section>
</template>

<style scoped>
.net {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 18px 18px;
    /* 与音乐页其它面板同材质：极淡白 + 发丝描边，不额外压暗 */
    background-color: color-mix(in srgb, #0a0b10 var(--ui-dim, 25%), transparent);
    background-image: linear-gradient(180deg, var(--glass-2) 0%, var(--glass-1) 100%);
    border: 1px solid var(--stroke-1);
    border-radius: var(--radius-lg);
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(var(--ui-saturate, 105%));
}

.net-head {
    display: flex;
    align-items: center;
    gap: 10px;
}

.net-eyebrow {
    font-size: 10px;
    letter-spacing: 0.16em;
    color: var(--text-3);
}

.net-grow {
    flex: 1;
}

.net-icon {
    border: 0;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    font-size: 13px;
    padding: 4px 6px;
    border-radius: 8px;
}

.net-icon:hover {
    background: var(--glass-2);
    color: var(--text-0);
}

.net-seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    border-radius: 999px;
    background: var(--glass-1);
    border: 1px solid var(--stroke-1);
    align-self: flex-start;
}

.net-seg button {
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: 12px;
    padding: 5px 14px;
    border-radius: 999px;
    cursor: pointer;
}

.net-seg button.on {
    background: var(--glass-3, var(--glass-2));
    color: var(--text-0);
}

.net-official {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 560px;
}

.net-line {
    margin: 0;
    font-size: 13px;
    color: var(--text-1);
}

.net-hint {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-3);
}

.net-hint code {
    font-family: var(--mono-font, monospace);
    font-size: 11px;
    padding: 0 3px;
    border-radius: 4px;
    background: var(--glass-2);
}

.net-error {
    margin: 0;
    font-size: 12px;
    color: var(--color-red, #ff6b6b);
}

.net-btn {
    align-self: flex-start;
    border: 1px solid var(--stroke-1);
    background: var(--glass-2);
    color: var(--text-0);
    font-size: 12px;
    padding: 7px 14px;
    border-radius: 10px;
    cursor: pointer;
}

.net-btn:disabled {
    opacity: 0.5;
    cursor: default;
}

.net-btn--ghost {
    background: transparent;
}

.net-cookie {
    display: flex;
    flex-direction: column;
    gap: 10px;
}

.net-textarea {
    width: 100%;
    resize: vertical;
    border: 1px solid var(--stroke-1);
    border-radius: 10px;
    background: var(--glass-1);
    color: var(--text-0);
    font-family: var(--mono-font, monospace);
    font-size: 12px;
    padding: 10px;
}

.net-account {
    display: flex;
    align-items: center;
    gap: 12px;
}

.net-avatar {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    object-fit: cover;
    border: 1px solid var(--stroke-1);
}

.net-account-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
}

.net-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 8px;
    max-height: 320px;
    overflow-y: auto;
}

.net-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    border: 1px solid var(--stroke-1);
    border-radius: 12px;
    background: var(--glass-1);
    color: inherit;
    text-align: left;
    cursor: pointer;
}

.net-item:hover {
    background: var(--glass-2);
}

.net-item-cover {
    width: 40px;
    height: 40px;
    border-radius: 9px;
    object-fit: cover;
    flex: none;
}

.net-item-cover--empty {
    display: grid;
    place-items: center;
    background: var(--glass-2);
    color: var(--text-3);
}

.net-item-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
}

.net-item-name {
    font-size: 13px;
    color: var(--text-0);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
</style>
