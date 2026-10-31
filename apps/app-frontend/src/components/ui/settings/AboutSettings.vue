<script setup lang="ts">
import {
    ChevronDownIcon,
    CodeIcon,
    ExternalIcon,
    GithubIcon,
    HeartHandshakeIcon,
    IssuesIcon,
    ScaleIcon,
    UsersIcon,
} from '@modrinth/assets'
import { Avatar, Button, defineMessages, useVIntl } from '@modrinth/ui'
import { getVersion } from '@tauri-apps/api/app'
import { inject, onMounted, ref } from 'vue'

import AfdianIcon from '@/assets/external/afdian.png'
import appLogo from '@/assets/hayenai.png'
import { BrandConfig } from '@/config'
import { contributors } from '@/data/about'
import { get as getSettings, getCurrentAppDatabasePath, getUpdateChannel } from '@/helpers/settings'

import QqChannelIcon from './QqChannelIcon.vue'
import SettingsSection from './SettingsSection.vue'

const { formatMessage } = useVIntl()
const version = ref('')
const replayOnboarding = inject<(mode: 'main' | 'instance') => Promise<void>>('replayOnboarding')

/**
 * Diagnostics loaded on demand.
 *
 * The onboarding state and the database path are the two facts needed to tell
 * "the write was lost" apart from "another build shares this data directory",
 * so they are shown instead of guessed at.
 */
const diagnostics = ref<{
    databasePath: string
    channel: string
    onboarded: boolean
    onboardingVersion: number
} | null>(null)
const diagnosticsError = ref<string | null>(null)

async function loadDiagnostics() {
    if (diagnostics.value || diagnosticsError.value) return
    try {
        const [databasePath, channel, settings] = await Promise.all([
            getCurrentAppDatabasePath(),
            getUpdateChannel(),
            getSettings(),
        ])
        diagnostics.value = {
            databasePath,
            channel,
            onboarded: settings.onboarded === true,
            onboardingVersion: settings.onboarding_version ?? 0,
        }
    } catch (error) {
        diagnosticsError.value = error instanceof Error ? error.message : String(error)
    }
}

const licenseUrl = `${BrandConfig.repositoryUrl}/blob/main/LICENSE`
const copyingUrl = `${BrandConfig.repositoryUrl}/blob/main/COPYING.md`
const thirdPartyLicensesUrl = `${BrandConfig.repositoryUrl}/tree/main/third-party/licenses`

onMounted(() => {
    void getVersion()
        .then((resolved) => {
            version.value = resolved
        })
        .catch(() => {
            // keep empty version string; do not fail the settings category
        })
})

const messages = defineMessages({
    productTitle: {
        id: 'app.settings.about.product-title',
        defaultMessage: 'About {productName}',
    },
    productDescription: {
        id: 'app.settings.about.description',
        defaultMessage: 'Your last launcher.',
    },
    version: {
        id: 'app.settings.about.version',
        defaultMessage: 'Version {version}',
    },
    modifiedFromUpstream: {
        id: 'app.settings.about.modified-from-upstream',
        defaultMessage: 'A modified version of {upstreamName}',
    },
    modifiedNotice: {
        id: 'app.settings.about.modified-notice',
        defaultMessage:
            'The appearance and branding are replaced by HaYenai; the upstream features and control layout are kept.',
    },
    replayOnboarding: {
        id: 'app.settings.about.replay-onboarding',
        defaultMessage: 'Replay tour',
    },
    developmentTeam: {
        id: 'app.settings.about.development-team',
        defaultMessage: 'Development team',
    },
    communitySupport: {
        id: 'app.settings.about.community-support',
        defaultMessage: 'Project & community',
    },
    repository: {
        id: 'app.settings.about.repository',
        defaultMessage: 'Source code',
    },
    reportIssue: {
        id: 'app.settings.about.report-issue',
        defaultMessage: 'Issues & feedback',
    },
    upstreamProject: {
        id: 'app.settings.about.upstream-project',
        defaultMessage: 'Upstream project',
    },
    qqChannel: {
        id: 'app.settings.about.qq-channel',
        defaultMessage: 'QQ channel',
    },
    afdian: {
        id: 'app.settings.about.afdian',
        defaultMessage: 'Support on Afdian',
    },
    afdianDescription: {
        id: 'app.settings.about.afdian-description',
        defaultMessage: 'Help support continued development',
    },
    licenseAttribution: {
        id: 'app.settings.about.license-attribution',
        defaultMessage: 'License & attribution',
    },
    attribution: {
        id: 'app.settings.about.attribution',
        defaultMessage: 'HaYenai is a modified version of the open-source Modrinth codebase.',
    },
    notAffiliated: {
        id: 'app.settings.about.not-affiliated',
        defaultMessage:
            'Modrinth is a trademark of Rinth, Inc. HaYenai is not affiliated with or endorsed by Rinth, Inc.',
    },
    originalSource: {
        id: 'app.settings.about.original-source',
        defaultMessage: 'Original Modrinth source',
    },
    projectLicense: {
        id: 'app.settings.about.project-license',
        defaultMessage: 'Project license (GPL-3.0)',
    },
    copyingGuidelines: {
        id: 'app.settings.about.copying-guidelines',
        defaultMessage: 'Copying guidelines',
    },
    thirdPartyLicenses: {
        id: 'app.settings.about.third-party-licenses',
        defaultMessage: 'Third-party licenses',
    },
    contributors: {
        id: 'app.settings.about.contributors',
        defaultMessage: 'Contributors',
    },
    diagnostics: {
        id: 'app.settings.about.diagnostics',
        defaultMessage: 'Diagnostics',
    },
    diagnosticsDatabase: {
        id: 'app.settings.about.diagnostics-database',
        defaultMessage: 'Database',
    },
    diagnosticsChannel: {
        id: 'app.settings.about.diagnostics-channel',
        defaultMessage: 'Update channel',
    },
    diagnosticsOnboarding: {
        id: 'app.settings.about.diagnostics-onboarding',
        defaultMessage: 'Onboarding',
    },
    diagnosticsUnavailable: {
        id: 'app.settings.about.diagnostics-unavailable',
        defaultMessage: 'Diagnostics are unavailable in this build.',
    },
    contributorsCount: {
        id: 'app.settings.about.contributors-count',
        defaultMessage: '{count, plural, one {# contributor} other {# contributors}}',
    },
})

/**
 * Project links, minus the ones the brand leaves empty. An empty `href` would
 * render an anchor that navigates to the settings page itself, so those entries
 * are dropped instead of rendered disabled.
 */
const projectLinks = [
    {
        href: BrandConfig.repositoryUrl,
        label: messages.repository,
        icon: GithubIcon,
    },
    {
        href: BrandConfig.supportUrl,
        label: messages.reportIssue,
        icon: IssuesIcon,
    },
    {
        href: BrandConfig.upstreamUrl,
        label: messages.upstreamProject,
        icon: ExternalIcon,
    },
    {
        href: BrandConfig.qqChannelUrl,
        label: messages.qqChannel,
        icon: QqChannelIcon,
    },
].filter((link) => link.href.length > 0)
</script>

<template>
    <div class="about-page flex flex-col gap-6">
        <!-- The hero has no heading, so it stays its own panel rather than a
		     `SettingsSection` (which would render no anchor without a title). -->
        <section id="settings-target-about-product" tabindex="-1" class="about-panel">
            <div class="flex flex-col items-center gap-4">
                <img
                    :src="appLogo"
                    alt=""
                    aria-hidden="true"
                    class="m-0 h-32 w-32 object-contain"
                />
                <div class="min-w-0 text-center">
                    <h2 class="m-0 text-xl font-semibold text-[var(--color-text-primary)]">
                        {{
                            formatMessage(messages.productTitle, {
                                productName: BrandConfig.productName,
                            })
                        }}
                    </h2>
                    <p class="m-0 mt-1 text-[var(--color-text-tertiary)]">
                        {{ formatMessage(messages.version, { version }) }}
                    </p>
                    <!-- Required first-screen disclosure: this is a modified
					     version of the upstream launcher, not the upstream build. -->
                    <p class="m-0 mt-2 font-semibold text-[var(--color-text-default)]">
                        {{
                            formatMessage(messages.modifiedFromUpstream, {
                                upstreamName: BrandConfig.upstreamName,
                            })
                        }}
                    </p>
                </div>
            </div>
            <p class="m-0 mt-3 text-center text-[var(--color-text-default)]">
                {{ formatMessage(messages.productDescription) }}
            </p>
            <p class="m-0 mt-2 text-center text-sm text-[var(--color-text-tertiary)]">
                {{ formatMessage(messages.modifiedNotice) }}
            </p>
        </section>

        <SettingsSection>
            <template #header>
                <h3
                    class="m-0 flex items-center gap-2 text-base font-semibold text-[var(--color-text-primary)]"
                >
                    <HeartHandshakeIcon class="size-5 text-[var(--color-text-tertiary)]" />
                    {{ formatMessage(messages.communitySupport) }}
                </h3>
            </template>
            <div class="grid gap-3 p-4 sm:grid-cols-2">
                <a
                    v-for="link in projectLinks"
                    :key="link.href"
                    :href="link.href"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="flex min-w-0 items-center gap-3 rounded-xl bg-surface-4 p-4 transition-colors hover:bg-surface-5"
                >
                    <span
                        class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-surface-2 text-[var(--color-text-primary)]"
                    >
                        <component :is="link.icon" class="size-6" />
                    </span>
                    <span class="min-w-0 flex-1 font-semibold text-[var(--color-text-primary)]">
                        {{ formatMessage(link.label) }}
                    </span>
                    <ExternalIcon class="size-5 shrink-0 text-[var(--color-text-tertiary)]" />
                </a>

                <!-- No donation page exists yet; an empty `href` would be a dead
				     link, so the card only renders once one is configured. -->
                <a
                    v-if="BrandConfig.sponsorUrl"
                    :href="BrandConfig.sponsorUrl"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="flex min-w-0 items-center gap-3 rounded-xl bg-surface-4 p-4 transition-colors hover:bg-surface-5"
                >
                    <span
                        class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-surface-2"
                    >
                        <img :src="AfdianIcon" alt="" class="size-7 object-contain" />
                    </span>
                    <span class="min-w-0 flex-1">
                        <span class="block font-semibold text-[var(--color-text-primary)]">
                            {{ formatMessage(messages.afdian) }}
                        </span>
                        <span class="block text-sm text-[var(--color-text-tertiary)]">
                            {{ formatMessage(messages.afdianDescription) }}
                        </span>
                    </span>
                    <ExternalIcon class="size-5 shrink-0 text-[var(--color-text-tertiary)]" />
                </a>
            </div>
        </SettingsSection>

        <SettingsSection>
            <template #header>
                <h3
                    class="m-0 flex items-center gap-2 text-base font-semibold text-[var(--color-text-primary)]"
                >
                    <ScaleIcon class="size-5 text-[var(--color-text-tertiary)]" />
                    {{ formatMessage(messages.licenseAttribution) }}
                </h3>
            </template>
            <div class="flex flex-col gap-3 p-4">
                <p class="m-0 text-[var(--color-text-default)]">
                    {{ formatMessage(messages.attribution) }}
                </p>
                <p class="m-0 text-[var(--color-text-default)]">
                    {{
                        formatMessage(messages.modifiedFromUpstream, {
                            upstreamName: BrandConfig.upstreamName,
                        })
                    }}:
                    <a
                        :href="BrandConfig.upstreamUrl"
                        target="_blank"
                        rel="noopener noreferrer"
                        class="text-brand hover:underline"
                        >{{ BrandConfig.upstreamUrl }}</a
                    >
                </p>
                <p class="m-0 mt-2 text-sm text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.notAffiliated) }}
                </p>
                <div class="mt-3 flex flex-wrap gap-2">
                    <a
                        :href="licenseUrl"
                        target="_blank"
                        rel="noopener noreferrer"
                        class="inline-flex items-center gap-2 rounded-lg bg-surface-4 px-3 py-2 text-sm font-semibold text-[var(--color-text-primary)] transition-colors hover:bg-surface-5"
                    >
                        {{ formatMessage(messages.projectLicense) }}
                        <ExternalIcon class="size-4 text-[var(--color-text-tertiary)]" />
                    </a>
                    <a
                        :href="copyingUrl"
                        target="_blank"
                        rel="noopener noreferrer"
                        class="inline-flex items-center gap-2 rounded-lg bg-surface-4 px-3 py-2 text-sm font-semibold text-[var(--color-text-primary)] transition-colors hover:bg-surface-5"
                    >
                        {{ formatMessage(messages.copyingGuidelines) }}
                        <ExternalIcon class="size-4 text-[var(--color-text-tertiary)]" />
                    </a>
                    <a
                        :href="thirdPartyLicensesUrl"
                        target="_blank"
                        rel="noopener noreferrer"
                        class="inline-flex items-center gap-2 rounded-lg bg-surface-4 px-3 py-2 text-sm font-semibold text-[var(--color-text-primary)] transition-colors hover:bg-surface-5"
                    >
                        {{ formatMessage(messages.thirdPartyLicenses) }}
                        <ExternalIcon class="size-4 text-[var(--color-text-tertiary)]" />
                    </a>
                    <a
                        href="https://github.com/modrinth/code"
                        target="_blank"
                        rel="noopener noreferrer"
                        class="inline-flex items-center gap-2 rounded-lg bg-surface-4 px-3 py-2 text-sm font-semibold text-[var(--color-text-primary)] transition-colors hover:bg-surface-5"
                    >
                        {{ formatMessage(messages.originalSource) }}
                        <ExternalIcon class="size-4 text-[var(--color-text-tertiary)]" />
                    </a>
                </div>
            </div>
        </SettingsSection>

        <details class="group pt-4 about-settings-details">
            <summary
                class="flex cursor-pointer list-none items-center gap-2 text-base font-semibold text-[var(--color-text-primary)] [&::-webkit-details-marker]:hidden"
            >
                <UsersIcon class="size-5 text-[var(--color-text-tertiary)]" />
                <span>{{ formatMessage(messages.contributors) }}</span>
                <span
                    class="rounded-full bg-surface-4 px-2 py-0.5 text-xs text-[var(--color-text-tertiary)]"
                >
                    {{ formatMessage(messages.contributorsCount, { count: contributors.length }) }}
                </span>
                <ChevronDownIcon
                    class="ml-auto size-5 text-[var(--color-text-tertiary)] transition-transform group-open:rotate-180"
                />
            </summary>
            <div class="mt-3 flex flex-wrap gap-2">
                <a
                    v-for="contributor in contributors"
                    :key="contributor.name"
                    :href="contributor.url"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="flex min-w-0 items-center gap-1.5 rounded-full bg-surface-4 py-1 pl-1 pr-2.5 transition-colors hover:bg-surface-5"
                >
                    <Avatar
                        :src="contributor.avatarUrl"
                        :alt="contributor.name"
                        size="1.5rem"
                        circle
                        no-shadow
                        loading="lazy"
                    />
                    <span class="truncate text-sm text-[var(--color-text-default)]">{{
                        contributor.name
                    }}</span>
                </a>
            </div>
        </details>

        <div id="settings-target-about-replay-tour" tabindex="-1" class="flex flex-wrap gap-2">
            <Button type="base" @click="replayOnboarding?.('main')">
                {{ formatMessage(messages.replayOnboarding) }}
            </Button>
        </div>

        <details class="group about-settings-details" @toggle="loadDiagnostics">
            <summary
                class="flex cursor-pointer list-none items-center gap-2 text-base font-semibold text-[var(--color-text-primary)] [&::-webkit-details-marker]:hidden"
            >
                <CodeIcon class="size-5 text-[var(--color-text-tertiary)]" />
                <span>{{ formatMessage(messages.diagnostics) }}</span>
                <ChevronDownIcon
                    class="ml-auto size-5 text-[var(--color-text-tertiary)] transition-transform group-open:rotate-180"
                />
            </summary>
            <div class="mt-3 flex flex-col gap-1 text-sm text-[var(--color-text-default)]">
                <p v-if="diagnosticsError" class="m-0 text-[var(--color-text-tertiary)]">
                    {{ formatMessage(messages.diagnosticsUnavailable) }}
                </p>
                <template v-else-if="diagnostics">
                    <p class="m-0 break-all">
                        <span class="text-[var(--color-text-tertiary)]"
                            >{{ formatMessage(messages.diagnosticsDatabase) }}:
                        </span>
                        <code>{{ diagnostics.databasePath }}</code>
                    </p>
                    <p class="m-0">
                        <span class="text-[var(--color-text-tertiary)]"
                            >{{ formatMessage(messages.diagnosticsChannel) }}:
                        </span>
                        <code>{{ diagnostics.channel }}</code>
                    </p>
                    <p class="m-0">
                        <span class="text-[var(--color-text-tertiary)]"
                            >{{ formatMessage(messages.diagnosticsOnboarding) }}:
                        </span>
                        <code
                            >onboarded={{ diagnostics.onboarded }}, onboarding_version={{
                                diagnostics.onboardingVersion
                            }}</code
                        >
                    </p>
                </template>
            </div>
        </details>
    </div>
</template>

<style scoped>
.about-settings-details {
    border-top: 1px solid
        var(--settings-divider, color-mix(in srgb, var(--surface-4) 55%, transparent));
}

.about-panel {
    padding: 1.25rem;
    border: 1px solid var(--surface-4);
    border-radius: var(--radius-md);
    background: var(--surface-3);
}
</style>
