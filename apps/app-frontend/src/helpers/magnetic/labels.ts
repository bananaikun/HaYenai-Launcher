/**
 * 磁悬条的名字与搜索别名。
 *
 * 为什么单独一个文件：组件名要参与 i18n（以前硬编码中文，英文界面下九张卡全叫中文名，
 * 而且搜索按中文串匹配，输入 "clock" 一条都搜不到）。id 的真源仍在
 * `helpers/magnetic-settings.ts` 的 `MAGNETIC_WIDGET_IDS`，这里只放「怎么显示 / 怎么搜」。
 */
import { defineMessages } from '@modrinth/ui'

import type { MagneticBarPage, MagneticWidgetId } from '@/helpers/magnetic-settings'

/** 面板里九张卡的标题。 */
export const magneticWidgetMessages = defineMessages({
    versions: { id: 'app.magnetic.widget.versions', defaultMessage: 'Instances' },
    shortcuts: { id: 'app.magnetic.widget.shortcuts', defaultMessage: 'Shortcuts' },
    clock: { id: 'app.magnetic.widget.clock', defaultMessage: 'Clock' },
    date: { id: 'app.magnetic.widget.date', defaultMessage: 'Date' },
    calendar: { id: 'app.magnetic.widget.calendar', defaultMessage: 'Calendar' },
    system: { id: 'app.magnetic.widget.system', defaultMessage: 'System' },
    server: { id: 'app.magnetic.widget.server', defaultMessage: 'Server' },
    lyrics: { id: 'app.magnetic.widget.lyrics', defaultMessage: 'Now playing' },
    downloads: { id: 'app.magnetic.widget.downloads', defaultMessage: 'Downloads' },
}) satisfies Record<MagneticWidgetId, { id: string; defaultMessage: string }>

/** 条子上三个页面的名字。 */
export const magneticBarPageMessages = defineMessages({
    lyrics: { id: 'app.magnetic.page.lyrics', defaultMessage: 'Lyrics' },
    launch: { id: 'app.magnetic.page.launch', defaultMessage: 'Launch' },
    system: { id: 'app.magnetic.page.system', defaultMessage: 'System' },
}) satisfies Record<MagneticBarPage, { id: string; defaultMessage: string }>

/**
 * 搜索别名（不进 i18n，纯 ASCII + 拼音，任何界面语言下都能命中）。
 *
 * 中文名走 `magneticWidgetMessages` 的译文参与匹配，这里补的是英文与拼音。
 */
export const MAGNETIC_WIDGET_KEYWORDS: Record<MagneticWidgetId, string[]> = {
    versions: ['instance', 'instances', 'version', 'banben', 'shili'],
    shortcuts: ['shortcut', 'shortcuts', 'jump', 'kuaijie', 'tiaozhuan'],
    clock: ['clock', 'time', 'shijian'],
    date: ['date', 'riqi'],
    calendar: ['calendar', 'rili'],
    system: ['system', 'cpu', 'memory', 'ram', 'network', 'disk', 'xitong', 'ziyuan'],
    server: ['server', 'ping', 'players', 'fuwuqi'],
    lyrics: ['lyrics', 'music', 'song', 'playing', 'geci', 'yinyue'],
    downloads: ['download', 'downloads', 'progress', 'xiazai', 'jindu'],
}

export const MAGNETIC_PAGE_KEYWORDS: Record<MagneticBarPage, string[]> = {
    lyrics: ['lyrics', 'music', 'geci'],
    launch: ['launch', 'play', 'qidong'],
    system: ['system', 'cpu', 'xitong'],
}
