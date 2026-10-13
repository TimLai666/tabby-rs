import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const nativeRequire = createRequire(import.meta.url)
const cache = new Map()
const fixtures = new Set()
const logger = { info () {}, debug () {}, warn () {}, error () {} }
const { Subject } = nativeRequire('rxjs')
const moduleTimers = new Set()
const moduleWarnings = []
const vmConsole = Object.create(console)
vmConsole.warn = (...args) => moduleWarnings.push(args)
function moduleSetTimeout (callback, delay, ...args) {
    const timer = setTimeout(() => {
        moduleTimers.delete(timer)
        callback(...args)
    }, delay)
    moduleTimers.add(timer)
    return timer
}
function moduleClearTimeout (timer) { clearTimeout(timer); moduleTimers.delete(timer) }
function clearModuleTimers () { for (const timer of moduleTimers) clearTimeout(timer); moduleTimers.clear() }
const core = { LogService: class {}, ConfigService: class {}, ProfilesService: class {},
    PromptModalComponent: class {}, VaultService: class {} }

class Middleware {
    outputToTerminal = new Subject()
    outputToSession = new Subject()
    feedFromSession (data) { this.outputToTerminal.next(data) }
    feedFromTerminal (data) { this.outputToSession.next(data) }
    close () { this.outputToTerminal.complete(); this.outputToSession.complete() }
}
class MiddlewareStack extends Middleware {
    push () {}
    unshift () {}
}
class BaseSession {
    open = false
    middleware = new MiddlewareStack()
    output = new Subject()
    output$ = this.output.asObservable()
    constructor () { this.middleware.push() }
    setLoginScriptsOptions () { this.loginScriptProcessor = { executeUnconditionalScripts () {} } }
    emitOutput (data) { this.output.next(data.toString()) }
    releaseInitialDataBuffer () {}
    async destroy () {
        if (this.open) {
            this.open = false
            await this.gracefullyKillProcess()
        }
        this.middleware.close()
        this.output.complete()
    }
}
class TerminalStreamProcessor extends Middleware {
    start () {}
    resize () {}
}
class InputProcessor extends Middleware { constructor () { super() } }
class UTF8SplitterMiddleware extends Middleware {}

// Execute actual connector session classes and the production SerialWriteQueue
// with real RxJS. Stub framework, BaseSession/terminal middleware, and bridge.
function load (file) {
    file = path.resolve(root, file)
    if (cache.has(file)) return cache.get(file).exports
    const module = { exports: {} }
    cache.set(file, module)
    const require = name => {
        if (name === '@angular/core') return { Injector: class {}, Component: () => target => target,
            Injectable: () => target => target }
        if (name === '@ng-bootstrap/ng-bootstrap') return { NgbModal: class {}, NgbModalRef: class {} }
        if (name === 'tabby-core') return core
        if (name === 'tabby-terminal') return { BaseSession, SessionMiddleware: Middleware,
            SessionMiddlewareStack: MiddlewareStack, TerminalStreamProcessor, InputProcessor, UTF8SplitterMiddleware }
        if (name === '../../../tabby-local/src/api') return load('tabby-local/src/api.ts')
        if (name === './shellProvider.service') return { TauriSpawnRequestService: class {} }
        if (['../api/hostBridge', './profile', '../../../tabby-ssh/src/api/interfaces'].includes(name)) return {}
        if (name === './sftp') return { TauriSftpSession: class {} }
        if (name === '../services/passwordStorage.service') return { TauriPasswordStorageService: class {} }
        if (name === './hostKeyPromptModal.component') return { TauriSshHostKeyPromptModalComponent: class {} }
        if (name.startsWith('.')) return load(path.resolve(path.dirname(file), `${name}.ts`))
        return nativeRequire(name)
    }
    const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
            esModuleInterop: true, experimentalDecorators: true },
    }).outputText
    vm.runInNewContext(code, { module, exports: module.exports, require, Buffer, TextDecoder,
        TextEncoder, console: vmConsole, setTimeout: moduleSetTimeout, clearTimeout: moduleClearTimeout,
        window: { crypto: { randomUUID } } }, { filename: file })
    return module.exports
}

function deferred () {
    let resolve, reject
    const promise = new Promise((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}
async function within (promise, label) {
    let timer
    try {
        return await Promise.race([promise, new Promise((_, reject) => {
            timer = setTimeout(() => reject(new Error(`${label} exceeded 1000 ms watchdog`)), 1000)
        })])
    } finally { clearTimeout(timer) }
}
async function settle () { await Promise.resolve(); await Promise.resolve() }
function expectAll (checks) {
    const failures = []
    collectAll(failures, checks)
    if (failures.length) throw new Error(failures.join('\n'))
}
function collectAll (failures, checks) {
    for (const [label, assertion] of checks) {
        try { assertion() } catch (error) { failures.push(`${label}: ${error.message}`) }
    }
}

class Bridge {
    calls = []
    invokes = []
    active = new Set()
    nextId = 0
    closeGate = null
    constructor (protocol) { this.protocol = protocol; fixtures.add(this) }
    listen (name, callback) {
        const gate = deferred()
        const entry = { name, callback, gate, disposerCalls: 0 }
        this.calls.push(entry)
        return gate.promise.then(() => {
            this.active.add(entry)
            return () => { entry.disposerCalls++; this.active.delete(entry) }
        })
    }
    invoke (name, request) {
        this.invokes.push({ name, request })
        if (['telnet.connect', 'serial.open', 'ssh.connect'].includes(name)) {
            return Promise.resolve({ id: `${this.protocol}-${++this.nextId}`, username: 'fixture-user' })
        }
        if (this.closeGate && ['telnet.close', 'serial.close'].includes(name)) return this.closeGate.promise
        return Promise.resolve({})
    }
    release (entry) { entry.gate.resolve() }
    cleanup () {
        for (const entry of this.calls) entry.gate.resolve()
        this.closeGate?.resolve({})
        return settle().then(() => {
            for (const entry of this.calls) if (this.active.has(entry)) entry.disposerCalls++
            this.active.clear()
        })
    }
}

function makeSession (protocol, bridge) {
    const file = `tabby-tauri/src/${protocol}/session.ts`
    const Class = load(file)[protocol === 'telnet' ? 'TauriTelnetSession'
        : protocol === 'serial' ? 'TauriSerialSession' : 'TauriSshSession']
    const injector = { get: token => token === core.LogService ? { create: () => logger } : {} }
    const options = { host: '127.0.0.1', port: protocol === 'serial' ? '/fixture' : 23,
        baudRate: 115200, dataBits: 8, stopBits: 1, parity: 'none', flowControl: 'none',
        readTimeoutMs: null, reconnect: { enabled: false }, input: {}, scripts: [], forwardedPorts: [],
        encoding: 'utf-8', keepaliveInterval: 0 }
    const profile = { id: 'fixture-profile', options }
    const session = protocol === 'ssh'
        ? new Class(injector, bridge, { isEnabled: () => false }, profile, { open () {} })
        : new Class(injector, bridge, profile)
    if (protocol === 'ssh') session.connectRequest = async () => ({})
    return session
}

let sshInitializeSessionClass
function getSshInitializeSessionClass () {
    if (sshInitializeSessionClass) return sshInitializeSessionClass
    const file = path.resolve(root, 'tabby-tauri/src/ssh/tab.component.ts')
    const source = fs.readFileSync(file, 'utf8')
    const sourceFile = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
    let initializeSession
    function findMethod (node) {
        if (ts.isClassDeclaration(node) && node.name?.text === 'TauriSshTabComponent') {
            initializeSession = node.members.find(member => ts.isMethodDeclaration(member)
                && member.name.getText(sourceFile) === 'initializeSession')
        }
        ts.forEachChild(node, findMethod)
    }
    findMethod(sourceFile)
    if (!initializeSession) throw new Error('TauriSshTabComponent.initializeSession not found in source')
    const methodSource = ts.createPrinter().printNode(ts.EmitHint.Unspecified, initializeSession, sourceFile)
    const wrapper = `class ActualSshTabComponent extends ConnectableTabBoundary { ${methodSource} }\nexports.ActualSshTabComponent = ActualSshTabComponent`
    const code = ts.transpileModule(wrapper, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
            experimentalDecorators: true },
    }).outputText
    const module = { exports: {} }
    class ConnectableTabBoundary {
        async initializeSession () {}
        setSession (session) { this.session = session }
        attachSessionHandler (observable, handler) { return observable.subscribe(handler) }
    }
    vm.runInNewContext(code, { module, exports: module.exports, ConnectableTabBoundary,
        TauriSshSession: load('tabby-tauri/src/ssh/session.ts').TauriSshSession }, { filename: file })
    sshInitializeSessionClass = module.exports.ActualSshTabComponent
    return sshInitializeSessionClass
}

function makeSshTabComponent (bridge) {
    const Component = getSshInitializeSessionClass()
    const component = new Component()
    const options = { host: 'fixture.invalid', port: 22, user: 'fixture-user', input: {}, scripts: [],
        forwardedPorts: [], reconnect: { enabled: false }, keepaliveInterval: 0 }
    const injector = { get: token => token === core.LogService ? { create: () => logger } : {} }
    Object.assign(component, {
        injector, bridge, vault: {}, modals: {}, profile: { id: 'fixture-profile', options },
        size: { columns: 80, rows: 24 }, session: null, writes: [],
        retainAuthenticatedSession () {}, clearAuthPrompt () {},
        write (text) { this.writes.push(text) },
    })
    return component
}
const counts = { telnet: 4, serial: 2, ssh: 7 }
const results = []
async function check (name, regression, body) {
    const before = new Set(fixtures)
    try {
        await within(body(), name)
        results.push({ name, regression, passed: true })
        console.log(`PASS ${name}`)
    } catch (error) {
        results.push({ name, regression, passed: false })
        console.log(`FAIL ${name}: ${error?.stack ?? error}`)
    } finally {
        for (const fixture of fixtures) if (!before.has(fixture)) await fixture.cleanup()
    }
}

for (const protocol of ['telnet', 'serial']) {
    await check(`${protocol}: close before listener ACK detaches late registrations and skips native start`, true, async () => {
        const bridge = new Bridge(protocol), session = makeSession(protocol, bridge)
        const started = session.start().then(() => null, error => error)
        const entries = [...bridge.calls]
        assert.equal(entries.length, counts[protocol])
        await within(session.destroy(), `${protocol} early destroy`)
        entries.forEach(entry => bridge.release(entry))
        assert.equal(await within(started, `${protocol} start`), null)
        await settle()
        expectAll([
            ['native start skipped', () => assert.equal(bridge.invokes.some(x => x.name === `${protocol}.${protocol === 'telnet' ? 'connect' : 'open'}`), false)],
            ['late registrations detached', () => assert.equal(bridge.active.size, 0)],
            ['each disposer invoked once', () => assert.ok(entries.every(x => x.disposerCalls === 1))],
        ])
    })
}

for (const protocol of ['telnet', 'serial', 'ssh']) {
    const failureIndexes = protocol === 'serial' ? [0, 1] : protocol === 'telnet' ? [1, 2] : [1, 2, 3, 4, 5]
    for (const failedIndex of failureIndexes) {
        await check(`${protocol}: partial registration failure at ${failedIndex} cleans all successes`, true, async () => {
            const bridge = new Bridge(protocol), session = makeSession(protocol, bridge)
            const rejection = new Error(`${protocol} listen ${failedIndex}`)
            const started = session.start()
            const entries = [...bridge.calls]
            assert.equal(entries.length, counts[protocol])
            for (let i = 0; i < failedIndex; i++) bridge.release(entries[i])
            await settle()
            entries[failedIndex].gate.reject(rejection)
            assert.equal(await within(started.then(() => null, error => error), `${protocol} rejected start`), rejection)
            await within(session.destroy(), `${protocol} destroy after rejection`)
            for (let i = failedIndex + 1; i < entries.length; i++) bridge.release(entries[i])
            await settle()
            expectAll([
                ['native start skipped', () => assert.equal(bridge.invokes.some(x => /\.(connect|open)$/.test(x.name)), false)],
                ['all successful listeners detached', () => assert.equal(bridge.active.size, 0)],
                ['successful disposers invoked once', () => assert.ok(entries.filter((_, i) => i !== failedIndex).every(x => x.disposerCalls === 1))],
            ])
        })
    }
}

for (const [protocol, failedIndex] of [['telnet', 1], ['serial', 0], ['serial', 1], ['ssh', 3]]) {
    await check(`${protocol}: registration failure automatically reclaims early and late successes`, true, async () => {
        const bridge = new Bridge(protocol), session = makeSession(protocol, bridge)
        const rejection = new Error(`${protocol} automatic cleanup rejection`)
        const entries = []
        const failures = []
        let serviceMessageCompleted = false, outputCompleted = false
        session.serviceMessage$.subscribe({ complete: () => { serviceMessageCompleted = true } })
        session.output$.subscribe({ complete: () => { outputCompleted = true } })
        let started
        try {
            started = session.start()
            entries.push(...bridge.calls)
            assert.equal(entries.length, counts[protocol])
            for (let i = 0; i < failedIndex; i++) bridge.release(entries[i])
            await settle()
            entries[failedIndex].gate.reject(rejection)
            const observed = await within(started.then(() => null, error => error), `${protocol} rejected start without destroy`)
            collectAll(failures, [
                ['original registration rejection preserved', () => assert.equal(observed, rejection)],
                ['successful registrations disposed at failure', () => assert.equal(bridge.active.size, 0)],
                ['early successful disposers invoked once', () => assert.ok(entries.slice(0, failedIndex).every(x => x.disposerCalls === 1))],
                ['service messages complete before caller destroy', () => assert.equal(serviceMessageCompleted, true)],
                ['output completes before caller destroy', () => assert.equal(outputCompleted, true)],
            ])
            for (let i = failedIndex + 1; i < entries.length; i++) bridge.release(entries[i])
            await settle()
            collectAll(failures, [
                ['late successful registrations disposed', () => assert.equal(bridge.active.size, 0)],
                ['all successful disposers invoked once', () => assert.ok(entries.filter((_, i) => i !== failedIndex).every(x => x.disposerCalls === 1))],
                ['native startup skipped', () => assert.equal(bridge.invokes.some(x => /\.(connect|open)$/.test(x.name)), false)],
                ['service messages remain complete after late ACK', () => assert.equal(serviceMessageCompleted, true)],
                ['output remains complete after late ACK', () => assert.equal(outputCompleted, true)],
            ])
            if (failures.length) throw new Error(failures.join('\n'))
        } finally {
            for (const entry of entries) bridge.release(entry)
            await within(session.destroy(), `${protocol} final destroy`)
            await bridge.cleanup()
        }
    })
}

for (const protocol of ['telnet', 'serial']) {
    await check(`${protocol}: destroy detaches listeners before pending native close and ignores queued events`, true, async () => {
        const bridge = new Bridge(protocol), session = makeSession(protocol, bridge)
        const messages = [], output = []
        session.serviceMessage$.subscribe(x => messages.push(x))
        session.output$.subscribe(x => output.push(x))
        let destroyed
        const entries = []
        try {
            const started = session.start()
            entries.push(...bridge.calls)
            entries.forEach(entry => bridge.release(entry))
            await within(started, `${protocol} start`)
            session.releaseInitialDataBuffer()
            bridge.closeGate = deferred()
            destroyed = session.destroy()
            const detachedBeforeClose = bridge.active.size === 0 && entries.every(x => x.disposerCalls === 1)
            const connectionId = session.connectionId
            const queued = protocol === 'telnet'
                ? [['telnet:output', { connectionId, data: [...Buffer.from('late output')] }],
                    ['telnet:message', { connectionId, message: 'late message' }]]
                : [['serial:output', { connectionId, data: [...Buffer.from('late output')] }],
                    ['serial:connectionState', { connectionId, state: 'reconnecting' }]]
            for (const [name, event] of queued) entries.find(x => x.name === name).callback(event)
            const close = bridge.invokes.filter(x => x.name === `${protocol}.close`)
            const oneMatchingClose = close.length === 1 && close[0]?.request.id === `${protocol}-1`
            expectAll([
                ['listeners detached before pending close', () => assert.equal(detachedBeforeClose, true)],
                ['queued output ignored', () => assert.deepEqual(output, [])],
                ['queued public messages ignored', () => assert.deepEqual(messages, [])],
                ['one close sent with matching id', () => assert.equal(oneMatchingClose, true)],
            ])
        } finally {
            bridge.closeGate?.resolve({})
            if (destroyed) await within(destroyed, `${protocol} close settles in finally`)
            await within(session.destroy(), `${protocol} final destroy`)
            await bridge.cleanup()
        }
    })
}

await check('ssh: all-success close during registration remains a passing control', false, async () => {
    const bridge = new Bridge('ssh'), session = makeSession('ssh', bridge)
    const started = session.start(), entries = [...bridge.calls]
    assert.equal(entries.length, 7)
    entries.forEach(entry => bridge.release(entry))
    const destroyed = session.destroy()
    await within(destroyed, 'SSH destroy')
    await within(started, 'SSH canceled start')
    assert.equal(bridge.active.size, 0)
    assert.ok(entries.every(x => x.disposerCalls === 1))
    assert.equal(bridge.invokes.some(x => x.name === 'ssh.connect'), false)
})

await check('telnet: destroying one owner preserves another owner same-name listeners', false, async () => {
    const bridge = new Bridge('telnet'), first = makeSession('telnet', bridge), second = makeSession('telnet', bridge)
    const a = first.start(), b = second.start(), entries = [...bridge.calls]
    assert.equal(entries.length, 8)
    entries.forEach(entry => bridge.release(entry))
    await within(Promise.all([a, b]), 'two Telnet sessions start')
    await within(first.destroy(), 'first owner destroy')
    const secondEntries = entries.slice(4)
    assert.equal(bridge.active.size, 4)
    assert.ok(secondEntries.every(x => x.disposerCalls === 0 && bridge.active.has(x)))
    await within(second.destroy(), 'second owner destroy')
})

function observeSessionCompletion (session) {
    const completion = { serviceMessage: false, output: false }
    session.serviceMessage$.subscribe({ complete: () => { completion.serviceMessage = true } })
    session.output$.subscribe({ complete: () => { completion.output = true } })
    return completion
}

await check('SSH tab control: current owner reports partial listener failure details', false, async () => {
    const bridge = new Bridge('ssh'), component = makeSshTabComponent(bridge)
    const rejection = Object.assign(new Error('fixture registration failure'), { details: 'native fixture detail' })
    let initializing, session
    try {
        initializing = component.initializeSession()
        await settle()
        assert.equal(bridge.calls.length, 7)
        session = component.session
        assert.ok(session instanceof load('tabby-tauri/src/ssh/session.ts').TauriSshSession)
        const completion = observeSessionCompletion(session)
        bridge.release(bridge.calls[0])
        await settle()
        bridge.calls[1].gate.reject(rejection)
        for (let i = 2; i < bridge.calls.length; i++) bridge.release(bridge.calls[i])
        await within(initializing, 'SSH tab partial registration failure handling')
        expectAll([
            ['native detail is visible to the current owner', () => assert.ok(component.writes.join('').includes('native fixture detail'))],
            ['native SSH connect is skipped', () => assert.equal(bridge.invokes.some(x => x.name === 'ssh.connect'), false)],
            ['service-message stream completes', () => assert.equal(completion.serviceMessage, true)],
            ['output stream completes', () => assert.equal(completion.output, true)],
        ])
    } finally {
        for (const entry of bridge.calls) bridge.release(entry)
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
    }
})

await check('SSH tab control: explicit user destroy before listener failure stays silent', false, async () => {
    const bridge = new Bridge('ssh'), component = makeSshTabComponent(bridge)
    const rejection = Object.assign(new Error('fixture registration failure'), { details: 'native fixture detail' })
    let initializing, session
    try {
        initializing = component.initializeSession()
        await settle()
        assert.equal(bridge.calls.length, 7)
        session = component.session
        assert.ok(session instanceof load('tabby-tauri/src/ssh/session.ts').TauriSshSession)
        const completion = observeSessionCompletion(session)
        bridge.release(bridge.calls[0])
        await settle()
        await within(session.destroy(), 'explicit SSH session destroy')
        bridge.calls[1].gate.reject(rejection)
        for (let i = 2; i < bridge.calls.length; i++) bridge.release(bridge.calls[i])
        await within(initializing, 'SSH tab cancellation handling')
        expectAll([
            ['explicitly cancelled failure stays silent', () => assert.deepEqual(component.writes, [])],
            ['native SSH connect is skipped', () => assert.equal(bridge.invokes.some(x => x.name === 'ssh.connect'), false)],
            ['service-message stream completes', () => assert.equal(completion.serviceMessage, true)],
            ['output stream completes', () => assert.equal(completion.output, true)],
        ])
    } finally {
        for (const entry of bridge.calls) bridge.release(entry)
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
    }
})

await check('SSH tab control: failure from replaced owner stays silent and preserves replacement', false, async () => {
    const bridge = new Bridge('ssh'), replacementBridge = new Bridge('ssh')
    const component = makeSshTabComponent(bridge)
    const rejection = Object.assign(new Error('fixture registration failure'), { details: 'native fixture detail' })
    let initializing, session
    const replacement = makeSession('ssh', replacementBridge)
    try {
        initializing = component.initializeSession()
        await settle()
        assert.equal(bridge.calls.length, 7)
        session = component.session
        assert.ok(session instanceof load('tabby-tauri/src/ssh/session.ts').TauriSshSession)
        const completion = observeSessionCompletion(session)
        component.session = replacement
        bridge.release(bridge.calls[0])
        await settle()
        bridge.calls[1].gate.reject(rejection)
        for (let i = 2; i < bridge.calls.length; i++) bridge.release(bridge.calls[i])
        await within(initializing, 'replaced SSH owner error handling')
        expectAll([
            ['stale owner failure is not written into replacement tab', () => assert.deepEqual(component.writes, [])],
            ['replacement session remains current', () => assert.equal(component.session, replacement)],
            ['native SSH connect is skipped', () => assert.equal(bridge.invokes.some(x => x.name === 'ssh.connect'), false)],
            ['service-message stream completes', () => assert.equal(completion.serviceMessage, true)],
            ['output stream completes', () => assert.equal(completion.output, true)],
        ])
    } finally {
        for (const entry of bridge.calls) bridge.release(entry)
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
        await replacementBridge.cleanup()
    }
})

class PtyBridge {
    calls = []
    invokes = []
    active = new Set()
    callWaiters = []
    invokeWaiters = []
    attachGate = null
    spawnResponse = { id: 'pty-startup', pid: 330 }
    existsResult = true
    pidResult = 330
    discardGate = null
    discardError = null
    listen (name, callback) {
        const gate = deferred()
        const entry = { name, callback, gate, disposerCalls: 0 }
        this.calls.push(entry)
        for (const waiter of this.callWaiters.splice(0)) if (this.calls.length >= waiter.count) waiter.resolve()
        return gate.promise.then(() => {
            this.active.add(entry)
            return () => { entry.disposerCalls++; this.active.delete(entry) }
        })
    }
    invoke (name, request) {
        this.invokes.push({ name, request })
        for (const waiter of this.invokeWaiters.splice(0)) if (this.invokes.length >= waiter.count) waiter.resolve()
        if (name === 'pty.attach' && this.attachGate) return this.attachGate.promise
        if (name === 'pty.spawn') return Promise.resolve(this.spawnResponse)
        if (name === 'pty.exists') return Promise.resolve(this.existsResult)
        if (name === 'pty.getPid') return Promise.resolve(this.pidResult)
        if (name === 'pty.discard') {
            if (this.discardGate) return this.discardGate.promise
            if (this.discardError) return Promise.reject(this.discardError)
        }
        return Promise.resolve({})
    }
    waitForCalls (count) {
        if (this.calls.length >= count) return Promise.resolve()
        const gate = deferred()
        this.callWaiters.push({ count, resolve: gate.resolve })
        return gate.promise
    }
    waitForInvokes (count) {
        if (this.invokes.length >= count) return Promise.resolve()
        const gate = deferred()
        this.invokeWaiters.push({ count, resolve: gate.resolve })
        return gate.promise
    }
    async cleanup () {
        for (const entry of this.calls) entry.gate.resolve()
        this.attachGate?.resolve({})
        await settle()
        this.active.clear()
        this.callWaiters.splice(0).forEach(x => x.resolve())
        this.invokeWaiters.splice(0).forEach(x => x.resolve())
        clearModuleTimers()
    }
}

function ptyClass () {
    return load('tabby-tauri/src/services/ptyInterface.service.ts').TauriPTYProxy
}
function ptyInterface (bridge, prepare = async options => options) {
    const Interface = load('tabby-tauri/src/services/ptyInterface.service.ts').TauriPTYInterface
    return new Interface({ prepare }, bridge)
}
async function ackPTYListener (bridge, absoluteIndex) {
    await within(bridge.waitForCalls(absoluteIndex + 1), `PTY listener ${absoluteIndex} registration`)
    bridge.calls[absoluteIndex].gate.resolve()
    await settle()
}

await check('PTY: later registration rejection automatically detaches prior ACK and skips attach', true, async () => {
    const bridge = new PtyBridge(), Proxy = ptyClass(), rejection = new Error('PTY exit listener rejected')
    let creating
    try {
        creating = Proxy.create({ id: 'pty-partial', pid: 321 }, bridge)
        await ackPTYListener(bridge, 0)
        await within(bridge.waitForCalls(2), 'second PTY listener')
        bridge.calls[1].gate.reject(rejection)
        const observed = await within(creating.then(() => null, error => error), 'PTY partial create rejection')
        expectAll([
            ['original listener rejection preserved', () => assert.equal(observed, rejection)],
            ['prior ACK listener disposed', () => assert.equal(bridge.active.size, 0)],
            ['prior disposer invoked once', () => assert.equal(bridge.calls[0].disposerCalls, 1)],
            ['pty.attach skipped', () => assert.equal(bridge.invokes.some(x => x.name === 'pty.attach'), false)],
        ])
    } finally {
        for (const entry of bridge.calls) entry.gate.resolve()
        await creating?.then(() => undefined, () => undefined)
        await bridge.cleanup()
    }
})

await check('PTY: attach rejection automatically detaches all four registrations', true, async () => {
    const bridge = new PtyBridge(), Proxy = ptyClass(), rejection = new Error('PTY attach rejected')
    bridge.attachGate = deferred()
    let creating
    try {
        creating = Proxy.create({ id: 'pty-attach-failure', pid: 322 }, bridge)
        for (let i = 0; i < 4; i++) await ackPTYListener(bridge, i)
        await within(bridge.waitForInvokes(1), 'PTY attach invocation')
        bridge.attachGate.reject(rejection)
        const observed = await within(creating.then(() => null, error => error), 'PTY attach rejection')
        expectAll([
            ['original attach rejection preserved', () => assert.equal(observed, rejection)],
            ['all registered PTY listeners detached', () => assert.equal(bridge.active.size, 0)],
            ['all four disposers invoked once', () => assert.ok(bridge.calls.every(x => x.disposerCalls === 1))],
            ['no detach substituted for listener cleanup', () => assert.equal(bridge.invokes.some(x => x.name === 'pty.detach'), false)],
        ])
    } finally {
        for (const entry of bridge.calls) entry.gate.resolve()
        bridge.attachGate.resolve({})
        await creating?.then(() => undefined, () => undefined)
        await bridge.cleanup()
    }
})

await check('PTY: normal create/detach is idempotent and owner-scoped control', false, async () => {
    const bridge = new PtyBridge(), Proxy = ptyClass()
    let first, second
    async function create (id, pid, firstIndex) {
        const pending = Proxy.create({ id, pid }, bridge)
        for (let i = 0; i < 4; i++) await ackPTYListener(bridge, firstIndex + i)
        return within(pending, `${id} PTY create`)
    }
    try {
        first = await create('pty-owner-a', 323, 0)
        second = await create('pty-owner-b', 324, 4)
        const firstCalls = bridge.calls.slice(0, 4), secondCalls = bridge.calls.slice(4, 8)
        const secondOutput = []
        second.subscribe('data', data => secondOutput.push(Buffer.from(data).toString()))
        first.unsubscribeAll()
        first.unsubscribeAll()
        const firstDetachCalls = bridge.invokes.filter(x => x.name === 'pty.detach' && x.request.id === 'pty-owner-a').length
        secondCalls.find(x => x.name === 'pty:output').callback({ id: 'pty-owner-b', sequence: 0, data: [...Buffer.from('live')] })
        expectAll([
            ['first owner detached exactly once', () => assert.equal(firstDetachCalls, 1)],
            ['first owner listener disposers invoked once', () => assert.ok(firstCalls.every(x => x.disposerCalls === 1))],
            ['other proxy listeners remain active', () => assert.ok(secondCalls.every(x => bridge.active.has(x) && x.disposerCalls === 0))],
            ['other owner still receives output', () => assert.deepEqual(secondOutput, ['live'])],
        ])
    } finally {
        first?.unsubscribeAll()
        second?.unsubscribeAll()
        await bridge.cleanup()
    }
})

await check('PTY: callbacks queued before listener removal cannot emit or respond after detach', true, async () => {
    const bridge = new PtyBridge(), Proxy = ptyClass()
    let creating, proxy
    try {
        creating = Proxy.create({ id: 'pty-stale-callbacks', pid: 325 }, bridge)
        for (let i = 0; i < 4; i++) await ackPTYListener(bridge, i)
        proxy = await within(creating, 'PTY create before stale-callback check')
        const events = [], warningsBefore = moduleWarnings.length
        proxy.subscribe('data', data => events.push(`data:${Buffer.from(data).toString()}`))
        proxy.subscribe('exit', (code, signal) => events.push(`exit:${code}:${signal}`))
        proxy.subscribe('close', () => events.push('close'))
        proxy.subscribe('error', error => events.push(`error:${error.code}`))
        proxy.unsubscribeAll()
        const stale = { id: 'pty-stale-callbacks' }
        bridge.calls.find(x => x.name === 'pty:output').callback({ ...stale, sequence: 0, data: [...Buffer.from('stale')] })
        bridge.calls.find(x => x.name === 'pty:exit').callback({ ...stale, exitCode: 9, signal: null })
        bridge.calls.find(x => x.name === 'pty:error').callback({ ...stale, code: 'fixture', details: 'stale' })
        bridge.calls.find(x => x.name === 'sudo:prompt').callback({ sessionId: stale.id, promptId: 'stale-prompt' })
        proxy.subscribe('data', data => events.push(`late-data:${Buffer.from(data).toString()}`))
        proxy.subscribe('exit', (code, signal) => events.push(`late-exit:${code}:${signal}`))
        proxy.subscribe('close', () => events.push('late-close'))
        proxy.subscribe('error', error => events.push(`late-error:${error.code}`))
        expectAll([
            ['stale callbacks produce no data, exit, close, or error', () => assert.deepEqual(events, [])],
            ['stale callbacks produce no warning or sudo response', () => {
                assert.equal(moduleWarnings.length, warningsBefore)
                assert.equal(bridge.invokes.some(x => x.name === 'sudo.respond'), false)
            }],
        ])
    } finally {
        proxy?.unsubscribeAll()
        await creating?.then(() => undefined, () => undefined)
        await bridge.cleanup()
    }
})

for (let failedIndex = 0; failedIndex < 4; failedIndex++) {
    await check(`PTY startup: fresh spawn listener ${failedIndex} failure awaits discard and preserves rejection`, true, async () => {
        const bridge = new PtyBridge(), rejection = new Error(`PTY startup listener ${failedIndex} rejected`)
        bridge.spawnResponse = { id: `pty-startup-listener-${failedIndex}`, pid: 330 + failedIndex }
        bridge.discardGate = deferred()
        const preparedOptions = []
        const api = ptyInterface(bridge, async options => { preparedOptions.push(options); return { prepared: true } })
        let starting, settled = false
        try {
            starting = api.spawn('fixture-command', ['arg'], { cwd: '/fixture', cols: 90, rows: 31 })
                .then(value => ({ type: 'value', value }), error => ({ type: 'error', error }))
            starting.then(() => { settled = true })
            await within(bridge.waitForInvokes(1), 'fresh PTY native spawn')
            for (let i = 0; i < failedIndex; i++) await ackPTYListener(bridge, i)
            await within(bridge.waitForCalls(failedIndex + 1), `listener ${failedIndex} registration`)
            bridge.calls[failedIndex].gate.reject(rejection)
            const beforeDiscard = await within(Promise.race([
                bridge.waitForInvokes(2).then(() => 'discard'),
                starting.then(() => 'settled'),
            ]), 'fresh PTY discard or premature startup settlement')
            assert.equal(beforeDiscard, 'discard', 'spawn must await pty.discard before rejecting')
            const discard = bridge.invokes.find(x => x.name === 'pty.discard')
            assert.ok(discard, 'fresh spawn failure must invoke pty.discard')
            assert.deepEqual({ ...discard.request }, { id: bridge.spawnResponse.id })
            expectAll([
                ['all acknowledged listeners were released', () => assert.equal(bridge.active.size, 0)],
                ['each acknowledged listener disposer ran once', () => assert.ok(bridge.calls.slice(0, failedIndex).every(x => x.disposerCalls === 1))],
            ])
            await settle()
            assert.equal(settled, false, 'startup rejection must wait for discard completion')
            bridge.discardGate.resolve(null)
            const outcome = await within(starting, 'fresh PTY startup rejection')
            expectAll([
                ['original listener rejection preserved', () => assert.equal(outcome.type === 'error' && outcome.error, rejection)],
                ['prepare received requested command and dimensions', () => {
                    assert.equal(preparedOptions.length, 1)
                    assert.equal(preparedOptions[0].command, 'fixture-command')
                    assert.deepEqual(preparedOptions[0].args, ['arg'])
                    assert.equal(preparedOptions[0].width, 90)
                    assert.equal(preparedOptions[0].height, 31)
                }],
                ['discard targets exactly the spawned id', () => assert.equal(discard.request.id, bridge.spawnResponse.id)],
            ])
        } finally {
            for (const entry of bridge.calls) entry.gate.resolve()
            bridge.discardGate.resolve(null)
            await starting?.then(() => undefined)
            await bridge.cleanup()
        }
    })
}

await check('PTY startup: attach failure awaits discard and keeps the attach error when cleanup rejects', true, async () => {
    const bridge = new PtyBridge(), attachError = new Error('native PTY attach rejected')
    const discardError = new Error('native PTY discard failed')
    bridge.spawnResponse = { id: 'pty-startup-attach-failure', pid: 340 }
    bridge.attachGate = deferred()
    bridge.discardError = discardError
    const warningsBefore = moduleWarnings.length
    let starting
    try {
        starting = ptyInterface(bridge).spawn('fixture-command', [])
            .then(value => ({ type: 'value', value }), error => ({ type: 'error', error }))
        await within(bridge.waitForInvokes(1), 'fresh PTY native spawn')
        for (let i = 0; i < 4; i++) await ackPTYListener(bridge, i)
        await within(bridge.waitForInvokes(2), 'PTY attach invocation')
        bridge.attachGate.reject(attachError)
        const beforeDiscard = await within(Promise.race([
            bridge.waitForInvokes(3).then(() => 'discard'),
            starting.then(() => 'settled'),
        ]), 'PTY discard or premature attach-failure settlement')
        assert.equal(beforeDiscard, 'discard', 'spawn must await pty.discard after attach rejection')
        const outcome = await within(starting, 'PTY attach failure after discard rejection')
        expectAll([
            ['attach rejection remains the caller error', () => assert.equal(outcome.type === 'error' && outcome.error, attachError)],
            ['discard targets the fresh PTY id', () => assert.deepEqual({ ...bridge.invokes.find(x => x.name === 'pty.discard')?.request }, { id: bridge.spawnResponse.id })],
            ['discard failure is reported as a warning', () => assert.ok(moduleWarnings.slice(warningsBefore).some(args => args.join(' ').includes(discardError.message)))],
            ['listener subscriptions were released', () => assert.equal(bridge.active.size, 0)],
            ['all four listener disposers ran once', () => assert.ok(bridge.calls.every(x => x.disposerCalls === 1))],
        ])
    } finally {
        bridge.attachGate.resolve({})
        for (const entry of bridge.calls) entry.gate.resolve()
        await starting?.then(() => undefined)
        await bridge.cleanup()
    }
})

await check('PTY startup: preparation and native spawn failures do not discard; successful spawn remains attached', false, async () => {
    const prepareError = new Error('prepare failed'), nativeSpawnError = new Error('native spawn failed')
    const prepareBridge = new PtyBridge()
    let prepared
    try {
        const api = ptyInterface(prepareBridge, async options => { prepared = options; throw prepareError })
        const outcome = await within(api.spawn('prepare-fixture', ['x']).then(() => null, error => error), 'prepare failure')
        expectAll([
            ['prepare rejection preserved', () => assert.equal(outcome, prepareError)],
            ['prepare failure did not invoke native spawn or discard', () => assert.equal(prepareBridge.invokes.some(x => ['pty.spawn', 'pty.discard'].includes(x.name)), false)],
            ['prepare received command options', () => assert.equal(prepared.command, 'prepare-fixture')],
        ])
    } finally { await prepareBridge.cleanup() }

    const spawnBridge = new PtyBridge()
    spawnBridge.invoke = function (name, request) {
        this.invokes.push({ name, request })
        for (const waiter of this.invokeWaiters.splice(0)) if (this.invokes.length >= waiter.count) waiter.resolve()
        if (name === 'pty.spawn') return Promise.reject(nativeSpawnError)
        return Promise.resolve({})
    }
    try {
        const api = ptyInterface(spawnBridge)
        const outcome = await within(api.spawn('native-fixture', []).then(() => null, error => error), 'native spawn failure')
        assert.equal(outcome, nativeSpawnError)
        assert.equal(spawnBridge.invokes.some(x => x.name === 'pty.discard'), false)
    } finally { await spawnBridge.cleanup() }

    const successBridge = new PtyBridge()
    successBridge.spawnResponse = { id: 'pty-startup-success', pid: 341 }
    let proxy
    try {
        const pending = ptyInterface(successBridge).spawn('success-fixture', [])
        await within(successBridge.waitForInvokes(1), 'successful native spawn')
        for (let i = 0; i < 4; i++) await ackPTYListener(successBridge, i)
        proxy = await within(pending, 'successful fresh PTY startup')
        expectAll([
            ['successful spawn returns its PTY proxy', () => assert.equal(proxy.getID(), successBridge.spawnResponse.id)],
            ['success does not discard the PTY', () => assert.equal(successBridge.invokes.some(x => x.name === 'pty.discard'), false)],
            ['success attaches the native PTY', () => assert.ok(successBridge.invokes.some(x => x.name === 'pty.attach' && x.request.id === proxy.getID()))],
        ])
    } finally {
        proxy?.unsubscribeAll()
        await successBridge.cleanup()
    }
})

await check('PTY restore: success, missing session, listener failure and attach failure never discard existing ids', false, async () => {
    const bridge = new PtyBridge(), api = ptyInterface(bridge)
    let proxy
    try {
        bridge.existsResult = false
        assert.equal(await within(api.restore('pty-existing-missing'), 'restore missing PTY'), null)
        bridge.existsResult = true
        bridge.pidResult = 342
        bridge.spawnResponse = { id: 'unused', pid: 0 }
        const restoring = api.restore('pty-existing-success')
        for (let i = 0; i < 4; i++) await ackPTYListener(bridge, i)
        proxy = await within(restoring, 'restore existing PTY')
        assert.equal(proxy.getID(), 'pty-existing-success')
        proxy.unsubscribeAll()
        proxy = null

        const listenerFailure = api.restore('pty-existing-listener-failure')
        await within(bridge.waitForCalls(5), 'restore listener registration')
        const restoreListenerError = new Error('restore listener failed')
        bridge.calls[4].gate.reject(restoreListenerError)
        assert.equal(await within(listenerFailure, 'restore listener error handled'), null)

        bridge.attachGate = deferred()
        const attachFailure = api.restore('pty-existing-attach-failure')
        for (let i = 5; i < 9; i++) await ackPTYListener(bridge, i)
        await within(bridge.waitForInvokes(5), 'restore attach invocation')
        bridge.attachGate.reject(new Error('restore attach failed'))
        assert.equal(await within(attachFailure, 'restore attach error handled'), null)
        expectAll([
            ['restore never discards an existing PTY', () => assert.equal(bridge.invokes.some(x => x.name === 'pty.discard'), false)],
            ['restore requests use existing ids', () => assert.ok(bridge.invokes.filter(x => x.name === 'pty.exists').every(x => x.request.id.startsWith('pty-existing-')))],
            ['restore success only attaches its existing id', () => assert.ok(bridge.invokes.some(x => x.name === 'pty.attach' && x.request.id === 'pty-existing-success'))],
        ])
    } finally {
        proxy?.unsubscribeAll()
        bridge.attachGate?.resolve({})
        for (const entry of bridge.calls) entry.gate.resolve()
        await bridge.cleanup()
    }
})

class SettingsBridge {
    calls = []
    active = new Set()
    listGate = null
    listenWaiters = []
    constructor () { fixtures.add(this) }
    invoke (name, args) {
        this.calls.push({ name, args })
        if (name === 'serial.list' && this.listGate) return this.listGate.promise
        return Promise.resolve([{ path: '/fixture', manufacturer: 'Fixture' }])
    }
    listen (name, callback) {
        const gate = deferred(), entry = { name, callback, gate, disposerCalls: 0 }
        this.calls.push(entry)
        for (const waiter of this.listenWaiters.splice(0)) waiter.resolve(entry)
        return gate.promise.then(() => {
            this.active.add(entry)
            return () => { entry.disposerCalls++; this.active.delete(entry) }
        })
    }
    waitForListen () {
        const entry = this.calls.find(x => x.name === 'serial:portsChanged')
        if (entry) return Promise.resolve(entry)
        const gate = deferred()
        this.listenWaiters.push(gate)
        return gate.promise
    }
    async cleanup () {
        this.listGate?.resolve([{ path: '/late', manufacturer: 'Fixture' }])
        for (const call of this.calls) call.gate?.resolve()
        this.listenWaiters.splice(0).forEach(x => x.resolve(null))
        await settle()
        this.active.clear()
    }
}
function settingsComponent (bridge) {
    return new (load('tabby-tauri/src/serial/profileSettings.component.ts').TauriSerialProfileSettingsComponent)(bridge)
}

await check('Serial settings: destroy during list skips listen and ignores late list', true, async () => {
    const bridge = new SettingsBridge(), component = settingsComponent(bridge)
    bridge.listGate = deferred()
    const sentinel = [{ path: '/preserved' }]
    component.ports = sentinel
    let initializing
    try {
        initializing = component.ngOnInit()
        component.ngOnDestroy()
        bridge.listGate.resolve([{ path: '/late-list' }])
        const outcome = await within(Promise.race([
            initializing.then(() => ({ type: 'initialized' }), error => ({ type: 'error', error })),
            bridge.waitForListen().then(entry => ({ type: 'listen', entry })),
        ]), 'settings list completion')
        if (outcome.type === 'listen') outcome.entry.gate.resolve()
        await within(initializing, 'settings initialization after list')
        expectAll([
            ['no listener registered after destruction', () => assert.equal(bridge.calls.filter(x => x.name === 'serial:portsChanged').length, 0)],
            ['late list result did not mutate ports', () => assert.equal(component.ports, sentinel)],
        ])
    } finally {
        bridge.listGate.resolve([])
        component.ngOnDestroy()
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
    }
})

await check('Serial settings: destroy during listen disposes late ACK exactly once', true, async () => {
    const bridge = new SettingsBridge(), component = settingsComponent(bridge)
    let initializing
    try {
        initializing = component.ngOnInit()
        const entry = await within(bridge.waitForListen(), 'settings listener registration')
        component.ngOnDestroy()
        entry.gate.resolve()
        await within(initializing, 'settings init after late listen ACK')
        expectAll([
            ['late disposer called exactly once', () => assert.equal(entry.disposerCalls, 1)],
            ['late listener no longer active', () => assert.equal(bridge.active.size, 0)],
        ])
    } finally {
        component.ngOnDestroy()
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
    }
})

await check('Serial settings: queued port-change callback after destroy is ignored', true, async () => {
    const bridge = new SettingsBridge(), component = settingsComponent(bridge)
    let initializing
    try {
        initializing = component.ngOnInit()
        const entry = await within(bridge.waitForListen(), 'settings listener registration')
        entry.gate.resolve()
        await within(initializing, 'settings initial load')
        const before = component.ports
        component.ngOnDestroy()
        entry.callback([{ path: '/stale-event' }])
        expectAll([
            ['registered listener disposed', () => assert.equal(entry.disposerCalls, 1)],
            ['late event did not mutate ports', () => assert.equal(component.ports, before)],
        ])
    } finally {
        component.ngOnDestroy()
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
    }
})

await check('Serial settings: normal list and refresh event remain a passing idempotent control', false, async () => {
    const bridge = new SettingsBridge(), component = settingsComponent(bridge)
    let initializing
    try {
        initializing = component.ngOnInit()
        const entry = await within(bridge.waitForListen(), 'settings listener registration')
        entry.gate.resolve()
        await within(initializing, 'normal settings initialization')
        assert.deepEqual(component.ports, [{ path: '/fixture', manufacturer: 'Fixture' }])
        entry.callback([{ path: '/refreshed' }])
        assert.deepEqual(component.ports, [{ path: '/refreshed' }])
        component.ngOnDestroy()
        component.ngOnDestroy()
        assert.equal(entry.disposerCalls, 1)
        assert.equal(bridge.active.size, 0)
    } finally {
        component.ngOnDestroy()
        await bridge.cleanup()
        await initializing?.then(() => undefined, () => undefined)
    }
})

const red = results.filter(x => x.regression && !x.passed).length
const unexpected = results.filter(x => x.regression && x.passed).length
const controlFailures = results.filter(x => !x.regression && !x.passed).length
console.log(`\nConnector listener lifecycle: ${results.length} cases; ${red} reproduced regressions; ${unexpected} regression cases passed; ${controlFailures} control failures`)
if (red || controlFailures) process.exitCode = 1
