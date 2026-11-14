<script setup lang="ts">
/**
 * 磁悬条的设置区块（挂在「设置 → 外观 → 首页与导航」下面）。
 *
 * 改一项就立刻写盘并广播：磁悬条在每个页面上都挂着，用户拖着不透明度滑块时
 * 应该当场看到条子变透明，而不是保存后才生效。
 */
import { ArrowDownIcon, ArrowUpIcon } from '@modrinth/assets'
import { defineMessages, StyledInput, useVIntl } from '@modrinth/ui'

// ⚠ SettingsSection / SettingsRow 是**本地组件**（AppearanceSettings 也从这里引），
// 从 @modrinth/ui 引会解析失败，整页渲染崩成空白。
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import { ref } from 'vue'

import { magneticBarPageMessages, magneticWidgetMessages } from '@/helpers/magnetic/labels'
import {
    clearWidgetPositions,
    moveWidget,
    readMagneticSettings,
    writeMagneticSettings,
    type MagneticBarPage,
    type MagneticSettings,
    type MagneticWidgetId,
} from '@/helpers/magnetic-settings'

const { formatMessage } = useVIntl()

const messages = defineMessages({
    title: { id: 'app.magnetic-settings.title', defaultMessage: 'Floating bar' },
    opacityTitle: { id: 'app.magnetic-settings.opacity', defaultMessage: 'Background opacity' },
    opacityDesc: {
        id: 'app.magnetic-settings.opacity-desc',
        defaultMessage:
            'How solid the bar background looks. 0 keeps only blur and the hairline border.',
    },
    scaleTitle: { id: 'app.magnetic-settings.scale', defaultMessage: 'Size' },
    scaleDesc: {
        id: 'app.magnetic-settings.scale-desc',
        defaultMessage: 'Changes the height and padding of the bar.',
    },
    resetLayoutTitle: {
        id: 'app.magnetic-settings.reset-layout',
        defaultMessage: 'Widget layout',
    },
    resetLayoutDesc: {
        id: 'app.magnetic-settings.reset-layout-desc',
        defaultMessage:
            'Widgets are placed freely in the panel — drag them anywhere. This puts them all back to the default rows.',
    },
    resetLayoutAction: {
        id: 'app.magnetic-settings.reset-layout-action',
        defaultMessage: 'Reset to default',
    },
    barPagesTitle: {
        id: 'app.magnetic-settings.bar-pages',
        defaultMessage: 'Pages on the bar',
    },
    barPagesDesc: {
        id: 'app.magnetic-settings.bar-pages-desc',
        defaultMessage:
            'These are the pages shown on the floating bar itself (scroll or click the dots). Order here = order on the bar.',
    },
    widthTitle: { id: 'app.magnetic-settings.width', defaultMessage: 'Panel width' },
    widthDesc: {
        id: 'app.magnetic-settings.width-desc',
        defaultMessage: 'How wide the expanded panel is. Narrower = the compact look.',
    },
    widgetsTitle: { id: 'app.magnetic-settings.widgets', defaultMessage: 'Widgets in the panel' },
    widgetsDesc: {
        id: 'app.magnetic-settings.widgets-desc',
        defaultMessage:
            'Tick to show, use the arrows to reorder. Order here is the order in the panel.',
    },
    serverTitle: { id: 'app.magnetic-settings.server', defaultMessage: 'Server status' },
    serverDesc: {
        id: 'app.magnetic-settings.server-desc',
        defaultMessage:
            'Shown by the server widget. Use host:port for a Minecraft server (real ping and player count), or an http(s) URL to check that it answers 2xx.',
    },
    serverName: { id: 'app.magnetic-settings.server-name', defaultMessage: 'Display name' },
    serverUrl: { id: 'app.magnetic-settings.server-url', defaultMessage: 'Address' },
    enabledTitle: { id: 'app.magnetic-settings.enabled', defaultMessage: 'Show the floating bar' },
    enabledDesc: {
        id: 'app.magnetic-settings.enabled-desc',
        defaultMessage: 'Turn the whole bar off when it gets in the way. Nothing renders and nothing polls.',
    },
    up: { id: 'app.magnetic-settings.move-up', defaultMessage: 'Move up' },
    down: { id: 'app.magnetic-settings.move-down', defaultMessage: 'Move down' },
})

const settings = ref<MagneticSettings>(readMagneticSettings())

function save(): void {
    writeMagneticSettings(settings.value)
}

function setBarEnabled(value: boolean): void {
    settings.value.barEnabled = value
    save()
}

function setOpacity(value: string | number): void {
    settings.value.opacity = Number(value)
    save()
}

function setScale(value: string | number): void {
    settings.value.scale = Number(value)
    save()
}

function resetLayout(): void {
    clearWidgetPositions(settings.value)
    save()
}

function setPanelWidth(value: string | number): void {
    settings.value.panelWidth = Number(value)
    save()
}

function toggleBarPage(id: MagneticBarPage, enabled: boolean): void {
    const item = settings.value.barPages.find((entry) => entry.id === id)
    if (!item) return
    // 至少留一页可显示，否则条子上会一片空白。
    const enabledCount = settings.value.barPages.filter((entry) => entry.enabled).length
    if (!enabled && enabledCount <= 1) return
    item.enabled = enabled
    save()
}

function moveBarPage(index: number, delta: number): void {
    const list = settings.value.barPages.slice()
    const target = index + delta
    if (target < 0 || target >= list.length) return
    const [moved] = list.splice(index, 1)
    list.splice(target, 0, moved)
    settings.value.barPages = list
    save()
}

function toggleWidget(id: MagneticWidgetId, enabled: boolean): void {
    const item = settings.value.widgets.find((entry) => entry.id === id)
    if (!item) return
    item.enabled = enabled
    save()
}

function move(index: number, delta: number): void {
    settings.value.widgets = moveWidget(settings.value.widgets, index, index + delta)
    save()
}
</script>

<template>
    <SettingsSection :title="formatMessage(messages.title)" title-id="settings-target-magnetic-bar">
        <SettingsRow>
            <template #label>{{ formatMessage(messages.enabledTitle) }}</template>
            <template #description>{{ formatMessage(messages.enabledDesc) }}</template>
            <template #control>
                <label class="mgs-check">
                    <input
                        type="checkbox"
                        :checked="settings.barEnabled"
                        @change="
                            setBarEnabled(($event.target as HTMLInputElement).checked)"
                    />
                </label>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.opacityTitle) }}</template>
            <template #description>{{ formatMessage(messages.opacityDesc) }}</template>
            <template #control>
                <div class="mgs-slider">
                    <input
                        type="range"
                        min="0"
                        max="100"
                        step="1"
                        :value="settings.opacity"
                        :aria-label="formatMessage(messages.opacityTitle)"
                        @input="setOpacity(($event.target as HTMLInputElement).value)"
                    />
                    <span>{{ settings.opacity }}%</span>
                </div>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.scaleTitle) }}</template>
            <template #description>{{ formatMessage(messages.scaleDesc) }}</template>
            <template #control>
                <div class="mgs-slider">
                    <input
                        type="range"
                        min="80"
                        max="120"
                        step="1"
                        :value="settings.scale"
                        :aria-label="formatMessage(messages.scaleTitle)"
                        @input="setScale(($event.target as HTMLInputElement).value)"
                    />
                    <span>{{ settings.scale }}%</span>
                </div>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.resetLayoutTitle) }}</template>
            <template #description>{{ formatMessage(messages.resetLayoutDesc) }}</template>
            <template #control>
                <button type="button" class="mgs-btn" @click="resetLayout">
                    {{ formatMessage(messages.resetLayoutAction) }}
                </button>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.barPagesTitle) }}</template>
            <template #description>{{ formatMessage(messages.barPagesDesc) }}</template>
            <template #control>
                <div class="mgs-list">
                    <div v-for="(item, index) in settings.barPages" :key="item.id" class="mgs-item">
                        <label class="mgs-check">
                            <input
                                type="checkbox"
                                :checked="item.enabled"
                                @change="
                                    toggleBarPage(
                                        item.id,
                                        ($event.target as HTMLInputElement).checked,
                                    )
                                "
                            />
                            <span>{{ formatMessage(magneticBarPageMessages[item.id]) }}</span>
                        </label>
                        <div class="mgs-move">
                            <button
                                type="button"
                                :disabled="index === 0"
                                v-tooltip="formatMessage(messages.up)"
                                @click="moveBarPage(index, -1)"
                            >
                                <ArrowUpIcon />
                            </button>
                            <button
                                type="button"
                                :disabled="index === settings.barPages.length - 1"
                                v-tooltip="formatMessage(messages.down)"
                                @click="moveBarPage(index, 1)"
                            >
                                <ArrowDownIcon />
                            </button>
                        </div>
                    </div>
                </div>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.widthTitle) }}</template>
            <template #description>{{ formatMessage(messages.widthDesc) }}</template>
            <template #control>
                <div class="mgs-slider">
                    <input
                        type="range"
                        min="560"
                        max="1080"
                        step="20"
                        :value="settings.panelWidth"
                        :aria-label="formatMessage(messages.widthTitle)"
                        @input="setPanelWidth(($event.target as HTMLInputElement).value)"
                    />
                    <span>{{ settings.panelWidth }}px</span>
                </div>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.widgetsTitle) }}</template>
            <template #description>{{ formatMessage(messages.widgetsDesc) }}</template>
            <template #control>
                <div class="mgs-list">
                    <div v-for="(item, index) in settings.widgets" :key="item.id" class="mgs-item">
                        <label class="mgs-check">
                            <input
                                type="checkbox"
                                :checked="item.enabled"
                                @change="
                                    toggleWidget(
                                        item.id,
                                        ($event.target as HTMLInputElement).checked,
                                    )
                                "
                            />
                            <span>{{ formatMessage(magneticWidgetMessages[item.id]) }}</span>
                        </label>
                        <div class="mgs-move">
                            <button
                                type="button"
                                :disabled="index === 0"
                                v-tooltip="formatMessage(messages.up)"
                                @click="move(index, -1)"
                            >
                                <ArrowUpIcon />
                            </button>
                            <button
                                type="button"
                                :disabled="index === settings.widgets.length - 1"
                                v-tooltip="formatMessage(messages.down)"
                                @click="move(index, 1)"
                            >
                                <ArrowDownIcon />
                            </button>
                        </div>
                    </div>
                </div>
            </template>
        </SettingsRow>

        <SettingsRow>
            <template #label>{{ formatMessage(messages.serverTitle) }}</template>
            <template #description>{{ formatMessage(messages.serverDesc) }}</template>
            <template #control>
                <div class="mgs-pair">
                    <StyledInput
                        :model-value="settings.server.name"
                        :placeholder="formatMessage(messages.serverName)"
                        @update:model-value="
                            (value) => {
                                settings.server.name = String(value)
                                save()
                            }
                        "
                    />
                    <StyledInput
                        :model-value="settings.server.url"
                        :placeholder="formatMessage(messages.serverUrl)"
                        @update:model-value="
                            (value) => {
                                settings.server.url = String(value)
                                save()
                            }
                        "
                    />
                </div>
            </template>
        </SettingsRow>
    </SettingsSection>
</template>

<style scoped>
.mgs-slider {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

.mgs-slider input[type='range'] {
    width: 160px;
    accent-color: var(--color-brand);
}

.mgs-slider span {
    min-width: 2.6rem;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: var(--color-text-secondary);
}

.mgs-list {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 210px;
}

.mgs-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.2rem 0.35rem;
    border-radius: 8px;
    border: 1px solid transparent;
}

.mgs-item:hover {
    border-color: var(--stroke-1);
    background: var(--glass-1);
}

.mgs-check {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.76rem;
    cursor: pointer;
}

.mgs-check input {
    accent-color: var(--color-brand);
}

.mgs-move {
    display: flex;
    gap: 0.1rem;
}

.mgs-move button {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    border: 1px solid var(--stroke-1);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
}

.mgs-move button:hover:not(:disabled) {
    background: var(--glass-2);
    color: var(--text-0);
}

.mgs-move button:disabled {
    opacity: 0.35;
    cursor: default;
}

.mgs-move :deep(svg) {
    width: 11px;
    height: 11px;
}

.mgs-pair {
    display: flex;
    gap: 0.4rem;
    min-width: 260px;
}

.mgs-btn {
    padding: 0.28rem 0.7rem;
    border-radius: 9px;
    border: 1px solid var(--stroke-1);
    background: var(--glass-1);
    color: var(--color-text-secondary);
    font-size: 0.74rem;
    cursor: pointer;
    transition:
        background var(--t-fast) var(--ease),
        color var(--t-fast) var(--ease);
}

.mgs-btn:hover {
    background: var(--glass-2);
    color: var(--text-0);
}

/* 恢复默认布局用的按钮，保持一致但不抢眼。 */
</style>
