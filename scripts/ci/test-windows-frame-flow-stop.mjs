import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const source = readFileSync(new URL(process.argv[2] ?? './capture-windows-apps.mjs', import.meta.url), 'utf8')
const start = source.indexOf('        for (const app of apps) {\n            if (flow')
const end = source.indexOf("        if (flow === 'terminal') {", start)
assert.ok(start >= 0 && end > start)
const block = source.slice(start, end).replace("const { exerciseWindowsFrame } = await import('./exercise-windows-frame.mjs')", 'const exerciseWindowsFrame = exerciseBoundary').replace("const { runWindowsFrameRestart } = await import('./run-windows-frame-restart.mjs')", 'const runWindowsFrameRestart = restartBoundary')
async function run (cleanupFailed, flow = 'frame', restartCleanupFailed = null) {
    const apps = [{ role: 'reference', port: 9222 }, { role: 'native', port: 9223 }]
    const calls = []
    const result = { errors: [], unverified: [] }
    const context = { running: new Map(), deadline: Date.now() + 10000 }
    const evaluate = vm.runInNewContext(`(async () => { ${block}; return result })`, {
        apps, result, context, flow, output: '/owned', powerShell: 'pwsh',
        prepareConsumedRestartCommand: () => {}, completeConsumedRestartCommand: async () => ({}),
        path: { join: (...parts) => parts.join('/') }, mkdirSync: () => {}, checkpoint: () => {}, verifyCdpOwnership: () => {},
        captureApp: async (_context, app) => {
            calls.push('launch-' + app.role)
            context.running.set(app.role, { page: {} })
            return { pid: 100, rootIdentity: { creationTimeUTC: 'owned' }, nativeWindow: { ok: true, data: { hwnd: '0x1234' } }, cdp: {} }
        },
        restartBoundary: async options => {
            calls.push('restart-' + options.role)
            return { ok: !restartCleanupFailed, inputCleanupFailed: restartCleanupFailed, error: 'injected restart failure' }
        },
        exerciseBoundary: async options => {
            calls.push('exercise-' + options.role)
            return { ok: restartCleanupFailed !== null, inputCleanupFailed: cleanupFailed, error: 'injected failure' }
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
const stoppedRestart = await run(false, 'frame', true)
assert.deepEqual(stoppedRestart.calls, ['launch-reference', 'exercise-reference', 'restart-reference'])
assert.ok(stoppedRestart.result.errors.some(error => error.includes('Stopped all GUI actions')))
const safeRestart = await run(false, 'frame', false)
assert.deepEqual(safeRestart.calls, ['launch-reference', 'exercise-reference', 'restart-reference', 'launch-native', 'exercise-native', 'restart-native'])
assert.ok(!safeRestart.result.errors.some(error => error.includes('Stopped all GUI actions')))
console.log('GUI flow cleanup stop and serial app input: 5 controlled checks passed')
