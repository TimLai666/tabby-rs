import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const root = process.cwd()
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')

class Subject { next () {} }

function loadClass (file, dependencies = {}) {
    const module = { exports: {} }
    const source = fs.readFileSync(path.join(root, file), 'utf8')
    const output = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(output, {
        exports: module.exports,
        module,
        console,
        require: name => {
            if (name === '@angular/core') return { Injectable: () => value => value }
            if (name === 'tabby-core') return { HostWindowService: class {
                windowShown = new Subject()
                windowCloseRequest = new Subject()
                windowMoved = new Subject()
                windowFocused = new Subject()
            } }
            if (name in dependencies) return dependencies[name]
            throw new Error('Unprovided dependency: ' + name)
        },
    })
    return module.exports
}

const { TauriHostWindowService } = loadClass('tabby-tauri/src/services/hostWindow.service.ts', {
    '../api/hostBridge': {},
})
const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(resolve => setImmediate(resolve))
}
function deferred () {
    let resolve
    let reject
    const promise = new Promise((res, rej) => { resolve = res; reject = rej })
    return { promise, resolve, reject }
}
function snapshot (overrides = {}) {
    return {
        visible: true, alwaysOnTop: false, fullscreen: false, maximized: false,
        minimized: false, focused: true,
        bounds: { x: 10, y: 20, width: 800, height: 600 }, scaleFactor: 1,
        capabilities: {
            absolutePositioning: true, docking: true, globalHotkey: true,
            opacity: false, vibrancy: false, progress: true, clipboard: true,
            dialogs: true, notifications: true,
        },
        ...overrides,
    }
}
function makeWindow (initialState = snapshot(), onGetState = () => initialState) {
    const listeners = new Map()
    const calls = []
    const bridge = {
        listen: async (event, callback) => { listeners.set(event, callback); return () => listeners.delete(event) },
        invoke: async (command, request) => {
            calls.push({ command, request })
            if (command === 'window.getState') return onGetState(calls.filter(c => c.command === command).length)
            return null
        },
    }
    const service = new TauriHostWindowService(bridge)
    return {
        service, calls, listeners,
        emit: (event, payload) => {
            const listener = listeners.get(event)
            assert.ok(listener, `listener registered for ${event}`)
            listener(payload)
        },
    }
}

const cases = []
async function test (name, body) {
    try { await body(); cases.push({ name, ok: true }) }
    catch (error) { cases.push({ name, ok: false, error }) }
}

await test('constructor initializes from window.getState', async () => {
    const expected = snapshot({ maximized: true, bounds: { x: 41, y: 52, width: 1234, height: 789 }, scaleFactor: 2 })
    const win = makeWindow(expected)
    await settle()
    assert.equal(win.calls.filter(call => call.command === 'window.getState').length, 1)
    assert.deepEqual(win.service.snapshot, expected)
})

await test('native resize refreshes flags and complete outer bounds from window.getState', async () => {
    let reply = snapshot()
    const win = makeWindow(reply, () => reply)
    await settle()
    reply = snapshot({ maximized: true, fullscreen: true, minimized: true, focused: false,
        bounds: { x: -120, y: 75, width: 1440, height: 930 }, scaleFactor: 1.5 })
    const readsBefore = win.calls.filter(call => call.command === 'window.getState').length
    win.emit('desktop:windowResized', { width: 900, height: 650 })
    await settle()
    assert.equal(win.calls.filter(call => call.command === 'window.getState').length, readsBefore + 1,
        'resize asks native window.getState for authoritative state')
    assert.deepEqual(win.service.snapshot, reply, 'flags and complete outer bounds come from native state')
    assert.notDeepEqual(win.service.snapshot.bounds, { x: 10, y: 20, width: 900, height: 650 },
        'inner resize event dimensions are not substituted for native outer bounds')
})

await test('restoring from a native maximized resize clears maximized', async () => {
    let reply = snapshot({ maximized: true, bounds: { x: 0, y: 0, width: 1600, height: 900 } })
    const win = makeWindow(reply, () => reply)
    await settle()
    reply = snapshot({ maximized: false, bounds: { x: 36, y: 48, width: 1000, height: 700 } })
    win.emit('desktop:windowResized', { width: 980, height: 680 })
    await settle()
    assert.equal(win.service.isMaximized(), false)
    assert.deepEqual(win.service.snapshot.bounds, reply.bounds)
})

await test('late older getState reply cannot overwrite a newer reply', async () => {
    const first = deferred()
    const second = deferred()
    let count = 0
    const win = makeWindow(snapshot(), () => {
        count++
        if (count === 1) return snapshot()
        return count === 2 ? first.promise : second.promise
    })
    await settle()
    win.emit('desktop:windowResized', { width: 810, height: 610 })
    win.emit('desktop:windowResized', { width: 820, height: 620 })
    await settle()
    assert.equal(count, 3, 'initialization and each resize issue a state read')
    const newer = snapshot({ maximized: true, fullscreen: true, bounds: { x: 1, y: 2, width: 1400, height: 850 }, scaleFactor: 2 })
    second.resolve(newer)
    await settle()
    const older = snapshot({ minimized: true, bounds: { x: 7, y: 8, width: 700, height: 500 }, scaleFactor: 1 })
    first.resolve(older)
    await settle()
    assert.deepEqual(win.service.snapshot, newer)
})

await test('native move refreshes complete state and rejects an older delayed reply', async () => {
    const olderRead = deferred()
    const newerRead = deferred()
    let count = 0
    const win = makeWindow(snapshot(), () => {
        count++
        if (count === 1) return snapshot()
        return count === 2 ? olderRead.promise : newerRead.promise
    })
    await settle()
    win.emit('desktop:windowResized', { width: 810, height: 610 })
    await settle()
    win.emit('desktop:windowMoved', { x: 300, y: 240 })
    await settle()
    assert.equal(count, 3, 'move requests a new authoritative state while the older read is pending')
    const latest = snapshot({ maximized: true, fullscreen: true, focused: false,
        bounds: { x: 300, y: 240, width: 1320, height: 820 }, scaleFactor: 2 })
    newerRead.resolve(latest)
    await settle()
    olderRead.resolve(snapshot({ minimized: true, bounds: { x: 2, y: 3, width: 700, height: 500 } }))
    await settle()
    assert.deepEqual(win.service.snapshot, latest, 'new move position and all native flags survive the older reply')
})

await test('native focus refreshes complete state and rejects an older delayed reply', async () => {
    const olderRead = deferred()
    const newerRead = deferred()
    let count = 0
    const win = makeWindow(snapshot(), () => {
        count++
        if (count === 1) return snapshot()
        return count === 2 ? olderRead.promise : newerRead.promise
    })
    await settle()
    win.emit('desktop:windowResized', { width: 810, height: 610 })
    await settle()
    win.emit('desktop:windowFocused', true)
    await settle()
    assert.equal(count, 3, 'focus requests a new authoritative state while the older read is pending')
    const latest = snapshot({ focused: true, maximized: true, minimized: false,
        bounds: { x: 44, y: 55, width: 1210, height: 760 }, scaleFactor: 1.5 })
    newerRead.resolve(latest)
    await settle()
    olderRead.resolve(snapshot({ focused: false, fullscreen: true,
        bounds: { x: 2, y: 3, width: 700, height: 500 } }))
    await settle()
    assert.deepEqual(win.service.snapshot, latest, 'new focus state and all native flags survive the older reply')
})

await test('failed getState retains confirmed state and handles rejection', async () => {
    const confirmed = snapshot({ maximized: true, bounds: { x: 12, y: 13, width: 1100, height: 720 } })
    const warnings = []
    const originalWarn = console.warn
    console.warn = (...args) => warnings.push(args)
    let reads = 0
    const win = makeWindow(confirmed, () => {
        reads++
        if (reads > 1) return Promise.reject(new Error('fixture getState failure'))
        return confirmed
    })
    let unhandled
    const onUnhandled = reason => { unhandled = reason }
    process.on('unhandledRejection', onUnhandled)
    try {
        await settle()
        win.service.setOpacity(0.7)
        await settle()
        await new Promise(resolve => setImmediate(resolve))
    } finally {
        process.off('unhandledRejection', onUnhandled)
        console.warn = originalWarn
    }
    assert.deepEqual(win.service.snapshot, confirmed, 'failed refresh preserves last confirmed snapshot')
    assert.equal(unhandled, undefined, 'rejected refresh is consumed')
    assert.ok(warnings.some(args => args.some(value => String(value).includes('fixture getState failure'))),
        'refresh failure is reported')
})

await test('separate bridges keep their window snapshots isolated', async () => {
    const aState = snapshot({ maximized: true, bounds: { x: 1, y: 2, width: 900, height: 700 } })
    const bState = snapshot({ fullscreen: true, bounds: { x: 100, y: 200, width: 1200, height: 800 }, scaleFactor: 2 })
    const a = makeWindow(aState)
    const b = makeWindow(bState)
    await settle()
    assert.deepEqual(a.service.snapshot, aState)
    assert.deepEqual(b.service.snapshot, bState)
    a.emit('desktop:windowResized', { width: 1, height: 1 })
    await settle()
    assert.deepEqual(b.service.snapshot, bState)
    assert.equal(a.calls.filter(call => call.command === 'window.getState').length, 2)
    assert.equal(b.calls.filter(call => call.command === 'window.getState').length, 1)
})

for (const result of cases) {
    if (result.ok) console.log(`PASS ${result.name}`)
    else console.error(`FAIL ${result.name}: ${result.error?.stack ?? result.error}`)
}
const failures = cases.filter(result => !result.ok).length
console.log(`Tauri host window state: ${cases.length - failures}/${cases.length} cases passed`)
if (failures) process.exitCode = 1
