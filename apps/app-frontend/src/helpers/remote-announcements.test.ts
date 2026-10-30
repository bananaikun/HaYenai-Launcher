import assert from 'node:assert/strict'
import test from 'node:test'

import { parseAnnouncements, toAnnouncementEntries } from './remote-announcements.ts'

test('update-service entries map onto the launcher feed model', () => {
    const entries = toAnnouncementEntries([
        {
            id: 'a1',
            title: 'Scheduled maintenance',
            body: 'The update service will restart at 03:00 UTC.',
            level: 'info',
            publishedAt: 1700000000,
            dismissible: true,
        },
    ])

    assert.equal(entries?.length, 1)
    const [entry] = parseAnnouncements(entries) ?? []
    assert.equal(entry.id, 'a1')
    assert.equal(entry.title, 'Scheduled maintenance')
    assert.equal(entry.content, 'The update service will restart at 03:00 UTC.')
    assert.equal(entry.type, 'notification')
    assert.equal(entry.priority, 'normal')
    assert.equal(entry.published_at, new Date(1700000000 * 1000).toISOString())
})

test('level controls urgency, and critical interrupts', () => {
    const parsed =
        parseAnnouncements(
            toAnnouncementEntries([
                { id: 'i', title: 'i', body: 'b', level: 'info', publishedAt: 1700000000 },
                { id: 'w', title: 'w', body: 'b', level: 'warn', publishedAt: 1700000100 },
                { id: 'c', title: 'c', body: 'b', level: 'critical', publishedAt: 1700000200 },
            ]),
        ) ?? []

    // parseAnnouncements sorts by priority then recency.
    assert.deepEqual(
        parsed.map((item) => item.priority),
        ['critical', 'high', 'normal'],
    )
    assert.equal(parsed.find((item) => item.id === 'c')?.type, 'modal')
    assert.equal(parsed.find((item) => item.id === 'w')?.type, 'notification')
})

test('an unknown level degrades to info instead of dropping the entry', () => {
    const parsed =
        parseAnnouncements(
            toAnnouncementEntries([
                { id: 'x', title: 't', body: 'b', level: 'emergency', publishedAt: 1700000000 },
            ]),
        ) ?? []

    assert.equal(parsed.length, 1)
    assert.equal(parsed[0].priority, 'normal')
})

test('entries without the fields the feed needs are dropped', () => {
    for (const bad of [
        { title: 'no id', body: 'b' },
        { id: 'no title', body: 'b' },
        { id: 'no body', title: 't' },
        { id: 'no date', title: 't', body: 'b' },
        { id: 'string date', title: 't', body: 'b', publishedAt: 'not a date' },
        null,
        42,
    ]) {
        assert.deepEqual(toAnnouncementEntries([bad]), [])
    }
})

test('an ISO timestamp is accepted as well as a unix one', () => {
    const entries = toAnnouncementEntries([
        { id: 'iso', title: 't', body: 'b', publishedAt: '2026-10-17T12:00:00.000Z' },
    ])

    assert.equal(entries?.length, 1)
    assert.equal(parseAnnouncements(entries)?.[0].published_at, '2026-10-17T12:00:00.000Z')
})

test('a non-array payload yields null so the caller keeps its cache', () => {
    assert.equal(toAnnouncementEntries(null), null)
    assert.equal(toAnnouncementEntries(undefined), null)
    assert.equal(toAnnouncementEntries({ data: [] }), null)
    assert.deepEqual(toAnnouncementEntries([]), [])
})
