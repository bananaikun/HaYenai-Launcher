<!--
  网易云面板（音乐页内的登录 + 我的歌单）
  ------------------------------------------------------------
  数据层在 `helpers/music/netease.ts`（走 Tauri plugin-http 直连官方接口）。
  这里只负责两件事：把人登录进来、把一个歌单的曲目交回父组件去播。

  为什么同时给「扫码」和「粘贴 Cookie」两条路：
  网易云已经对扫码整体收紧（8821 = 要求行为验证码），单靠扫码必然会有一部分人
  怎么扫都进不来；粘贴 Cookie 是官方的登录凭证，永远可用。两条都给，不赌。
-->
<script setup lang="ts">
import { useVIntl } from '@modrinth/ui'
import QRCode from 'qrcode'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import {
    loginState,
    loginWithCookie,
    logoutNetease,
    myPlaylists,
    playlistSongs,
    qrPoll,
    qrStart,
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

type Channel = 'qr' | 'cookie'

const state = ref<NeteaseLoginState>(loginState())
const channel = ref<Channel>('qr')
const qrCanvas = ref<HTMLCanvasElement | null>(null)
const qrKey = ref('')
const qrStatus = ref<'idle' | 'loading' | 'waiting' | 'scanned' | 'expired' | 'ok'>('idle')
const qrMessage = ref('')
const cookieInput = ref('')
const busy = ref(false)
const error = ref('')
const playlists = ref<NeteasePlaylist[]>([])
const loadingPlaylists = ref(false)
const pickingId = ref<number | null>(null)

let pollTimer: number | null = null

const loggedIn = computed(() => state.value.loggedIn)

function stopPoll() {
    if (pollTimer !== null) {
        window.clearInterval(pollTimer)
        pollTimer = null
    }
}

async function drawQr(url: string) {
    const canvas = qrCanvas.value
    if (!canvas) return
    await QRCode.toCanvas(canvas, url, {
        width: 176,
        margin: 1,
        errorCorrectionLevel: 'M',
        color: { dark: '#11131a', light: '#ffffff' },
    })
}

async function refreshQr() {
    if (busy.value) return
    busy.value = true
    error.value = ''
    stopPoll()
    qrStatus.value = 'loading'
    qrMessage.value = ''
    try {
        const { key, url } = await qrStart()
        qrKey.value = key
        qrStatus.value = 'waiting'
        await drawQr(url)
        pollTimer = window.setInterval(() => void pollOnce(), 2000)
    } catch (e) {
        qrStatus.value = 'idle'
        error.value = (e as Error).message || '生成二维码失败'
    } finally {
        busy.value = false
    }
}

async function pollOnce() {
    if (!qrKey.value) return
    try {
        const result = await qrPoll(qrKey.value)
        if (result.status === 'waiting') return
        if (result.status === 'scanned') {
            qrStatus.value = 'scanned'
            qrMessage.value = '已扫码，请在手机上确认登录'
            return
        }
        if (result.status === 'expired') {
            stopPoll()
            qrStatus.value = 'expired'
            qrMessage.value = '二维码已过期，点「刷新」重新获取'
            return
        }
        if (result.status === 'error') {
            stopPoll()
            qrStatus.value = 'expired'
            /* 2026-11-01（实测，见 netease.ts 文件头）：
               8821 = 网易风控拒了这次扫码会话。我们这边有一个结构性劣势 ——
               Tauri 的 http 插件**不把 `Set-Cookie` 交给 JS**（已用 https://music.163.com/
               验证：响应头里没有 set-cookie），所以「签发 unikey 时的会话 cookie」
               带不进轮询，网易就会在确认那一刻拒掉。
               结论：扫码这条路在当前架构下不可靠，直接把用户送到**粘贴 Cookie** 那一页，
               不要让他对着一个永远不会成功的二维码反复扫。 */
            qrMessage.value = result.message
                ? `扫码被网易拒绝（code ${result.code}）：${result.message}`
                : `扫码被网易拒绝（code ${result.code}）`
            channel.value = 'cookie'
            return
        }
        stopPoll()
        qrStatus.value = 'ok'
        state.value = result.session
        await loadPlaylists()
    } catch (e) {
        stopPoll()
        qrStatus.value = 'expired'
        error.value = (e as Error).message || '轮询失败'
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
    qrStatus.value = 'idle'
}

watch(channel, (next) => {
    if (next === 'qr' && qrStatus.value === 'idle') void refreshQr()
})

onMounted(() => {
    if (loggedIn.value) void loadPlaylists()
    else void refreshQr()
})

onBeforeUnmount(stopPoll)
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
                <button type="button" :class="{ on: channel === 'qr' }" @click="channel = 'qr'">
                    扫码登录
                </button>
                <button type="button" :class="{ on: channel === 'cookie' }" @click="channel = 'cookie'">
                    粘贴 Cookie
                </button>
            </div>

            <div v-if="channel === 'qr'" class="net-qr">
                <canvas ref="qrCanvas" class="net-qr-canvas" />
                <div class="net-qr-side">
                    <p class="net-line">
                        {{
                            qrStatus === 'loading'
                                ? '正在生成二维码…'
                                : qrStatus === 'scanned'
                                  ? qrMessage || '已扫码，请在手机上确认'
                                  : qrStatus === 'expired'
                                    ? qrMessage || '二维码已过期'
                                    : qrMessage || '打开网易云音乐 App → 扫一扫'
                        }}
                    </p>
                    <p class="net-hint">
                        扫码可能被网易风控拒绝（8821）；一旦被拒会自动切到「粘贴 Cookie」。
                    </p>
                    <button type="button" class="net-btn" :disabled="busy" @click="refreshQr">
                        刷新二维码
                    </button>
                </div>
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

.net-qr {
    display: flex;
    gap: 18px;
    align-items: flex-start;
}

.net-qr-canvas {
    width: 176px;
    height: 176px;
    border-radius: 12px;
    background: #fff;
    padding: 6px;
}

.net-qr-side {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
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
