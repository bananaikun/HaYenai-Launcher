import './meta'

import type { RouteRecordRaw } from 'vue-router'

/** Top-level utility pages that do not belong to a larger content vertical. */
export const utilityRoutes: RouteRecordRaw[] = [
    {
        path: '/worlds',
        name: 'Worlds',
        component: () => import('@/pages/Worlds.vue'),
        meta: {
            breadcrumb: [{ name: 'Worlds' }],
        },
    },
    {
        path: '/create',
        name: 'Create',
        component: () => import('@/pages/Create.vue'),
        meta: { useRootContext: false },
    },
    {
        path: '/downloads',
        name: 'Downloads',
        component: () => import('@/pages/Downloads.vue'),
        meta: {
            breadcrumb: [{ name: 'Downloads' }],
            discordActivity: 'Idling...',
        },
    },
    {
        path: '/music',
        name: 'Music',
        component: () => import('@/pages/Music.vue'),
        meta: {
            breadcrumb: [{ name: 'Music' }],
            discordActivity: 'Listening to music...',
        },
    },
    // 2026-11-14 房间功能暂时下线：路由保留但重定向到首页，避免旧书签/旧链接
    // 打开一个「看起来还能用」的空房间页。恢复时改回 component 那一行即可。
    {
        path: '/rooms',
        redirect: '/',
    },
    {
        path: '/agent',
        name: 'Agent',
        component: () => import('@/pages/Agent.vue'),
        meta: {
            breadcrumb: [{ name: 'Agent' }],
        },
    },
    {
        path: '/settings',
        name: 'Settings',
        component: () => import('@/pages/Settings.vue'),
        meta: {
            breadcrumb: [{ name: 'Settings' }],
            discordActivity: 'Idling...',
        },
    },
    {
        path: '/skins',
        name: 'SkinSelector',
        component: () => import('@/pages/Skins.vue'),
        meta: {
            breadcrumb: [{ name: 'Skin selector' }],
            discordActivity: 'Changing skins...',
        },
    },
    {
        path: '/screenshots',
        name: 'Screenshots',
        component: () => import('@/pages/Screenshots.vue'),
        meta: { breadcrumb: [{ name: 'Screenshots' }] },
    },
    {
        path: '/help/drop',
        name: 'DropHelp',
        component: () => import('@/pages/help/DropHelp.vue'),
        meta: {
            breadcrumb: [{ name: 'Drop help' }],
        },
    },
]
