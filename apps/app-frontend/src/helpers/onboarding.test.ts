import assert from 'node:assert/strict'
import test from 'node:test'

import {
    CURRENT_ONBOARDING_VERSION,
    needsInstanceTour,
    needsOnboarding,
    onboardingCompletionPatch,
    verifyOnboardingPersisted,
} from './onboarding.ts'

test('a profile with no settings at all needs the wizard', () => {
    assert.equal(needsOnboarding(null), true)
    assert.equal(needsOnboarding(undefined), true)
    assert.equal(needsOnboarding({}), true)
})

test('a completed wizard survives a restart', () => {
    // The regression this guards: `{onboarded: true}` closed the overlay but the
    // next launch asked again because the version was still below the current one.
    const persisted = { onboarded: true, onboarding_version: CURRENT_ONBOARDING_VERSION }

    assert.equal(needsOnboarding(persisted), false)
    assert.equal(verifyOnboardingPersisted(persisted), true)
})

test('the version decides, not the legacy boolean', () => {
    // An older build could leave `onboarded: false` next to a current version:
    // the version is the authority so the wizard does not come back.
    assert.equal(needsOnboarding({ onboarded: false, onboarding_version: 2 }), false)
    // ...and the opposite combination still needs the wizard.
    assert.equal(needsOnboarding({ onboarded: true, onboarding_version: 0 }), true)
})

test('a bogus stored version is treated as never onboarded', () => {
    for (const value of [-3, 0, Number.NaN, Number.POSITIVE_INFINITY]) {
        assert.equal(needsOnboarding({ onboarded: true, onboarding_version: value }), true)
    }
    assert.equal(needsOnboarding({ onboarded: true, onboarding_version: '1' }), true)
})

test('verification rejects a half-written state', () => {
    assert.equal(verifyOnboardingPersisted({ onboarded: false, onboarding_version: 1 }), false)
    assert.equal(verifyOnboardingPersisted({ onboarding_version: 1 }), false)
    assert.equal(verifyOnboardingPersisted(null), false)
})

test('the completion patch carries both fields', () => {
    assert.deepEqual(onboardingCompletionPatch(), {
        onboarded: true,
        onboarding_version: CURRENT_ONBOARDING_VERSION,
    })
})

test('the instance tour is owed only after the main wizard', () => {
    assert.equal(needsInstanceTour({ onboarding_instance_tour_completed: false }), true)
    assert.equal(needsInstanceTour({}), true)
    assert.equal(needsInstanceTour({ onboarding_instance_tour_completed: true }), false)
    assert.equal(needsInstanceTour(null), false)
})
