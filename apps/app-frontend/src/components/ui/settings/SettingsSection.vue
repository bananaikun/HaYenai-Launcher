<script setup lang="ts">
import HyIcon from '@/ui/icons/HyIcon.vue'

withDefaults(
    defineProps<{
        title?: string
        description?: string
        titleId?: string
        /** 后台 .set-head 左侧的图标底座；不传则不画 */
        icon?: string
    }>(),
    { title: undefined, description: undefined, titleId: undefined, icon: undefined },
)
</script>

<template>
    <!-- 结构与类名照后台管理 <Section>：.hy-set.panel.fill > .set-head + .set-body -->
    <section class="hy-set panel fill">
        <header v-if="title || description || $slots.header || $slots.extra" class="set-head">
            <span v-if="icon" class="ico"><HyIcon :name="icon" :size="18" /></span>
            <div class="txt">
                <h2 v-if="title" :id="titleId" :tabindex="titleId ? -1 : undefined" class="t">
                    {{ title }}
                </h2>
                <p v-if="description" class="d">{{ description }}</p>
                <slot name="header" />
            </div>
            <div class="right"><slot name="extra" /></div>
        </header>
        <div class="set-body">
            <slot />
        </div>
    </section>
</template>
