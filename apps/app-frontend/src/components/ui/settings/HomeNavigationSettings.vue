<script setup lang="ts">
import AppearanceSettings from './AppearanceSettings.vue'
import MagneticSettingsSection from './MagneticSettingsSection.vue'
</script>

<template>
    <!--
        ⚠ 必须只有一个根节点：设置页用 `<KeepAlive><component :is="category.content" /></KeepAlive>`，
        KeepAlive 要求单一根。之前这里并列两个组件（多根 Fragment），切分类时搬 DOM
        会拿到 null 的 parentNode，整块内容区直接空白。
    -->
    <div class="flex flex-col gap-3">
        <AppearanceSettings scope="home-navigation" />
        <!-- 磁悬条：外观 + 组件清单 + 自由布局复位 -->
        <MagneticSettingsSection />
    </div>
</template>
