import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const root = process.cwd()
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const failures = []
let passed = 0

if (process.argv.includes('--help')) {
    console.log(`Usage: node scripts/test-windows-caption-service.mjs

Exercises the Tauri caption service through initialize, bridge/config events,
and destruction using fake browser/bridge boundaries and a bounded RAF clock.`)
    process.exit(0)
}

const sourcePath = 'tabby-tauri/src/services/windowsCaption.service.ts'
const source = fs.readFileSync(path.join(root, sourcePath), 'utf8')
const output = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2020,
    module: ts.ModuleKind.CommonJS,
    experimentalDecorators: true,
} }).outputText

class Subject {
    observers = new Set()
    subscribe (callback) {
        this.observers.add(callback)
        return { unsubscribe: () => this.observers.delete(callback) }
    }
    next (value) { for (const callback of [...this.observers]) callback(value) }
    get size () { return this.observers.size }
}

function deferred () {
    let resolve
    let reject
    const promise = new Promise((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}

function createEnvironment ({ platform = 'Windows', ready = true, state = standardState(), listenGate, readGate, readError, listenError, rendererError } = {}) {
    let now = 0
    let nextFrame = 1
    const frames = new Map()
    const canvases = []
    const renderCalls = []
    const configChanged = new Subject()
    const readyGate = deferred()
    if (ready) readyGate.resolve(true)
    const config = {
        ready$: { toPromise: () => readyGate.promise },
        changed$: configChanged,
        store: { terminal: { colorScheme: { foreground: '#b050d0' } } },
    }
    const body = {
        children: [],
        appendChild (element) { this.children.push(element); element.parentNode = this; return element },
        removeChild (element) { this.children = this.children.filter(child => child !== element); element.parentNode = null },
    }
    const document = {
        body,
        createElement (tag) {
            assert.equal(tag, 'canvas', 'service creates only its canvas')
            const context = { clearRect () {}, getImageData () { return { data: new Uint8ClampedArray(4) } } }
            const canvas = {
                tagName: 'CANVAS', width: 300, height: 150,
                style: {}, attrs: {}, parentNode: null,
                getContext: () => context,
                setAttribute (key, value) { this.attrs[key] = value },
                getAttribute (key) { return this.attrs[key] ?? null },
                remove () { this.parentNode?.removeChild(this) },
            }
            canvases.push(canvas)
            return canvas
        },
    }
    let eventCallback
    let unlistenCount = 0
    const callOrder = []
    const bridge = {
        async listen (event, callback) {
            callOrder.push(['listen', event])
            if (listenGate) await listenGate.promise
            if (listenError) throw listenError
            eventCallback = callback
            return () => { unlistenCount++ }
        },
        async invoke (command, request) {
            callOrder.push(['invoke', command, request])
            if (readGate) await readGate.promise
            if (readError) throw readError
            if (command === 'window.getState') return state
            throw new Error(`unexpected bridge command: ${command}`)
        },
        emit (value) { eventCallback?.(value) },
    }
    const hostApp = { platform, windowsBuild: 22631 }
    const renderer = {
        renderWindowsCaption (canvas, nextState, foreground, windowsBuild) {
            if (rendererError) throw rendererError
            renderCalls.push({ canvas, state: nextState, foreground, windowsBuild, at: now })
            if (nextState?.buttons?.length) {
                canvas.style.display = ''
            } else {
                canvas.style.display = 'none'
            }
        },
    }
    const module = { exports: {} }
    vm.runInNewContext(output, {
        exports: module.exports,
        module,
        require: name => {
            if (name === '@angular/core') return { Injectable: () => target => target }
            if (name === 'tabby-core') return { ConfigService: class {}, Platform: { Windows: 'Windows', Linux: 'Linux', macOS: 'macOS', Web: 'Web' } }
            if (name.endsWith('/hostBridge')) return { HostBridge: class {} }
            if (name.endsWith('/hostApp.service')) return { TauriHostAppService: class {} }
            if (name.endsWith('/windowsCaption')) return renderer
            throw new Error(`Unprovided dependency: ${name}`)
        },
        document,
        performance: { now: () => now },
        requestAnimationFrame: callback => { const id = nextFrame++; frames.set(id, callback); return id },
        cancelAnimationFrame: id => frames.delete(id),
    }, { filename: sourcePath })
    const service = new module.exports.TauriWindowsCaptionService(bridge, config, hostApp)
    function tick (milliseconds) {
        now += milliseconds
        const due = [...frames.entries()]
        frames.clear()
        for (const [, callback] of due) callback(now)
    }
    return {
        service, bridge, config, hostApp, body, canvases, renderCalls, callOrder, frames,
        tick, get now () { return now }, get unlistenCount () { return unlistenCount },
        get eventCallback () { return eventCallback }, readyGate, configChanged,
    }
}

function standardState (overrides = {}) {
    const maximized = overrides.maximized ?? false
    return {
        visible: true, alwaysOnTop: false, fullscreen: false, maximized: false,
        minimized: false, focused: true, bounds: { x: 0, y: 0, width: 800, height: 600 },
        scaleFactor: 1, capabilities: {},
        captionControls: { scaleFactor: 1, maximized, hoverColor: '#ffffff', buttons: [
            { kind: 'minimize', x: 662, y: 1, width: 45, height: 31, hovered: false, pressed: false },
            { kind: 'maximize', x: 707, y: 1, width: 46, height: 31, hovered: false, pressed: false },
            { kind: 'close', x: 753, y: 1, width: 46, height: 31, hovered: false, pressed: false },
        ] },
        ...overrides,
    }
}

async function bounded (promise, label) {
    let timer
    try {
        return await Promise.race([
            promise,
            new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out after 500ms`)), 500) }),
        ])
    } finally { clearTimeout(timer) }
}

async function check (name, fn) {
    try {
        await fn()
        passed++
        console.log(`PASS ${name}`)
    } catch (error) {
        failures.push({ name, message: error instanceof Error ? error.message : String(error) })
        console.error(`FAIL ${name}: ${error instanceof Error ? error.message : String(error)}`)
    }
}

async function settle () { for (let i = 0; i < 8; i++) await new Promise(resolve => setImmediate(resolve)) }

await check('Windows initialize waits for config, listens before reading and renders the snapshot once', async () => {
    const initial = standardState()
    const env = createEnvironment({ state: initial })
    const initialization = env.service.initialize()
    await bounded(initialization, 'Windows initialization')
    assert.deepEqual(env.callOrder.map(item => item.slice(0, 2)), [
        ['listen', 'desktop:windowCaptionChanged'], ['invoke', 'window.getState'],
    ])
    assert.equal(env.body.children.length, 1)
    assert.equal(env.renderCalls.length, 1)
    assert.equal(env.renderCalls[0].state.maximized, initial.captionControls.maximized)
    assert.equal(env.renderCalls[0].state.buttons.length, initial.captionControls.buttons.length)
    assert.equal(env.renderCalls[0].state.buttons[1].x, initial.captionControls.buttons[1].x)
    assert.equal(env.renderCalls[0].state.buttons[1].hoverProgress, 0)
    assert.equal(env.renderCalls[0].windowsBuild, 22631)
    const canvas = env.body.children[0]
    assert.equal(canvas.getAttribute('aria-hidden'), 'true')
    assert.equal(canvas.tabIndex, -1)
    assert.equal(canvas.style.pointerEvents, 'none')
    assert.equal(canvas.style.position, 'fixed')
})

await check('non-Windows has no caption side effects after the Windows positive guard', async () => {
    const positive = createEnvironment()
    await bounded(positive.service.initialize(), 'Windows guard initialization')
    assert.equal(positive.renderCalls.length, 1, 'positive guard requires a real Windows render')
    const env = createEnvironment({ platform: 'Linux' })
    await bounded(env.service.initialize(), 'non-Windows initialization')
    assert.equal(env.callOrder.length, 0)
    assert.equal(env.body.children.length, 0)
})

await check('repeated initialize shares pending work and one owned canvas', async () => {
    const gate = deferred()
    const env = createEnvironment({ readGate: gate })
    const first = env.service.initialize()
    const second = env.service.initialize()
    await settle()
    assert.equal(env.callOrder.filter(item => item[0] === 'listen').length, 1)
    assert.equal(env.callOrder.filter(item => item[0] === 'invoke').length, 1)
    gate.resolve()
    await Promise.all([bounded(first, 'first initialize'), bounded(second, 'second initialize')])
    assert.equal(env.body.children.length, 1)
})

await check('caption event received before initial read resolves wins, including null', async () => {
    const gate = deferred()
    const env = createEnvironment({ readGate: gate, state: standardState() })
    const initialization = env.service.initialize()
    await settle()
    const eventState = standardState({ maximized: true })
    env.bridge.emit(eventState.captionControls)
    gate.resolve()
    await bounded(initialization, 'event-before-read initialization')
    assert.equal(env.renderCalls.at(-1)?.state?.maximized, true)
    const nullGate = deferred()
    const nullEnv = createEnvironment({ readGate: nullGate })
    const nullInitialization = nullEnv.service.initialize()
    await settle()
    nullEnv.bridge.emit(null)
    nullGate.resolve()
    await bounded(nullInitialization, 'null event-before-read initialization')
    assert.equal(nullEnv.renderCalls.at(-1)?.state, null)
})

await check('same geometry reuses canvas and foreground changes repaint current state', async () => {
    const env = createEnvironment()
    await bounded(env.service.initialize(), 'initialization')
    const canvas = env.body.children[0]
    env.bridge.emit(standardState().captionControls)
    env.config.store.terminal.colorScheme.foreground = '#22aa44'
    env.configChanged.next()
    assert.equal(env.body.children[0], canvas)
    assert.equal(env.callOrder.filter(item => item[0] === 'listen').length, 1, 'foreground changes do not add bridge listeners')
    assert.equal(env.configChanged.size, 1, 'foreground changes reuse the existing config subscription')
    assert.ok(env.renderCalls.some(call => call.foreground === '#22aa44'))
    assert.equal(env.body.children.length, 1)
})

await check('two windows isolate state, canvases, events and teardown', async () => {
    const left = createEnvironment()
    const right = createEnvironment()
    await Promise.all([bounded(left.service.initialize(), 'left initialization'), bounded(right.service.initialize(), 'right initialization')])
    assert.notEqual(left.body.children[0], right.body.children[0])
    left.bridge.emit(standardState({ maximized: true }).captionControls)
    assert.equal(left.renderCalls.at(-1)?.state?.maximized, true)
    assert.notEqual(right.renderCalls.at(-1)?.state?.maximized, true)
    left.service.ngOnDestroy()
    assert.equal(left.body.children.length, 0)
    assert.equal(right.body.children.length, 1)
    right.service.ngOnDestroy()
})

await check('destroy during config readiness makes late completion harmless', async () => {
    const env = createEnvironment({ ready: false })
    const initialization = env.service.initialize()
    let initializationSettled = false
    initialization.then(() => { initializationSettled = true }, () => { initializationSettled = true })
    await settle()
    assert.equal(initializationSettled, false, 'initialize remains pending until config readiness')
    env.service.ngOnDestroy()
    env.readyGate.resolve(true)
    await bounded(initialization, 'destroy-during-ready initialization')
    await settle()
    assert.equal(env.callOrder.length, 0)
    assert.equal(env.body.children.length, 0)
})

await check('pending config readiness starts the Windows lifecycle only after resolution', async () => {
    const env = createEnvironment({ ready: false })
    const initialization = env.service.initialize()
    await settle()
    assert.equal(env.callOrder.length, 0, 'bridge registration waits for config readiness')
    env.readyGate.resolve(true)
    await bounded(initialization, 'pending-config initialization')
    assert.equal(env.renderCalls.length, 1)
})

await check('destroy while listen acquisition is pending unlistens late result once', async () => {
    const gate = deferred()
    const env = createEnvironment({ listenGate: gate })
    const initialization = env.service.initialize()
    await settle()
    env.service.ngOnDestroy()
    gate.resolve()
    await bounded(initialization, 'destroy-during-listen initialization')
    assert.equal(env.unlistenCount, 1)
    assert.equal(env.body.children.length, 0)
})

await check('destroy during initial read removes owned canvas and ignores late snapshot', async () => {
    const gate = deferred()
    const env = createEnvironment({ readGate: gate })
    const initialization = env.service.initialize()
    await settle()
    env.service.ngOnDestroy()
    gate.resolve()
    await bounded(initialization, 'destroy-during-read initialization')
    assert.equal(env.body.children.length, 0)
    assert.equal(env.renderCalls.length, 0)
    assert.equal(env.unlistenCount, 1)
})

for (const failureKind of ['listen', 'read']) {
    await check(`${failureKind} error rejects unchanged and cleans subscriptions, canvas and RAF`, async () => {
        const original = new Error(`${failureKind} failed`)
        const env = createEnvironment(failureKind === 'listen' ? { listenError: original } : { readError: original })
        await assert.rejects(bounded(env.service.initialize(), `${failureKind} error cleanup`), error => error === original)
        assert.equal(env.configChanged.size, 0)
        assert.equal(env.body.children.length, 0)
        assert.equal(env.frames.size, 0)
    })
}

await check('destroy is idempotent and callbacks after destroy do nothing', async () => {
    const env = createEnvironment()
    await bounded(env.service.initialize(), 'initialization')
    const calls = env.renderCalls.length
    env.service.ngOnDestroy()
    env.service.ngOnDestroy()
    env.bridge.emit(standardState({ maximized: true }).captionControls)
    env.configChanged.next()
    assert.equal(env.unlistenCount, 1)
    assert.equal(env.renderCalls.length, calls)
    assert.equal(env.body.children.length, 0)
})

await check('null clears same canvas, cancels animation, and later state restores it', async () => {
    const env = createEnvironment()
    await bounded(env.service.initialize(), 'initialization')
    const canvas = env.body.children[0]
    const hoverState = standardState().captionControls
    hoverState.buttons[0].hovered = true
    env.bridge.emit(hoverState)
    assert.ok(env.frames.size > 0, 'hover transition schedules a frame before null')
    env.bridge.emit(null)
    assert.equal(canvas.style.display, 'none')
    assert.equal(env.frames.size, 0)
    env.bridge.emit(standardState().captionControls)
    assert.equal(env.body.children[0], canvas)
    assert.equal(canvas.style.display, '')
})

await check('hover animation reaches .75 at 75ms, settles at 150ms, and reverses over remaining distance', async () => {
    const env = createEnvironment()
    await bounded(env.service.initialize(), 'initialization')
    const hovered = standardState().captionControls
    hovered.buttons[0].hovered = true
    env.bridge.emit(hovered)
    env.tick(75)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0.75)
    const changedGeometry = standardState().captionControls
    changedGeometry.buttons[0].hovered = true
    changedGeometry.buttons[0].x += 1
    changedGeometry.buttons[0].width -= 1
    env.bridge.emit(changedGeometry)
    env.tick(75)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 1,
        'geometry-only snapshots preserve the active animation')
    const exited = standardState().captionControls
    env.bridge.emit(exited)
    env.tick(37.5)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0.5625)
    env.bridge.emit(exited)
    env.tick(150)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0)
    env.bridge.emit(hovered)
    env.tick(75)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0.75)
    env.bridge.emit(exited)
    env.tick(56.25)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0.1875,
        'partial reversal uses 150ms times remaining distance')
    env.tick(56.25)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0)
})

await check('pressed snaps animation to zero, pressed-to-hover snaps one, stable state schedules no frames', async () => {
    const env = createEnvironment()
    await bounded(env.service.initialize(), 'initialization')
    const pressed = standardState().captionControls
    pressed.buttons[0].pressed = true
    env.bridge.emit(pressed)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 0)
    const hovered = standardState().captionControls
    hovered.buttons[0].hovered = true
    hovered.buttons[0].pressed = false
    env.bridge.emit(hovered)
    assert.equal(env.renderCalls.at(-1)?.state?.buttons[0]?.hoverProgress, 1)
    env.tick(200)
    assert.equal(env.frames.size, 0)
})

await check('input snapshots are not mutated by animation', async () => {
    const env = createEnvironment()
    await bounded(env.service.initialize(), 'initialization')
    assert.equal(env.renderCalls.length, 1, 'input immutability guard requires the initial state to render')
    const event = standardState().captionControls
    event.buttons[1].hovered = true
    const before = JSON.stringify(event)
    env.bridge.emit(event)
    assert.ok(env.renderCalls.length > 1, 'event must reach the renderer')
    env.tick(75)
    assert.equal(JSON.stringify(event), before)
})

console.log(`\n${passed} passed, ${failures.length} failed`)
if (failures.length) process.exitCode = 1
