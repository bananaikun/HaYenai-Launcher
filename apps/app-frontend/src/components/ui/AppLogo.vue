<template>
    <div
        aria-label="HaYenai"
        class="flex h-full items-center gap-2 font-extrabold text-[var(--color-text-primary)]"
    >
        <img aria-hidden="true" class="aspect-square h-full object-contain" :src="logoVisual" />
        <span v-if="!iconOnly" class="hidden text-sm tracking-wide xl:inline">HaYenai</span>
    </div>
</template>

<script setup lang="ts">
import logoVisual from '@/assets/hayenai.png'

defineProps<{
    iconOnly?: boolean
}>()
</script>
