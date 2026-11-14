import { invoke } from '@tauri-apps/api/core'

export interface RoomConfig {
    id: string
    name: string
    code: string
    hostName: string
    createdAt: number
    settings: RoomSettings
}

export interface RoomSettings {
    plugins: Record<string, boolean>
    bot?: RoomBot | null
    qq?: RoomQq | null
}

export interface RoomBot {
    name: string
    presetId: string
    mentionOnly: boolean
}

export interface RoomQq {
    enabled: boolean
    groupId: string
}

export interface RoomMessage {
    id: number
    from: string
    /** text / file / pack / system */
    kind: string
    text: string
    payload?: Record<string, unknown> | null
    ts: number
}

export interface RoomMember {
    name: string
    accountKind: string
    note: string
    joinedAt: number
    /** 房间内的固定虚拟地址（房主 .1，成员按加入顺序 .2 起）。老数据可能为空。 */
    virtualIp?: string | null
}

export interface HostInfo {
    running: boolean
    roomId: string | null
    addr: string | null
}

export interface PluginInfo {
    id: string
    label: string
}

export const roomList = () => invoke<RoomConfig[]>('room_list')

/** 当前登录的 Minecraft 账号名。房间页用它当昵称，不再让用户手填。 */
export const roomMyName = () => invoke<string | null>('room_my_name')

export const roomCreate = (name: string, hostName: string) =>
    invoke<RoomConfig>('room_create', { name, hostName })

export const roomRemove = (roomId: string) => invoke<void>('room_remove', { roomId })

export const roomMessages = (roomId: string, since: number) =>
    invoke<RoomMessage[]>('room_messages', { roomId, since })

export const roomMembers = (roomId: string) => invoke<RoomMember[]>('room_members', { roomId })

export const roomRemoteMembers = (base: string, code: string) =>
    invoke<RoomMember[]>('room_remote_members', { base, code })

export const roomJoin = (roomId: string, name: string, accountKind: string, note: string) =>
    invoke<RoomMember[]>('room_join', { roomId, name, accountKind, note })

export const roomHostStart = (roomId: string, bind: string, port: number) =>
    invoke<string>('room_host_start', { roomId, bind, port })

export const roomHostStop = () => invoke<void>('room_host_stop')

export const roomHostInfo = () => invoke<HostInfo>('room_host_info')

export const roomPlugins = () => invoke<PluginInfo[]>('room_plugins')

export const roomGetSettings = (roomId: string) =>
    invoke<RoomSettings>('room_get_settings', { roomId })

export const roomSaveSettings = (roomId: string, settings: RoomSettings) =>
    invoke<RoomSettings>('room_save_settings', { roomId, settings })

export const roomSend = (options: {
    roomId: string
    from: string
    text: string
    kind?: string
    payload?: Record<string, unknown> | null
    base?: string | null
    code?: string | null
}) =>
    invoke<void>('room_send', {
        roomId: options.roomId,
        from: options.from,
        text: options.text,
        kind: options.kind ?? 'text',
        payload: options.payload ?? null,
        base: options.base ?? null,
        code: options.code ?? null,
    })

export const roomPull = (
    roomId: string,
    since: number,
    base?: string | null,
    code?: string | null,
) =>
    invoke<RoomMessage[]>('room_pull', {
        roomId,
        since,
        base: base ?? null,
        code: code ?? null,
    })

/** 分享本机文件。房主写本地目录，成员推给房主（base/code 非空即成员身份）。 */
export const roomShareFile = (
    roomId: string,
    path: string,
    from: string,
    base?: string | null,
    code?: string | null,
) =>
    invoke<RoomMessage>('room_share_file', {
        roomId,
        path,
        from,
        base: base ?? null,
        code: code ?? null,
    })

/** 成员把本地文件推到房主服务。 */
export const roomUploadFile = (base: string, code: string, name: string, bytes: number[]) =>
    invoke<unknown>('room_upload_file', { base, code, name, bytes })

export const roomInstanceManifest = (instanceId: string) =>
    invoke<{
        instance: string
        game_version: string
        loaders: string[]
        mods: { file: string; name: string; size: number }[]
        mod_count: number
    }>('room_instance_manifest', { instanceId })

/* ------------------------------------------------------------------ *
 * 组网（EasyTier）
 * ------------------------------------------------------------------ */

export interface LanEnv {
    localIps: string[]
    suggestedCidr: string | null
    isAdmin: boolean
    missingFiles: string[]
    corePath: string | null
}

export interface LanPeer {
    ipv4: string
    hostname: string
    cost: string
    latency: string
    loss: string
    natType: string
    tunnel: string
}

export interface LanStatus {
    running: boolean
    elevated: boolean
    /** 是否常驻托管（退房间不停）。 */
    resident: boolean
    /** 自动重连已用次数。 */
    restarts: number
    /** 自动重连已放弃，需要手动重连。 */
    gaveUp: boolean
    virtualIp: string | null
    hostname: string | null
    publicIp: string | null
    natType: string | null
    peers: LanPeer[]
    error: string | null
}

export interface LanConfig {
    networkName: string
    networkSecret: string
    peers: string[]
    proxyCidr?: string | null
    virtualIp?: string | null
    role: 'host' | 'member'
}

export const lanEnv = () => invoke<LanEnv>('lan_env')
export const lanStart = (config: LanConfig) => invoke<void>('lan_start', { config })

/** 幂等启动：已经在跑同一个网络就什么都不做。进房间统一走这条。 */
export const lanEnsure = (config: LanConfig) => invoke<void>('lan_ensure', { config })
export const lanStop = () => invoke<void>('lan_stop')
export const lanStatus = () => invoke<LanStatus>('lan_status')

/** 房间服务默认端口。挑一个不常用的，避开 25565 / 24454 这些游戏端口。 */
export const ROOM_PORT = 28900
