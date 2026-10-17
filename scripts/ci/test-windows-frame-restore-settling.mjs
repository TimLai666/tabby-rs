import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
const source = readFileSync(new URL('./exercise-windows-frame.mjs', import.meta.url), 'utf8')
const start = source.indexOf("        phase = 'caption restore bounds'")
const end = source.indexOf("        await pty('restored')", start)
assert.ok(start >= 0 && end > start)
const bounds = { left: 57, top: 64, right: 797, bottom: 664 }
async function run (snapshots, budget = 100000) {
    let now = 1000
    const calls = []
    const program = vm.runInNewContext('(async () => { let phase; ' + source.slice(start, end) + '; return restored })', {
        resized: { outer: bounds }, deadline: now + budget,
        Date: { now: () => now }, delay: async ms => { now += ms },
        native: action => {
            calls.push(action)
            return snapshots[Math.min(calls.length - 1, snapshots.length - 1)]
        },
    })
    try { return { result: await program(), calls, elapsed: now - 1000 } }
    catch (error) { return { error, calls, elapsed: now - 1000 } }
}
const transition = { outer: { left: -8, top: -8, right: 1032, bottom: 736 } }
const ready = { outer: bounds }
const settled = await run([transition, ready])
assert.equal(settled.error, undefined)
assert.deepEqual(settled.calls, ['Restore', 'Snapshot'])
const immediate = await run([ready])
assert.deepEqual(immediate.calls, ['Restore'])
for (const edge of Object.keys(bounds)) {
    const mismatch = await run([{ outer: { ...bounds, [edge]: bounds[edge] + 1 } }])
    assert.match(mismatch.error.message, new RegExp('restore changed the ordinary outer ' + edge))
    assert.ok(mismatch.elapsed >= 5000 && mismatch.elapsed < 5100)
    assert.equal(mismatch.calls.filter(action => action === 'Restore').length, 1)
}
const expired = await run([transition], 45000)
assert.ok(expired.error)
assert.deepEqual(expired.calls, ['Restore'])
console.log('Restore bounds settling: 7 controlled checks passed; exact four-edge rejection retained')
