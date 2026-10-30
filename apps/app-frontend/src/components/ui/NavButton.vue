<!--
  NavButton · 侧栏里的一行导航
  ------------------------------------------------------------
  结构与类名照后台管理 <Sidebar> 的 nav-item：`.nav-item > .nav-ico + (.nav-cn + .nav-en)`，
  样式来自 `assets/stylesheets/admin-verbatim.scss`（后台 shell.css 的逐字副本），本组件只负责
  三段式渲染（路由链接 / 禁用 / 命令式）和选中态判定。

  `#trail` 槽给父级项放展开箭头；`label` 为空时不渲染文字块 —— 收起态侧栏只有 46px 图标位，
  留一个空 span 会让 gap 把图标顶偏。
-->
<template>
    <RouterLink
        v-if="typeof to === 'string' && !disabled"
        :to="to"
        v-bind="$attrs"
        active-class="nav-router-active"
        :class="activeClasses"
        class="nav-item"
    >
        <span class="nav-ico"><slot /></span>
        <span v-if="label" class="nav-label">
            <span class="nav-cn">{{ label }}</span>
            <span v-if="enLabel" class="nav-en">{{ enLabel }}</span>
        </span>
        <slot name="trail" />
    </RouterLink>
    <button
        v-else-if="typeof to === 'string'"
        v-bind="$attrs"
        type="button"
        aria-disabled="true"
        tabindex="-1"
        :class="activeClasses"
        class="nav-item"
        @click.prevent
        @keydown.enter.prevent
        @keyup.enter.prevent
        @keydown.space.prevent
        @keyup.space.prevent
    >
        <span class="nav-ico"><slot /></span>
        <span v-if="label" class="nav-label">
            <span class="nav-cn">{{ label }}</span>
            <span v-if="enLabel" class="nav-en">{{ enLabel }}</span>
        </span>
        <slot name="trail" />
    </button>
    <button
        v-else
        v-bind="$attrs"
        type="button"
        class="nav-item button-animation"
        :disabled="disabled"
        @click="to"
    >
        <span class="nav-ico"><slot /></span>
        <span v-if="label" class="nav-label">
            <span class="nav-cn">{{ label }}</span>
            <span v-if="enLabel" class="nav-en">{{ enLabel }}</span>
        </span>
        <slot name="trail" />
    </button>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'
import { RouterLink, useLink, useRoute } from 'vue-router'

const route = useRoute()

type RouteFunction = (route: RouteLocationNormalizedLoaded) => boolean

const props = withDefaults(
    defineProps<{
        to: (() => void) | string
        isPrimary?: RouteFunction
        isSubpage?: RouteFunction
        highlightOverride?: boolean
        disabled?: boolean
        /** 中文主标签；为空则不渲染文字块（收起态） */
        label?: string
        /** 英文副标签，全大写；不给则按路由首段推一个 */
        en?: string
    }>(),
    {
        disabled: false,
        isPrimary: undefined,
        isSubpage: undefined,
        label: undefined,
        en: undefined,
    },
)

const enLabel = computed(() => {
    if (props.en) return props.en
    if (!props.label) return ''
    const raw = typeof props.to === 'string' ? props.to : ''
    const seg = raw.split('/').filter(Boolean)[0] ?? 'home'
    return seg.toUpperCase()
})

const link = useLink({ to: computed(() => (typeof props.to === 'string' ? props.to : '/')) })
const activeClasses = computed(() => {
    const primary = props.isPrimary ? props.isPrimary(route) : link.isActive.value
    return {
        'router-link-active': primary,
        'subpage-active': !primary && !!props.isSubpage?.(route),
        on: primary,
    }
})

defineOptions({
    inheritAttrs: false,
})
</script>
