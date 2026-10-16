import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const source = readFileSync(new URL(process.argv[2] ?? './capture-windows-apps.mjs', import.meta.url), 'utf8')
const start = source.indexOf('        for (const app of apps) {\n            const info = await captureApp(context, app)')
const end = source.indexOf("        if (flow === 'terminal') {", start)
assert.ok(start >= 0 && end > start)
const block = source.slice(start, end).replace("const { exerciseWindowsFrame } = await import('./exercise-windows-frame.mjs')", 'const exerciseWindowsFrame = exerciseBoundary')
async function run (cleanupFailed, flow = 'frame') {
    const apps = [{ role: 'reference', port: 9222 }, { role: 'native', port: 9223 }]
    const calls = []
    const result = { errors: [], unverified: [] }
    const context = { running: new Map(), deadline: Date.now() + 10000 }
    const evaluate = vm.runInNewContext(`(async () => { ${block}; return result })`, {
        installPtyWriteSerializingProbe: async () => ({ installed: true }),
        collectPtyWriteSerializingProbe: async () => ({ restored: true }),
        apps, result, context, flow, output: '/owned', powerShell: 'pwsh',
        path: { join: (...parts) => parts.join('/') }, mkdirSync: () => {}, checkpoint: () => {}, verifyCdpOwnership: () => {},
        captureApp: async (_context, app) => {
            calls.push('launch-' + app.role)
            context.running.set(app.role, { page: {} })
            return { pid: 100, rootIdentity: { creationTimeUTC: 'owned' }, nativeWindow: { ok: true, data: { hwnd: '0x1234' } }, cdp: {} }
        },
        exerciseBoundary: async options => {
            calls.push('exercise-' + options.role)
            return { ok: false, inputCleanupFailed: cleanupFailed, error: 'injected failure' }
        },
    })
    await evaluate()
    return { calls, result }
}
const stopped = await run(true)
assert.deepEqual(stopped.calls, ['launch-reference', 'exercise-reference'])
assert.ok(stopped.result.errors.some(error => error.includes('Stopped all GUI actions')))
const independent = await run(false)
assert.deepEqual(independent.calls, ['launch-reference', 'exercise-reference', 'launch-native', 'exercise-native'])
const startup = await run(false, 'startup')
assert.deepEqual(startup.calls, ['launch-reference', 'launch-native'])
console.log('GUI flow cleanup stop and serial app input: 3 controlled checks passed')
