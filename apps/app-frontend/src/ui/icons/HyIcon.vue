<!--
  HyIcon · 统一图标组件
  ------------------------------------------------------------
  <HyIcon name="settings" :size="18" />

  - 24×24 视口，线性描边 + 圆角端点（stroke-linecap/linejoin: round）
  - 默认线宽 1.75；后台原图是 1.25，逐像素对齐后台时传 :sw="1.25"
  - 颜色跟随 currentColor，直接在父级写 color 即可
  - name 不在表里时兜底成 grid（与后台 Icon.tsx 的行为一致），不会再出现空白按钮
  - 仅 github / play 是实心填充，其余一律描边
-->
<script setup lang="ts">
import { computed } from 'vue'

import { FILLED_ICONS, ICON_PATHS, type IconName } from './names'

const props = withDefaults(
    defineProps<{
        /** 图标名，见 names.ts 的 ICON_PATHS */
        name: IconName | string
        /** 边长，px */
        size?: number
        /** 线宽，viewBox 单位 */
        sw?: number
        /** 可访问名；不传则视为装饰性图标并对读屏隐藏 */
        label?: string
    }>(),
    { size: 18, sw: 1.75, label: undefined },
)

const paths = computed<readonly string[]>(
    () => ICON_PATHS[props.name as IconName] ?? ICON_PATHS.grid,
)
const filled = computed(() => FILLED_ICONS.has(props.name))
</script>

<template>
    <svg
        class="hy-icon"
        :width="size"
        :height="size"
        viewBox="0 0 24 24"
        :fill="filled ? 'currentColor' : 'none'"
        :stroke="filled ? 'none' : 'currentColor'"
        :stroke-width="sw"
        stroke-linecap="round"
        stroke-linejoin="round"
        :aria-hidden="label ? undefined : 'true'"
        :role="label ? 'img' : undefined"
        :focusable="false"
    >
        <title v-if="label">{{ label }}</title>
        <path v-for="(d, index) in paths" :key="index" :d="d" />
    </svg>
</template>

<style scoped>
.hy-icon {
    display: block;
    flex: none;
    vertical-align: middle;
}
</style>
