<template>
    <Button
        v-if="showUpdatePill"
        type="outlined"
        color="brand"
        class="!h-[34px] text-sm !transition-[opacity,transform,background-color,color,filter] !duration-200 ease-out [&:not(:disabled):hover]:!bg-brand [&:not(:disabled):hover]:!text-[var(--color-accent-contrast)] [&:not(:disabled):focus-visible]:!bg-brand [&:not(:disabled):focus-visible]:!text-[var(--color-accent-contrast)]"
        native-type="button"
        :class="{
            'opacity-0 scale-[0.96]': finishedDownloading && !animateReadyPill,
            'opacity-100 scale-100': finishedDownloading && animateReadyPill,
        }"
        :disabled="isUpdateDownloading"
        :aria-busy="isUpdateDownloading"
        @click="handleUpdateClick"
    >
        <RefreshCwIcon v-if="finishedDownloading" :class="{ 'animate-spin': restarting }" />
        <DownloadIcon v-else />
        <span v-if="isUpdateDownloading">
            {{ formatMessage(messages.downloadingUpdate) }}
            <span class="inline-block w-[3ch] text-right tabular-nums">{{ downloadPercent }}%</span>
        </span>
        <span v-else>{{ updateLabel }}</span>
    </Button>
</template>

<script setup lang="ts">
import { DownloadIcon, RefreshCwIcon } from '@modrinth/assets'
import { Button, defineMessages, useVIntl } from '@modrinth/ui'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'

import {
    appUpdateState,
    downloadAvailableAppUpdate,
    installAvailableAppUpdate,
} from '@/providers/app-update'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    update: {
        id: 'app.action-bar.update',
        defaultMessage: 'Update',
    },
    downloadingUpdate: {
        id: 'app.action-bar.downloading-update',
        defaultMessage: 'Downloading update',
    },
    reloadToUpdate: {
        id: 'app.action-bar.reload-to-update',
        defaultMessage: 'Reload to update',
    },
})

const {
    availableUpdate,
    downloading,
    downloadPercent,
    downloadProgress,
    finishedDownloading,
    isVisible: isUpdateVisible,
    restarting,
} = appUpdateState

const isUpdateDownloading = computed(
    () =>
        downloading.value ||
        (downloadProgress.value > 0 && downloadProgress.value < 1 && !finishedDownloading.value),
)
/* 2026-11-01 主人：「每次都要进设置界面刷新然后手动更新…能不能添加一个更新选项。」
   原来的条件是 `isUpdateVisible && (finishedDownloading || metered)` —— 也就是
   **有新版本但还没下载时按钮根本不渲染**，标题栏里看不到任何"该更新了"的入口，
   人只能进「设置 → 更新」手动点检查。改成只要有可用更新就常驻显示。
   点击逻辑本来就覆盖三态（下载中忽略 / 已下载则安装 / 否则开始下载），无需改。 */
const showUpdatePill = computed(() => isUpdateVisible.value)
const animateReadyPill = ref(false)
const updateLabel = computed(() => {
    if (isUpdateDownloading.value) {
        return formatMessage(messages.downloadingUpdate)
    }

    if (finishedDownloading.value) {
        return formatMessage(messages.reloadToUpdate)
    }

    // 带版本号：光写「更新」看不出要更新到哪一版（这也是主人报"还在检测 1.0.33"的观感来源）。
    const version = availableUpdate.value?.version
    const base = formatMessage(messages.update)
    return version ? `${base} v${version}` : base
})
let readyPillAnimationFrame: number | null = null
watch([showUpdatePill, finishedDownloading], async ([show, ready], [wasShown, wasReady]) => {
    if (readyPillAnimationFrame !== null) {
        cancelAnimationFrame(readyPillAnimationFrame)
        readyPillAnimationFrame = null
    }

    if (!show || !ready) {
        animateReadyPill.value = false
        return
    }

    if (wasShown && wasReady) {
        return
    }

    animateReadyPill.value = false
    await nextTick()
    readyPillAnimationFrame = requestAnimationFrame(() => {
        animateReadyPill.value = true
        readyPillAnimationFrame = null
    })
})
async function handleUpdateClick() {
    if (isUpdateDownloading.value) {
        return
    }

    if (finishedDownloading.value) {
        await installAvailableAppUpdate()
    } else {
        await downloadAvailableAppUpdate()
    }
}

onBeforeUnmount(() => {
    if (readyPillAnimationFrame !== null) {
        cancelAnimationFrame(readyPillAnimationFrame)
    }
})
</script>
