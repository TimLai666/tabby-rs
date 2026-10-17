import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import test from 'node:test'

const source = readFileSync(new URL('./capture-windows-apps.mjs', import.meta.url), 'utf8')
const first = source.indexOf('async function restartCapturedApp (')
const last = source.indexOf('function prepareConsumedRestartCommand (', first)
assert.ok(first >= 0 && last > first)
const root = { pid: 100, creationTimeUTC: '2026-10-01T00:00:00.0000000Z' }
const child = { pid: 120, creationTimeUTC: '2026-10-01T00:00:01.0000000Z' }
const shellA = { pid: 130, creationTimeUTC: '2026-10-01T00:00:02.0000000Z' }
const shellB = { pid: 140, creationTimeUTC: '2026-10-01T00:00:03.0000000Z' }
const before = { processes: [root, child, shellA, shellB], error: null }

async function exercise (after, oldWindows = [{ hwnd: 1, class: 'AppWindow' }, { hwnd: 2, class: 'AppWindow' }], windows = [{ hwnd: 5, class: 'AppWindow' }]) {
    const calls = [], proofs = []
    const halt = new Error('stop after observing proof boundary')
    const page = { getByRole: () => ({ click: async () => { calls.push('click') } }) }
    const run = { page }
    const identity = { pid: root.pid, creationUtc: root.creationTimeUTC, hwnd: 1 }
    const info = { pid: root.pid, rootIdentity: root, executableSha256: 'same', initialProcessTree: before }
    let snapshots = 0, observations = 0, now = Date.now()
    const restart = vm.runInNewContext(source.slice(first, last) + '; restartCapturedApp', {
        Date: class extends Date { static now () { return now } }, Buffer, Number, JSON, Map, Error,
        delay: async ms => { calls.push('wait'); now += ms },
        mkdirSync: () => {}, writeFileSync: () => {},
        path: { join: (...parts) => parts.join('/') },
        verifyCdpOwnership: () => {}, sha256File: async () => 'same',
        snapshotProcessTree: () => { calls.push('snapshot'); return snapshots++ ? after : before },
        startRestartProcessObserver: async () => {
            calls.push('observer-ready')
            return { descriptor: {}, trace: () => ({}), close: async () => { calls.push('observer-close') } }
        },
        runPowerShell: () => {
            calls.push('query')
            const visible = typeof windows === 'function' ? windows(observations++) : windows
            return { ok: true, stdout: JSON.stringify({
                candidates: [{ pid: 210, parentPid: 120, creationTimeUTC: '2026-10-17T00:00:00.000Z', handles: visible.map(w => w.hwnd), windows: visible }],
                oldWindows: [], sessionsRemaining: [], trackedRemaining: [], oldProcessGone: true,
            }) }
        },
        proveRestartOrigin: input => { proofs.push(input); throw halt },
        message: error => String(error),
    })
    let failure
    try {
        await restart({ running: new Map([['native', run]]), deadline: Date.now() + 60000 },
            { role: 'native', exe: '/owned/app.exe', port: 9223 }, info,
            { identity, page, oldWindows, sessions: [shellA, shellB].map(s => ({ pid: s.pid, creationUtc: s.creationTimeUTC })), directory: '/fresh', mode: 'thin' })
    } catch (error) { failure = error }
    return { calls, proofs, failure, halt }
}

test('reused descendant PID in snapshot/subscription gap cannot become an owned seed', async () => {
    const r = await exercise({ processes: [root, { ...child, creationTimeUTC: '2026-10-16T00:00:00.0000000Z' }, shellA, shellB], error: null })
    assert.equal(r.failure, r.halt)
    assert.ok(!r.proofs[0].seeds.some(p => p.pid === child.pid))
    assert.deepEqual(r.calls.slice(0, 4), ['snapshot', 'observer-ready', 'snapshot', 'click'])
})
test('descendant gone at subscription acknowledgement cannot remain an owned seed', async () => {
    const r = await exercise({ processes: [root, shellA, shellB], error: null })
    assert.equal(r.failure, r.halt)
    assert.ok(!r.proofs[0].seeds.some(p => p.pid === child.pid))
})
test('unchanged descendants remain usable after the observer acknowledgement', async () => {
    const r = await exercise(before)
    assert.equal(r.failure, r.halt)
    assert.deepEqual(r.proofs[0].seeds, before.processes)
    assert.deepEqual(r.calls.slice(0, 4), ['snapshot', 'observer-ready', 'snapshot', 'click'])
})
for (const [name, after] of [
    ['root PID reused', { processes: [{ ...root, creationTimeUTC: '2026-10-16T00:00:00.0000000Z' }, child, shellA, shellB], error: null }],
    ['root missing', { processes: [child, shellA, shellB], error: null }],
    ['snapshot failed', { processes: [], error: 'CIM failed' }],
]) test(name + ' after acknowledgement stops before restart GUI input', async () => {
    const r = await exercise(after)
    assert.ok(r.failure)
    assert.ok(!r.calls.includes('click'))
    assert.equal(r.proofs.length, 0)
    assert.ok(r.calls.includes('observer-close'))
})

test('two verified application windows can coexist with owned native event/control windows', async () => {
    const r = await exercise(before, [{ hwnd: 1, class: 'AppWindow' }, { hwnd: 2, class: 'AppWindow' }, { hwnd: 3, class: 'TaoEvent' }, { hwnd: 4, class: 'Control' }])
    assert.equal(r.failure, r.halt)
    assert.equal(r.proofs.length, 1)
})


test('replacement maps the observed application class while preserving auxiliary window inventory', async () => {
    const windows = [{ hwnd: 6, class: 'TaoEvent' }, { hwnd: 5, class: 'AppWindow' }, { hwnd: 7, class: 'Control' }]
    const r = await exercise(before, undefined, windows)
    assert.equal(r.failure, r.halt)
    assert.equal(r.proofs.length, 1)
    assert.deepEqual(Array.from(r.proofs[0].candidate.handles), [5])
    assert.deepEqual(r.proofs[0].candidate.windows, windows)
})
test('replacement rejects two application windows before claiming process ownership', async () => {
    const r = await exercise(before, undefined, [{ hwnd: 5, class: 'AppWindow' }, { hwnd: 6, class: 'AppWindow' }])
    assert.match(String(r.failure), /ambiguous/)
    assert.equal(r.proofs.length, 0)
})
test('only auxiliary windows never reach ownership before the bounded observation expires', async () => {
    const r = await exercise(before, undefined, [{ hwnd: 5, class: 'TaoEvent' }])
    assert.match(String(r.failure), /UI restart did not prove replacement/)
    assert.equal(r.proofs.length, 0)
    assert.ok(r.calls.filter(x => x === 'wait').length > 1)
    assert.equal(r.calls.filter(x => x === 'click').length, 1)
    assert.equal(r.calls.filter(x => x === 'observer-close').length, 1)
})
test('a later primary window is checked for origin after waiting past auxiliary-only startup', async () => {
    const r = await exercise(before, undefined, index => index === 0 ? [{ hwnd: 6, class: 'TaoEvent' }] : [{ hwnd: 6, class: 'TaoEvent' }, { hwnd: 5, class: 'AppWindow' }])
    assert.equal(r.failure, r.halt)
    assert.equal(r.proofs.length, 1)
    assert.deepEqual(Array.from(r.proofs[0].candidate.handles), [5])
    assert.equal(r.calls.filter(x => x === 'wait').length, 1)
    assert.equal(r.calls.filter(x => x === 'click').length, 1)
})
test('missing observed primary class rejects before GUI input', async () => {
    const r = await exercise(before, [{ hwnd: 1 }, { hwnd: 2 }])
    assert.match(String(r.failure), /class/)
    assert.ok(!r.calls.includes('click'))
})
