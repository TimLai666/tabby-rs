import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const source = readFileSync(new URL(process.argv[2] ?? './capture-windows-apps.mjs', import.meta.url), 'utf8')
const start = source.indexOf("        if (flow === 'frame') {")
const end = source.indexOf('        result.bothAliveBeforeTeardown', start)
assert.ok(start >= 0 && end > start)
const block = source.slice(start, end).replace("const { exerciseWindowsFrame } = await import('./exercise-windows-frame.mjs')", 'const exerciseWindowsFrame = exerciseBoundary')
async function run (cleanupFailed) {
    const apps = [{ role: 'reference', port: 9222 }, { role: 'native', port: 9223 }]
    const calls = []
    const result = { errors: [], unverified: [] }
    for (const app of apps) result[app.role] = { pid: 100, rootIdentity: { creationTimeUTC: 'owned' }, nativeWindow: { ok: true, data: { hwnd: '0x1234' } }, cdp: {} }
    const context = { running: new Map(apps.map(app => [app.role, { page: {} }])), deadline: Date.now() + 10000 }
    const evaluate = vm.runInNewContext(`(async () => { ${block}; return result })`, {
        apps, result, context, flow: 'frame', output: '/owned', powerShell: 'pwsh',
        path: { join: (...parts) => parts.join('/') }, mkdirSync: () => {}, checkpoint: () => {}, verifyCdpOwnership: () => {},
        exerciseBoundary: async options => {
            calls.push(options.role)
            return { ok: false, inputCleanupFailed: cleanupFailed, error: 'injected failure' }
        },
    })
    await evaluate()
    return { calls, result }
}
const stopped = await run(true)
assert.deepEqual(stopped.calls, ['reference'])
assert.ok(stopped.result.errors.some(error => error.includes('Stopped all GUI actions')))
const independent = await run(false)
assert.deepEqual(independent.calls, ['reference', 'native'])
console.log('GUI flow cleanup stop: 2 controlled checks passed')
