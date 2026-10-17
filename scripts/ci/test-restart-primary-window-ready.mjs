import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
const source = readFileSync(process.argv[2] ?? new URL('./capture-windows-apps.mjs', import.meta.url), 'utf8')
const start = source.indexOf('            if (observation.candidates.length === 1) {')
const end = source.indexOf('            if (selected && observation.oldProcessGone', start)
assert.ok(start >= 0 && end > start)
const block = source.slice(start, end)
const old = { pid: 101, creationUtc: '2026-10-17T02:47:22.1157510Z' }
const app = { pid: 202, creationTimeUTC: '2026-10-17T02:55:28.1859890Z', windows: [
    { hwnd: 10, class: 'portable-sic' }, { hwnd: 11, class: 'Tao Thread Event Target' },
] }
function probe (candidate, selected, owned = true) {
    const sandbox = { observation: { candidates: [structuredClone(candidate)] }, selected,
        identity: old, primaryWindow: [{ class: 'Tauri Window' }], seeds: [], observer: { trace: () => [] },
        evidence: { ownershipProofs: [], replacement: null }, started: '2026-10-17T02:55:28.134Z',
        proveRestartOrigin: () => ({ owned }) }
    vm.runInNewContext(block, sandbox)
    return sandbox
}
let checks = 0
assert.doesNotThrow(() => probe(app), 'background-only replacement should remain pending'); checks++
let pending = probe(app)
assert.equal(pending.selected, undefined)
assert.equal(pending.evidence.replacement, null)
assert.equal(pending.evidence.ownershipProofs.length, 0); checks++
const ready = { ...app, windows: [...app.windows, { hwnd: 12, class: 'Tauri Window' }] }
const accepted = probe(ready)
assert.equal(accepted.selected.pid, app.pid)
assert.deepEqual(Array.from(accepted.selected.handles), [12]); checks++
assert.equal(probe(ready, undefined, false).selected, undefined); checks++
assert.throws(() => probe({ ...ready, windows: [...ready.windows, { hwnd: 13, class: 'Tauri Window' }] }), /identity is ambiguous/); checks++
assert.throws(() => probe({ ...ready, pid: old.pid, creationTimeUTC: old.creationUtc }), /identity is ambiguous/); checks++
assert.throws(() => probe(ready, { ...ready, pid: 303 }), /replacement identity changed/); checks++
assert.throws(() => probe(ready, { ...ready }, false), /origin evidence became invalid/); checks++
assert.throws(() => probe({ ...ready, windows: null }), /inventory is missing/); checks++
assert.equal(probe(app, { ...ready }).selected.pid, ready.pid); checks++
assert.throws(() => probe({ ...app, pid: 303 }, { ...ready }), /replacement identity changed/); checks++
console.log('Restart primary-window readiness: ' + checks + ' controlled checks passed; background windows are never adopted')
