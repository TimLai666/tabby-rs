import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import test from 'node:test'
const source = readFileSync(new URL('./restart-process-observer.mjs', import.meta.url), 'utf8')
const start = source.indexOf('    const trace = ')
const end = source.indexOf('    const close = ', start)
assert.ok(start >= 0 && end > start)
const ready = { pid: 100, creationTimeUTC: '2026-10-16T00:00:00Z', readyUtc: '2026-10-16T00:00:01Z' }
const header = { kind: 'ready', nonce: 'owned', ...ready }
const event = { kind: 'start', nonce: 'owned', pid: 200, parentPid: 100, fileTime: '134366660324682337' }
const heartbeat = { kind: 'heartbeat', nonce: 'owned', observedUtc: new Date().toISOString(), eventsCount: 1 }
function read (rows, { final = true, ended = true, suffix = '\n' } = {}) {
    const text = rows.map(row => JSON.stringify(row)).join('\n') + suffix
    const trace = vm.runInNewContext(source.slice(start, end) + '; trace', {
        spawnError: null, ended, stderr: '', ready, nonce: 'owned', child: { pid: 100 }, eventsFile: '/owned.jsonl',
        existsSync: () => true, readFileSync: () => text, Date, Error, JSON, Number,
    })
    return trace(final)
}
test('reads complete stopped history after the owned observer exits', () => {
    const actual = read([header, event, heartbeat, { kind: 'stopped', nonce: 'owned', eventsCount: 1 }])
    assert.equal(actual.events.length, 1)
    assert.equal(actual.events[0].pid, event.pid)
})
test('final read rejects missing stopped record', () => {
    assert.throws(() => read([header, event, heartbeat]), /stopped|final|complete/)
})
test('final read rejects stopped record count mismatch', () => {
    assert.throws(() => read([header, event, heartbeat, { kind: 'stopped', nonce: 'owned', eventsCount: 0 }]), /count|stopped/)
})
test('final read rejects records after stopped', () => {
    assert.throws(() => read([header, event, heartbeat, { kind: 'stopped', nonce: 'owned', eventsCount: 1 }, event]), /stopped|final|last/)
})
test('final read rejects a truncated last line', () => {
    assert.throws(() => read([header, event, heartbeat, { kind: 'stopped', nonce: 'owned', eventsCount: 1 }], { suffix: '' }), /complete|truncated|final/)
})
test('live read still rejects a stopped observer', () => {
    assert.throws(() => read([header, event, heartbeat, { kind: 'stopped', nonce: 'owned', eventsCount: 1 }], { final: false, ended: false }), /stopped/)
})
