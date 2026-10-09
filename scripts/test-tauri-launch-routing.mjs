import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const read = file => fs.readFileSync(new URL(file, root), 'utf8')
const logger = { info () {}, warn () {}, error () {} }
class HostAppService { logger = logger }
class CLIHandler {}
const core = { HostAppService, CLIHandler, Platform: {} }
function load (file) {
    const exports = {}
    vm.runInNewContext(ts.transpileModule(read(file), { compilerOptions: {
        target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true,
    } }).outputText, { exports, require: name => {
        if (name === '@angular/core') return { Injectable: () => target => target, Inject: () => () => {} }
        if (name === 'tabby-core') return core
        if (name === './api/cli') return { CLIHandler }
        return {}
    } })
    return exports
}
const { TauriHostAppService } = load('tabby-tauri/src/services/hostApp.service.ts')
const { LastCLIHandler } = load('tabby-core/src/cli.ts')
const context = (name, extra = {}) => ({ request: { newWindow: false, argv: { _: ['profile'], profileName: name } },
    cwd: '/fixture', secondInstance: true, parseError: null, ...extra })

for (const handled of [true, false]) {
    const calls = []
    const received = []
    const service = Object.assign(Object.create(TauriHostAppService.prototype), {
        logger, zone: { run: fn => fn() }, bridge: { invoke: async (...args) => calls.push(args) },
    })
    const fallback = new LastCLIHandler(service)
    service.injector = { get: () => [fallback, { priority: 0, firstMatchOnly: true, handle: async event => {
        received.push(event)
        return handled
    } }] }
    const launch = context('fixture')
    await service.dispatchLaunch(launch)
    assert.equal(received.length, 1, 'Second invocation must reach the CLI handlers')
    assert.equal(received[0].argv, launch.request.argv)
    assert.equal(received[0].cwd, launch.cwd)
    assert.equal(received[0].secondInstance, true)
    assert.equal(calls.length, handled ? 0 : 1, 'Only an unhandled second invocation opens a window')
    if (!handled) assert.equal(calls[0][0], 'window.new')
    await service.dispatchLaunch(context('invalid', { parseError: 'invalid launch' }))
    assert.equal(received.length, 1, 'Rejected input must not reach handlers')
    const explicit = context('explicit', { request: { newWindow: true, argv: {} } })
    await service.dispatchLaunch(explicit)
    assert.equal(calls.at(-1)[0], 'window.new')
    assert.equal(calls.at(-1)[1].launch, explicit)
    assert.equal(received.length, 1, 'Explicit new-window input is handed off once')
}
console.log('Launch handlers: second invocation, fallback, rejection, and explicit handoff passed')

// Native event payloads are wake-ups; only the invoking window may drain its queue.
let finishListen
let wake
let reads = 0
let activeReads = 0
let maxReads = 0
let failRead = false
const queued = [context('before-listener')]
const received = []
const service = new TauriHostAppService({ get: () => [{ priority: 0, handle: async event => {
    received.push(event.argv.profileName)
    return true
} }] }, { run: fn => fn() }, {
    listen: (_event, callback) => {
        wake = callback
        return new Promise(resolve => { finishListen = resolve })
    },
    invoke: async command => {
        if (command !== 'app.initialLaunch') return null
        if (failRead) {
            failRead = false
            throw new Error('fixture IPC unavailable')
        }
        reads++
        maxReads = Math.max(maxReads, ++activeReads)
        await new Promise(resolve => setImmediate(resolve))
        activeReads--
        return queued.shift() ?? null
    },
}, { platform: 'macos' })
const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(resolve => setImmediate(resolve))
}
await settle()
assert.equal(reads, 0, 'Register the listener before reading pending launches, so no wake-up can fall in between')
queued.push(context('during-registration'))
finishListen(() => {})
await settle()
assert.deepEqual(received, [], 'CLI services must be ready before any queued launch runs')
service.emitReady()
await settle()
assert.deepEqual(received, ['before-listener', 'during-registration'])
queued.push(context('after-ready'), context('next'))
wake(null)
wake(null)
await settle()
assert.deepEqual(received, ['before-listener', 'during-registration', 'after-ready', 'next'])
assert.equal(maxReads, 1, 'Overlapping wake-ups must not race to consume launch requests')
wake(null)
await settle()
assert.equal(received.length, 4, 'An empty or duplicate wake-up must not replay a launch')
queued.push(context('after-failed-read'))
failRead = true
wake(null)
await settle()
assert.equal(received.length, 4)
wake(null)
await settle()
assert.equal(received.at(-1), 'after-failed-read', 'A failed IPC read must not poison subsequent drains')
console.log('Launch readiness: registration race, delayed readiness, queued requests, and duplicate wake-ups passed')

const initialOnly = [context('listener-unavailable')]
const initialReceived = []
const failedListener = new TauriHostAppService({ get: () => [{ priority: 0, handle: async event => {
    initialReceived.push(event.argv.profileName)
    return true
} }] }, { run: fn => fn() }, {
    listen: async () => { throw new Error('fixture listener unavailable') },
    invoke: async command => command === 'app.initialLaunch' ? initialOnly.shift() ?? null : null,
}, { platform: 'macos' })
failedListener.emitReady()
await settle()
assert.deepEqual(initialReceived, ['listener-unavailable'], 'A listener failure must still permit initial launch processing')
console.log('Launch failures: later notifications recover IPC reads; listener failure preserves initial processing')
