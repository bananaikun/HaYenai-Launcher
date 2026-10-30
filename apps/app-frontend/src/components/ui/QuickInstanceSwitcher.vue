<script setup>
import { SpinnerIcon } from '@modrinth/assets'
import { injectNotificationManager, defineMessages, useVIntl } from '@modrinth/ui'
import dayjs from 'dayjs'
import { computed, onUnmounted, ref } from 'vue'

import InstanceIcon from '@/components/ui/InstanceIcon.vue'
import NavButton from '@/components/ui/NavButton.vue'
import { instance_listener } from '@/helpers/events.js'
import { list } from '@/helpers/instance'
import { useTheming } from '@/store/state'

// 该组件是纯 JS 的 <script setup>（没有 lang="ts"），所以 props 用运行时声明
defineProps({ variant: { type: String, default: 'rail' } })

const { handleError } = injectNotificationManager()
const { formatMessage } = useVIntl()

const messages = defineMessages({
    noInstances: { id: 'app.navigation.no-instances', defaultMessage: 'No instances yet' },
})

const themeStore = useTheming()

const fullInstanceList = ref([])
const instanceCount = computed(() => themeStore.sidebarInstanceCount)

const recentInstances = computed(() => {
    if (instanceCount.value > 0) {
        return fullInstanceList.value.slice(0, instanceCount.value)
    }
    return fullInstanceList.value
})

const getInstances = async () => {
    const instances = await list().catch((error) => {
        handleError(error)
        return []
    })

    fullInstanceList.value = instances.sort((a, b) => {
        const dateACreated = dayjs(a.created)
        const dateAPlayed = a.last_played ? dayjs(a.last_played) : dayjs(0)

        const dateBCreated = dayjs(b.created)
        const dateBPlayed = b.last_played ? dayjs(b.last_played) : dayjs(0)

        const dateA = dateACreated.isAfter(dateAPlayed) ? dateACreated : dateAPlayed
        const dateB = dateBCreated.isAfter(dateBPlayed) ? dateBCreated : dateBPlayed

        if (dateA.isSame(dateB)) {
            return a.name.localeCompare(b.name)
        }

        return dateB - dateA
    })
}

await getInstances()

const unlistenInstance = await instance_listener(async (event) => {
    if (event.event !== 'synced') {
        await getInstances()
    }
})

onUnmounted(() => {
    unlistenInstance()
})
</script>

<template>
    <!--
      list：246px 侧栏里的一行一条（图标 + 名字），侧栏收起后由 CSS 退化成图标列。
      rail：原有的 4rem 图标轨道形态，留给还没迁到新外壳的宿主。
    -->
    <template v-if="variant === 'list'">
        <NavButton
            v-for="instance in recentInstances"
            :key="instance.id"
            :to="`/instance/${encodeURIComponent(instance.id)}`"
            :label="instance.name"
            :en="(instance.loader || 'vanilla').toUpperCase()"
            :title="instance.name"
        >
            <InstanceIcon
                :icon-path="instance.icon_path"
                :instance-id="instance.id"
                :loader="instance.loader"
                size="22px"
                :class="
                    instance.install_stage !== 'installed'
                        ? 'brightness-[0.25] scale-[0.85]'
                        : undefined
                "
            />
            <template v-if="instance.install_stage !== 'installed'" #trail>
                <SpinnerIcon class="ml-auto size-3.5 shrink-0 animate-spin" />
            </template>
        </NavButton>
        <span v-if="recentInstances.length === 0" class="side-empty">
            {{ formatMessage(messages.noInstances) }}
        </span>
    </template>

    <template v-else>
        <div v-for="instance in recentInstances" :key="instance.id" v-tooltip.right="instance.name">
            <NavButton :to="`/instance/${encodeURIComponent(instance.id)}`" class="relative">
                <InstanceIcon
                    :icon-path="instance.icon_path"
                    :instance-id="instance.id"
                    :loader="instance.loader"
                    size="28px"
                    :class="`transition-all ${instance.install_stage !== 'installed' ? `brightness-[0.25] scale-[0.85]` : `group-hover:brightness-75`}`"
                />
                <div
                    v-if="instance.install_stage !== 'installed'"
                    class="absolute inset-0 flex items-center justify-center z-10 pointer-events-none"
                >
                    <SpinnerIcon class="animate-spin w-4 h-4" />
                </div>
            </NavButton>
        </div>
        <div v-if="recentInstances.length > 0" class="h-px w-6 mx-auto my-2 bg-divider"></div>
    </template>
</template>

<style scoped lang="scss">
/* 没有实例时的一行提示：与 .nav-cn 同字号，不抢导航的层级 */
.side-empty {
    display: block;
    padding: 8px 12px;
    font-size: 12px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
</style>
