import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import test from 'node:test'
const source = readFileSync(new URL('./capture-windows-apps.mjs', import.meta.url), 'utf8')
const first = source.indexOf('async function restartCapturedApp (')
const last = source.indexOf('function prepareConsumedRestartCommand (', first)
assert.ok(first >= 0 && last > first)

test('a final rejected candidate never becomes a teardown target', async () => {
    const root = { pid: 100, creationTimeUTC: '2026-10-01T00:00:00.0000000Z' }
    const sessions = [{ pid: 130, creationUtc: 'shell-a' }, { pid: 140, creationUtc: 'shell-b' }]
    const oldTree = { processes: [root, ...sessions.map(s => ({ pid: s.pid, creationTimeUTC: s.creationUtc }))], error: null }
    const candidate = { pid: 210, parentPid: 100, creationTimeUTC: 'new', handles: [5], windows: [{ hwnd: 5, class: 'AppWindow' }] }
    const calls = []
    const page = { getByRole: () => ({ click: async () => { calls.push('click') } }) }
    const run = { page, browser: { close: async () => { calls.push('browser-close') } } }
    const info = { pid: 100, rootIdentity: root, executableSha256: 'same', initialProcessTree: oldTree }
    const restart = vm.runInNewContext(source.slice(first, last) + '; restartCapturedApp', {
        Date, Buffer, Number, JSON, Map, Error,
        mkdirSync: () => {}, writeFileSync: () => {}, path: { join: (...parts) => parts.join('/') },
        verifyCdpOwnership: () => {}, sha256File: async () => 'same',
        snapshotProcessTree: (_ps, pid) => { calls.push('snapshot-' + pid); return pid === root.pid ? oldTree : { processes: [candidate], error: null } },
        startRestartProcessObserver: async () => ({ descriptor: {}, trace: () => ({ final: false }), close: async () => ({ trace: { final: true } }) }),
        runPowerShell: () => ({ ok: true, stdout: JSON.stringify({ candidates: [candidate], oldWindows: [], sessionsRemaining: [], trackedRemaining: [], oldProcessGone: true }) }),
        proveRestartOrigin: ({ trace }) => ({ owned: !trace.final, reason: trace.final ? 'final evidence rejected' : 'provisional', chain: [] }),
        message: error => String(error),
    })
    await assert.rejects(restart({ running: new Map([['native', run]]), deadline: Date.now() + 60000 },
        { role: 'native', exe: '/owned/app.exe', port: 9223 }, info,
        { identity: { pid: root.pid, creationUtc: root.creationTimeUTC, hwnd: 1 }, page, oldWindows: [{ hwnd: 1, class: 'AppWindow' }, { hwnd: 2, class: 'AppWindow' }], sessions, directory: '/fresh', mode: 'thin' }), /origin history invalidates/)
    assert.equal(info.pid, root.pid)
    assert.equal(info.rootIdentity, root)
    assert.equal(info.initialProcessTree, oldTree)
    assert.equal(info.restartHistory, undefined)
    assert.equal(run.replaced, undefined)
    assert.ok(!calls.includes('snapshot-' + candidate.pid))
    assert.ok(!calls.includes('browser-close'))
})
