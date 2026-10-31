<script setup lang="ts">
import { CalendarIcon, ExternalIcon, HistoryIcon } from '@modrinth/assets'
import { Accordion, defineMessages, TagItem, useVIntl } from '@modrinth/ui'
import { renderString } from '@modrinth/utils'
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, onMounted, ref } from 'vue'

import { fetchReleaseAnnouncements, type ReleaseAnnouncement } from '@/announcements/github'

/**
 * 更新公告 = 本仓库的 GitHub Releases（发版说明）。
 *
 * 2026-10-31 重写：原数据源是 `announcements/catalog.ts`（上游 Axolotl 的更新日志：
 * 1.10.0-beta.1 / 1.9.8 / 1.9.7 …），和这个启动器没有关系，主人要求全部换掉。
 * 现在整个面板只有一条真源：GitHub Releases 的 Markdown 正文。
 */
const props = defineProps<{
    currentVersion: string
}>()

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: {
        id: 'app.settings.updates.announcements.title',
        defaultMessage: 'Update announcements',
    },
    description: {
        id: 'app.settings.updates.announcements.description',
        defaultMessage: 'See what changed in this version and browse previous releases.',
    },
    history: {
        id: 'app.settings.updates.announcements.history',
        defaultMessage: 'Version history',
    },
    empty: {
        id: 'app.settings.updates.announcements.empty',
        defaultMessage: 'No release notes are available yet.',
    },
    loading: {
        id: 'app.settings.updates.announcements.loading',
        defaultMessage: 'Loading release notes…',
    },
    openRelease: {
        id: 'app.settings.updates.announcements.open-release',
        defaultMessage: 'Open release page',
    },
})

const items = ref<ReleaseAnnouncement[]>([])
const loading = ref(true)

onMounted(async () => {
    items.value = await fetchReleaseAnnouncements()
    loading.value = false
})

/** 当前版本那条；没有就退回最新一条（刚发包还没写 notes 时不至于空掉）。 */
const current = computed<ReleaseAnnouncement | null>(() => {
    const list = items.value
    if (!list.length) return null
    return list.find((item) => item.version === props.currentVersion) ?? list[0]
})

const history = computed(() =>
    current.value ? items.value.filter((item) => item.version !== current.value?.version) : [],
)
const currentHtml = computed(() => renderString(current.value?.body ?? ''))

async function openRelease(url: string): Promise<void> {
    if (!url) return
    try {
        await openUrl(url)
    } catch {
        // opener 拒绝（非法地址）时静默：读不到链接不该弹错误
    }
}
</script>

<template>
    <section class="update-announcement-history">
        <div class="flex min-w-0 flex-col gap-1">
            <h2 class="m-0 text-lg font-semibold text-[var(--color-text-primary)]">
                {{ formatMessage(messages.title) }}
            </h2>
            <p class="m-0 leading-relaxed text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.description) }}
            </p>
        </div>

        <p v-if="loading" class="m-0 text-sm text-[var(--color-text-tertiary)]">
            {{ formatMessage(messages.loading) }}
        </p>

        <template v-else-if="current">
            <div class="min-w-0">
                <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
                    <span class="font-semibold text-[var(--color-text-primary)]">
                        {{ current.name }}
                    </span>
                    <TagItem class="px-1.5 py-0.5 text-xs">v{{ current.version }}</TagItem>
                    <span
                        v-if="current.publishedAt"
                        class="flex items-center gap-1 text-sm text-[var(--color-text-tertiary)]"
                    >
                        <CalendarIcon aria-hidden="true" class="size-3.5" />
                        <time :datetime="current.publishedAt">{{ current.publishedAt }}</time>
                    </span>
                </div>
                <div class="markdown-body mt-3 break-words" v-html="currentHtml" />
                <button
                    v-if="current.url"
                    type="button"
                    class="mt-3 flex items-center gap-2 text-sm font-semibold text-[var(--color-text-link)]"
                    @click="openRelease(current.url)"
                >
                    <ExternalIcon aria-hidden="true" class="size-4" />
                    {{ formatMessage(messages.openRelease) }}
                </button>
            </div>

            <div v-if="history.length" class="flex min-w-0 flex-col gap-3">
                <h3
                    class="m-0 flex items-center gap-2 text-base font-semibold text-[var(--color-text-primary)]"
                >
                    <HistoryIcon
                        aria-hidden="true"
                        class="size-4 text-[var(--color-text-tertiary)]"
                    />
                    {{ formatMessage(messages.history) }}
                </h3>
                <div class="flex min-w-0 flex-col gap-2">
                    <Accordion
                        v-for="item in history"
                        :key="item.version"
                        class="update-announcement-history-item"
                        button-class="group flex w-full cursor-pointer items-center gap-3 border-0 bg-transparent px-4 py-3 text-left"
                    >
                        <template #title>
                            <div class="flex min-w-0 flex-1 flex-col gap-1">
                                <span
                                    class="truncate font-semibold text-[var(--color-text-default)]"
                                >
                                    {{ item.name }}
                                </span>
                                <div
                                    class="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-[var(--color-text-tertiary)]"
                                >
                                    <TagItem class="px-1.5 py-0.5 text-xs"
                                        >v{{ item.version }}</TagItem
                                    >
                                    <span v-if="item.publishedAt" class="flex items-center gap-1">
                                        <CalendarIcon aria-hidden="true" class="size-3.5" />
                                        <time :datetime="item.publishedAt">{{
                                            item.publishedAt
                                        }}</time>
                                    </span>
                                </div>
                            </div>
                        </template>
                        <div class="update-announcement-history-item-content">
                            <div class="markdown-body break-words" v-html="renderString(item.body)" />
                        </div>
                    </Accordion>
                </div>
            </div>
        </template>

        <p v-else class="m-0 text-sm text-[var(--color-text-tertiary)]">
            {{ formatMessage(messages.empty) }}
        </p>
    </section>
</template>

<style scoped>
/* 2026-10-31：原来是 `border:1px solid var(--surface-4)` + `background: var(--surface-2)`
   —— 面板是一块实心色。统一成全站玻璃配方（描边 stroke 令牌 + glass 渐变 + 模糊）。 */
.update-announcement-history {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: var(--gap-xl);
    padding: var(--gap-xl);
    border: 1px solid var(--stroke-1);
    border-radius: var(--radius-md);
    background-color: color-mix(in srgb, #0a0b10 var(--ui-dim, 25%), transparent);
    background-image: linear-gradient(180deg, var(--glass-2) 0%, var(--glass-1) 100%);
    -webkit-backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(var(--ui-saturate, 105%));
    backdrop-filter: blur(var(--ui-glass-blur, 28px)) saturate(var(--ui-saturate, 105%));
    box-shadow: var(--inset-1);
}

.update-announcement-history-item {
    min-width: 0;
    overflow: hidden;
    border: 1px solid var(--stroke-1);
    border-radius: var(--radius-sm);
    background: var(--glass-1);
    transition: border-color 120ms ease;
}

.update-announcement-history-item-content {
    padding: var(--gap-lg);
    border-top: 1px solid var(--stroke-1);
}
</style>
