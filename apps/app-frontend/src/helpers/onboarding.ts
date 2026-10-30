/**
 * Onboarding state rules.
 *
 * The wizard used to be decided by `!settings.onboarded` in three different
 * places, which made "did the user finish it" and "does the UI show it" drift
 * apart: a write that never reached the database still closed the overlay, and
 * the next launch asked again. Everything now goes through one versioned
 * predicate, and a finished wizard is verified against a fresh read before the
 * overlay closes.
 */

/**
 * Bump this when the tour content changes enough to deserve a replay.
 *
 * The stored `onboarding_version` is compared against it, so raising it shows
 * the wizard exactly once more for every existing profile — no manual reset.
 */
export const CURRENT_ONBOARDING_VERSION = 1

type OnboardingSettings = {
    onboarded?: boolean | null
    onboarding_version?: number | null
    onboarding_instance_tour_completed?: boolean | null
}

/** Reads a version field defensively; a missing or bogus value counts as 0. */
function readVersion(value: unknown): number {
    return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0
}

/**
 * Whether this profile still needs the main wizard.
 *
 * A profile with no settings at all needs it; a profile at or above the current
 * version does not, even if `onboarded` was somehow written as `false` by an
 * older build — the version is the authority.
 */
export function needsOnboarding(settings: OnboardingSettings | null | undefined): boolean {
    if (!settings) return true
    return readVersion(settings.onboarding_version) < CURRENT_ONBOARDING_VERSION
}

/**
 * Whether a finished wizard is genuinely persisted.
 *
 * Both halves matter: the version proves the current build wrote it, and
 * `onboarded` proves the older field stayed consistent for anything still
 * reading it.
 */
export function verifyOnboardingPersisted(
    settings: OnboardingSettings | null | undefined,
): boolean {
    if (!settings) return false
    return !needsOnboarding(settings) && settings.onboarded === true
}

/** Whether the per-instance tour is still owed for this profile. */
export function needsInstanceTour(settings: OnboardingSettings | null | undefined): boolean {
    if (!settings) return false
    return settings.onboarding_instance_tour_completed !== true
}

/** The patch a completed main wizard writes. */
export function onboardingCompletionPatch(version = CURRENT_ONBOARDING_VERSION): {
    onboarded: true
    onboarding_version: number
} {
    return { onboarded: true, onboarding_version: version }
}
