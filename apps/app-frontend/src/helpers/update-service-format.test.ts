import assert from 'node:assert/strict'
import test from 'node:test'

import {
    formatLastOk,
    formatLatency,
    normalizeStatus,
    statusChanged,
    UNKNOWN_STATUS,
    type UpdateServiceStatus,
} from './update-service-format.ts'

test('a full payload is read as-is', () => {
    assert.deepEqual(
        normalizeStatus({
            state: 'online',
            rttMs: 123.4,
            baseUrl: 'https://v.example/',
            lastOkAt: 1700000000,
            lastFailure: null,
        }),
        {
            state: 'online',
            rttMs: 123.4,
            baseUrl: 'https://v.example/',
            lastOkAt: 1700000000,
            lastFailure: null,
        },
    )
})

test('missing fields degrade to unknown instead of throwing', () => {
    for (const payload of [null, undefined, 42, 'online', []]) {
        assert.deepEqual(normalizeStatus(payload), UNKNOWN_STATUS)
    }
    assert.deepEqual(normalizeStatus({}), UNKNOWN_STATUS)
    assert.deepEqual(normalizeStatus({ state: 'bogus', rttMs: -5, baseUrl: '' }), UNKNOWN_STATUS)
})

test('a failure category survives but a foreign one does not', () => {
    assert.equal(
        normalizeStatus({ state: 'offline', lastFailure: 'certificate' }).lastFailure,
        'certificate',
    )
    assert.equal(
        normalizeStatus({ state: 'offline', lastFailure: 'timeout' }).lastFailure,
        'timeout',
    )
    assert.equal(normalizeStatus({ state: 'offline', lastFailure: 'ENOTFOUND' }).lastFailure, null)
})

test('latency is rounded, and absent while there is no sample', () => {
    assert.equal(formatLatency(null), '--')
    assert.equal(formatLatency(0), '0 ms')
    assert.equal(formatLatency(123.4), '123 ms')
})

test('the last success timestamp is formatted, not echoed', () => {
    assert.equal(formatLastOk(null), '--')
    assert.equal(formatLastOk(Number.NaN), '--')
    assert.match(formatLastOk(1700000000, 'en-US'), /\d{1,2}:\d{2}:\d{2}/)
})

function status(overrides: Partial<UpdateServiceStatus> = {}): UpdateServiceStatus {
    return { ...UNKNOWN_STATUS, ...overrides }
}

test('events are ignored when nothing visible changed', () => {
    const base = status({ state: 'online', rttMs: 100 })

    assert.equal(statusChanged(base, status({ state: 'online', rttMs: 101 })), false)
    assert.equal(statusChanged(base, status({ state: 'online', rttMs: 140 })), true)
    assert.equal(statusChanged(base, status({ state: 'offline', rttMs: 100 })), true)
    assert.equal(
        statusChanged(base, status({ state: 'online', rttMs: 100, baseUrl: 'https://x/' })),
        true,
    )
    assert.equal(
        statusChanged(base, status({ state: 'online', rttMs: 100, lastFailure: 'timeout' })),
        true,
    )
})

test('a sample appearing or disappearing is always a change', () => {
    assert.equal(statusChanged(status({ rttMs: null }), status({ rttMs: 100 })), true)
    assert.equal(statusChanged(status({ rttMs: 100 }), status({ rttMs: null })), true)
    assert.equal(statusChanged(status({ rttMs: null }), status({ rttMs: null })), false)
})
