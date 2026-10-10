// Regression contract for the SSH session close lifecycle, checked against
// upstream 14e2d60.
//
// Root cause under test: TauriSshSession.destroy() awaits pending native cleanup
// (connect cancellation, SFTP close, forwarding stop, and the native ssh.close)
// before it reaches BaseSession.destroy(). The fixed upstream SSHSession.destroy()
// initiates `this.ssh.disconnect()` without awaiting it, and the SSH shell's
// destroy/unref path synchronously flips BaseSession.open and emits closed/
// destroyed. The current shared reconnect awaits session.destroy(), so pending
// native cleanup also delays reconnection. The original reconnect does not await
// destruction. These checks require replacement before cleanup completes, without
// requiring identical microtask timing or testing the full rendered tab lifecycle.
//
// The reference control runs the verbatim upstream BaseSession + SSHSession +
// SSHShellSession class bodies extracted from git 14e2d60. Its shared disconnect/
// reconnect methods are extracted verbatim from the upstream
// connectableTerminalTab.component.ts. The current variant runs the real
// tabby-tauri TauriSshSession (whole source), the real TauriSftpSession.open/close
// contract, and the current shared disconnect/reconnect methods extracted verbatim.
// Only the transport/auth-preparation boundary and the UI tab host are stubbed.
//
// The reference is always green. The current variant is expected to fail before the
// implementation lands, specifically because logical destroy stays pending while
// native cleanup is outstanding. This file asserts logical lifecycle, cleanup
// initiation, and event isolation only; it does not assert wire cleanup, desktop
// parity, or the separately-recorded late-forwarding-start/cancellation race.

import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const logger = { info () {}, debug () {}, warn () {}, error () {} }
const { Subject } = createRequire(import.meta.url)('rxjs')

// --- Module loader (mirrors scripts/test-ssh-forwarding-startup.mjs) -----------
// The './sftp' boundary resolves to the real tabby-tauri/src/ssh/sftp.ts so the
// production TauriSftpSession.open/close contract runs, not a fake close.
const cache = new Map()

function load (file) {
    file = path.resolve(root, file)
    if (cache.has(file)) { return cache.get(file).exports }
    const module = { exports: {} }
    cache.set(file, module)
    const require = name => {
        if (name === '@angular/core') { return { Injector: class {}, InjectionToken: class {} } }
        if (name === '@ng-bootstrap/ng-bootstrap') { return { NgbModal: class {}, NgbModalRef: class {} } }
        if (name === 'tabby-core') {
            return {
                ...load('tabby-core/src/components/base.component.ts'), ...load('tabby-core/src/utfSplitter.ts'),
                LogService: class {}, ConfigService: class {}, ProfilesService: class {},
                PromptModalComponent: class {}, VaultService: class {}, Logger: class {},
            }
        }
        if (name === 'tabby-terminal') {
            return Object.assign({}, ...[
                'session', 'api/middleware', 'middleware/inputProcessing', 'middleware/streamProcessing', 'middleware/utf8Splitter',
            ].map(part => load(`tabby-terminal/src/${part}.ts`)))
        }
        if (name === './hostKeyPromptModal.component') { return { TauriSshHostKeyPromptModalComponent: class {} } }
        if (name === './sftp') { return load('tabby-tauri/src/ssh/sftp.ts') }
        if (name === '../services/passwordStorage.service') { return { TauriPasswordStorageService: class {} } }
        if (name === '../api/hostBridge') { return { HostBridge: class {}, TAURI_RUNTIME_INFO: {} } }
        if (name.startsWith('.')) { return load(path.resolve(path.dirname(file), `${name}.ts`)) }
        return createRequire(file)(name)
    }
    const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, esModuleInterop: true },
    }).outputText
    vm.runInNewContext(code, {
        module, exports: module.exports, require, Buffer, TextDecoder, console,
        setTimeout, clearTimeout, setInterval, clearInterval, process, window: { crypto: { randomUUID } },
    }, { filename: file })
    return module.exports
}

const { TauriSshSession } = load('tabby-tauri/src/ssh/session.ts')
load('tabby-tauri/src/ssh/sftp.ts')

const deps = {
    ...load('tabby-terminal/src/api/middleware.ts'),
    ...load('tabby-terminal/src/middleware/loginScriptProcessing.ts'),
    ...load('tabby-terminal/src/middleware/oscProcessing.ts'),
    ...load('tabby-terminal/src/middleware/utf8Splitter.ts'),
    ...load('tabby-terminal/src/middleware/inputProcessing.ts'),
}

// --- Verbatim upstream reference classes --------------------------------------
function gitShow (spec) {
    return execFileSync('git', ['show', spec], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
}

function extractClass (source, name) {
    const file = ts.createSourceFile('source.ts', source, ts.ScriptTarget.Latest, true)
    let node = null
    file.forEachChild(child => {
        if (ts.isClassDeclaration(child) && child.name?.text === name) { node = child }
    })
    assert.ok(node, `could not extract class ${name}`)
    return node.getText(file).replace(/^export\s+(abstract\s+)?/, '')
}

function extractMembers (source, className, names) {
    const file = ts.createSourceFile(`${className}.ts`, source, ts.ScriptTarget.Latest, true)
    const cls = file.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    assert.ok(cls, `class ${className} not found`)
    const printer = ts.createPrinter()
    const out = []
    for (const member of cls.members) {
        const name = member.name && (ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)) ? member.name.text : undefined
        if (!names.includes(name)) { continue }
        if (ts.isMethodDeclaration(member) || ts.isPropertyDeclaration(member)) {
            out.push(printer.printNode(ts.EmitHint.Unspecified, member, file))
        }
    }
    return out.join('\n')
}

const tokenNames = ['LogService', 'PasswordStorageService', 'NgbModal', 'HostAppService', 'NotificationsService',
    'FileProvidersService', 'ConfigService', 'TranslateService', 'SSHKnownHostsService', 'AutoPrivateKeyLocator']
const tokens = Object.fromEntries(tokenNames.map(name => [name, class {}]))
const injector = { get: () => ({ create: () => logger, store: { ssh: {} }, isEnabled: () => false }) }
const vault = { isEnabled: () => false }
const modals = { open () { throw new Error('unexpected modal') } }

const refctx = vm.createContext({
    ...deps, ...tokens, Subject, Buffer, console, logger,
    setTimeout, clearTimeout, setInterval, clearInterval, process,
})
const refBody = extractClass(gitShow('14e2d60:tabby-terminal/src/session.ts'), 'BaseSession') + '\n' +
    extractClass(gitShow('14e2d60:tabby-ssh/src/session/ssh.ts'), 'SSHSession') + '\n' +
    extractClass(gitShow('14e2d60:tabby-ssh/src/session/shell.ts'), 'SSHShellSession')
vm.runInContext(ts.transpileModule(refBody, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.None },
}).outputText, refctx)
const RefSshSession = vm.runInContext('SSHSession', refctx)
const RefShellSession = vm.runInContext('SSHShellSession', refctx)

// Shared connectable-tab methods, extracted verbatim for each variant. Only the
// UI host and session initialization boundary are hand-written below.
const TAB_METHODS = ['disconnect', 'reconnect', 'clearServiceMessagesOnConnect']
const upstreamTabMethods = extractMembers(gitShow('14e2d60:tabby-terminal/src/api/connectableTerminalTab.component.ts'),
    'ConnectableTerminalTabComponent', TAB_METHODS)
const currentTabMethods = extractMembers(fs.readFileSync(path.join(root, 'tabby-terminal/src/api/connectableTerminalTab.component.ts'), 'utf8'),
    'ConnectableTerminalTabComponent', TAB_METHODS)

function buildTabHost (methodsText, model) {
    const code = `
        class Host {
            constructor (model) {
                this.model = model
                this.isDisconnectedByHand = false
                this.profile = { options: {} }
                this.session = null
                this.frontend = {
                    resetTerminalModes () { model.events.push('resetTerminalModes') },
                    clear () { model.events.push('clear') },
                }
            }
            async initializeSession () {
                const session = this.model.createSession()
                this.session = session
                this.model.events.push('initializeSession')
                if (this.model.startSession) { await this.model.startSession(session) }
                return session
            }
            ${methodsText}
        }
        module.exports = { Host }
    `
    const module = { exports: {} }
    vm.runInNewContext(ts.transpileModule(code, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, esModuleInterop: true },
    }).outputText, { module, exports: module.exports, console, Promise, setTimeout, clearTimeout })
    return new module.exports.Host(model)
}

// --- Fixtures -----------------------------------------------------------------
function makeProfile (options = {}) {
    return {
        id: 'probe',
        options: {
            host: 'fixture.invalid', port: 22, user: 'alice', auth: 'password', password: '',
            privateKeys: [], forwardedPorts: [], environment: {}, x11: false, agentForward: false,
            keepaliveInterval: 0, keepaliveCountMax: 1, jumpHost: null,
            input: { backspace: 'backspace' }, scripts: [], ...options,
        },
    }
}

const tick = () => new Promise(resolve => setImmediate(resolve))

// Every gate registers itself so a failing flow can never leave a pending native
// operation blocking the process; runCloseContract releases leftovers in finally.
const pendingGates = new Set()
function makeGate () {
    let resolveGate
    const promise = new Promise(resolve => { resolveGate = resolve })
    const gate = { promise, release: value => { pendingGates.delete(gate); resolveGate(value) } }
    pendingGates.add(gate)
    return gate
}
function releaseAllGates () {
    for (const gate of [...pendingGates]) { gate.release() }
}

async function settledWithin (promise, ms = 40) {
    let done = false
    Promise.resolve(promise).then(() => { done = true }, () => { done = true })
    await new Promise(resolve => setTimeout(resolve, ms))
    return done
}

async function waitFor (predicate, tries = 200) {
    for (let index = 0; index < tries; index++) {
        if (predicate()) { return true }
        await tick()
    }
    return false
}

function makeBridge (handlers = {}) {
    const listeners = new Map()
    const calls = []
    let nextSessionId = 0
    const bridge = {
        calls,
        listeners,
        async listen (name, callback) {
            let set = listeners.get(name)
            if (!set) { set = new Set(); listeners.set(name, set) }
            set.add(callback)
            return () => { set.delete(callback); if (set.size === 0) { listeners.delete(name) } }
        },
        async invoke (name, request) {
            calls.push({ name, request })
            if (handlers[name]) { return handlers[name](request, calls) }
            if (name === 'ssh.connect') { return { id: `session-${++nextSessionId}`, username: 'alice', usedPrivateKey: true } }
            if (name === 'ssh.forwardingStart') { return { id: `fwd-${request.bindPort}` } }
            if (name === 'sftp.open') { return { id: request.id } }
            return {}
        },
    }
    bridge.emit = (name, event) => { for (const callback of [...(listeners.get(name) ?? [])]) { callback(event) } }
    return bridge
}

const newCurrentSession = (profile, bridge) => new TauriSshSession(injector, bridge, vault, profile, modals)
async function startCurrentSession (profile, bridge) {
    const session = newCurrentSession(profile, bridge)
    await session.start()
    return session
}

function makeReferenceSsh (profile, disconnect) {
    const ssh = new RefSshSession(injector, profile)
    ssh.ssh = { disconnect }
    ssh.openShellChannel = async () => ({ data$: new Subject(), eof$: new Subject() })
    return ssh
}

async function startReferenceShell (profile, disconnect) {
    const ssh = makeReferenceSsh(profile, disconnect)
    const shell = new RefShellSession(injector, ssh, profile)
    await shell.start()
    return shell
}
// --- Reference flows ----------------------------------------------------------
async function referenceEstablished (failures) {
    const gate = makeGate()
    let disconnects = 0
    const shell = await startReferenceShell(makeProfile(), () => { disconnects++; return gate.promise })
    shell.releaseInitialDataBuffer()
    const received = []
    shell.binaryOutput$.subscribe(data => received.push(Buffer.from(data)))
    shell.shell.data$.next(Buffer.from('before close'))
    if (Buffer.concat(received).toString() !== 'before close') { failures.push('reference established: pre-close output not delivered') }
    let closed = 0
    let destroyed = 0
    shell.closed$.subscribe(() => closed++)
    shell.destroyed$.subscribe(() => destroyed++)
    const pending = shell.destroy()
    if (!await settledWithin(pending)) { failures.push('reference established: destroy did not resolve while disconnect pending') }
    if (shell.open !== false) { failures.push('reference established: open not false') }
    if (closed !== 1) { failures.push(`reference established: closed count ${closed}`) }
    if (destroyed !== 1) { failures.push(`reference established: destroyed count ${destroyed}`) }
    if (disconnects !== 1) { failures.push(`reference established: disconnect count ${disconnects}`) }
    shell.shell.data$.next(Buffer.from('after close'))
    if (Buffer.concat(received).toString() !== 'before close') { failures.push('reference established: late output delivered after close') }
    gate.release()
    await pending
}

async function referenceForwardings (failures) {
    const profile = makeProfile({
        forwardedPorts: [
            { type: 'Local', host: '127.0.0.1', port: 8080, targetAddress: 'db', targetPort: 5432 },
            { type: 'Dynamic', host: '127.0.0.1', port: 1080 },
        ],
    })
    const gate = makeGate()
    const stops = []
    const ssh = new RefSshSession(injector, profile)
    ssh.forwardedPorts = [
        { stopLocalListener () { stops.push('local') } },
        { stopLocalListener () { stops.push('dynamic') } },
    ]
    let disconnects = 0
    let stopsAtDisconnect = null
    ssh.ssh = { disconnect () { disconnects++; stopsAtDisconnect = [...stops].sort(); return gate.promise } }
    const pending = ssh.destroy()
    if (!await settledWithin(pending)) { failures.push('reference forwardings: destroy did not resolve while disconnect pending') }
    if (disconnects !== 1) { failures.push(`reference forwardings: disconnect count ${disconnects}`) }
    if (stops.length !== 2) { failures.push(`reference forwardings: stop count ${stops.length}`) }
    if (stopsAtDisconnect?.join(',') !== 'dynamic,local') {
        failures.push(`reference forwardings: listeners not stopped before disconnect (${JSON.stringify(stopsAtDisconnect)})`)
    }
    gate.release()
    await pending
}

async function referenceSharedTab (failures) {
    {
        const gate = makeGate()
        const shell = await startReferenceShell(makeProfile(), () => gate.promise)
        let closed = 0
        let destroyed = 0
        shell.closed$.subscribe(() => closed++)
        shell.destroyed$.subscribe(() => destroyed++)
        const host = buildTabHost(upstreamTabMethods, { events: [], createSession: () => null, startSession: async () => {} })
        host.session = shell
        const pending = host.disconnect()
        if (!await settledWithin(pending)) { failures.push('reference shared disconnect: did not resolve while old disconnect pending') }
        if (shell.open !== false || closed !== 1 || destroyed !== 1) { failures.push('reference shared disconnect: logical end not reached') }
        gate.release()
        await pending
    }
    {
        const gate = makeGate()
        const shell = await startReferenceShell(makeProfile(), () => gate.promise)
        const created = []
        const host = buildTabHost(upstreamTabMethods, {
            events: [],
            createSession: () => {
                const shellSession = new RefShellSession(injector, makeReferenceSsh(makeProfile(), () => Promise.resolve()), makeProfile())
                created.push(shellSession)
                return shellSession
            },
            startSession: session => session.start(),
        })
        host.session = shell
        const pending = host.reconnect()
        if (!await settledWithin(pending)) { failures.push('reference shared reconnect: replacement did not complete while old disconnect pending') }
        if (created.length !== 1) { failures.push('reference shared reconnect: replacement not installed while old disconnect pending') }
        if (host.session !== created[0]) { failures.push('reference shared reconnect: session not replaced') }
        if (shell.open !== false) { failures.push('reference shared reconnect: old session not logically closed') }
        const replacement = created[0]
        let replacementDestroyed = 0
        if (replacement) { replacement.destroyed$.subscribe(() => replacementDestroyed++) }
        gate.release()
        await pending
        await tick()
        if (replacement && replacement.open !== true) { failures.push('reference shared reconnect: replacement not open') }
        if (replacementDestroyed !== 0) { failures.push('reference shared reconnect: old cleanup affected replacement') }
    }
}
// --- Current flows ------------------------------------------------------------
async function currentEstablished (failures) {
    const gate = makeGate()
    let closes = 0
    const bridge = makeBridge({ 'ssh.close': () => { closes++; return gate.promise } })
    const session = await startCurrentSession(makeProfile(), bridge)
    const connectionId = bridge.calls.find(call => call.name === 'ssh.connect').request.connectionId
    session.releaseInitialDataBuffer()
    const received = []
    session.binaryOutput$.subscribe(data => received.push(Buffer.from(data)))
    bridge.emit('ssh:output', { connectionId, data: Array.from(Buffer.from('before close')), extended: false })
    if (Buffer.concat(received).toString() !== 'before close') { failures.push('current established: pre-close output not delivered') }
    let closed = 0
    let destroyed = 0
    session.closed$.subscribe(() => closed++)
    session.destroyed$.subscribe(() => destroyed++)
    const pending = session.destroy()
    if (!await settledWithin(pending)) {
        failures.push('current established: logical destroy did not resolve while native ssh.close stays pending')
    }
    if (session.open !== false || closed !== 1 || destroyed !== 1) { failures.push('current established: logical end not reached before native cleanup completes') }
    if (bridge.listeners.size !== 0) { failures.push('current established: listeners not detached before native cleanup completes') }
    bridge.emit('ssh:output', { connectionId, data: Array.from(Buffer.alloc(2048, 0x61)), extended: false })
    if (Buffer.concat(received).toString() !== 'before close') { failures.push('current established: late output retained after destroy') }
    gate.release()
    await pending
    await tick()
    if (session.open !== false) { failures.push('current established: open not false after destroy') }
    if (closed !== 1) { failures.push(`current established: closed count ${closed}`) }
    if (destroyed !== 1) { failures.push(`current established: destroyed count ${destroyed}`) }
    if (closes !== 1) { failures.push(`current established: ssh.close count ${closes}`) }
    if (session.authUsername !== 'alice') { failures.push('current established: authenticated username not retained') }
    if (session.activePrivateKey !== true) { failures.push('current established: private-key flag not retained') }
    if (bridge.listeners.size !== 0) { failures.push(`current established: listeners not detached (${bridge.listeners.size})`) }
}

async function currentForwardings (failures) {
    const sshGate = makeGate()
    const sftpGate = makeGate()
    const forwardingGate = makeGate()
    let sftpCloses = 0
    let closes = 0
    const stops = []
    const bridge = makeBridge({
        'ssh.close': () => { closes++; return sshGate.promise },
        'sftp.close': () => { sftpCloses++; return sftpGate.promise },
        'ssh.forwardingStop': request => { stops.push(request.id); return forwardingGate.promise },
    })
    const profile = makeProfile({
        forwardedPorts: [
            { type: 'Local', host: '127.0.0.1', port: 8080, targetAddress: 'db', targetPort: 5432 },
            { type: 'Dynamic', host: '127.0.0.1', port: 1080 },
        ],
    })
    const session = await startCurrentSession(profile, bridge)
    const sftp = await session.openSFTP()
    assert.equal(sftp.id, 'session-1')
    let closed = 0
    let destroyed = 0
    session.closed$.subscribe(() => closed++)
    session.destroyed$.subscribe(() => destroyed++)
    const pending = session.destroy()
    if (!await settledWithin(pending)) {
        failures.push('current forwardings: logical destroy did not resolve while sftp.close/native close stay pending')
    }
    if (session.open !== false || closed !== 1 || destroyed !== 1) { failures.push('current forwardings: logical end not reached while all cleanup is pending') }
    if (bridge.listeners.size !== 0) { failures.push('current forwardings: listeners remain attached while cleanup is pending') }
    if (sftpCloses !== 1) { failures.push(`current forwardings: sftp.close not initiated promptly (${sftpCloses})`) }
    if ([...stops].sort().join(',') !== 'fwd-1080,fwd-8080') {
        failures.push(`current forwardings: forwarding stop ids not initiated promptly (${JSON.stringify(stops)})`)
    }
    if (closes !== 1) { failures.push(`current forwardings: native close not initiated promptly (${closes})`) }
    sftpGate.release()
    forwardingGate.release()
    sshGate.release()
    await pending
    await tick()
    if ([...stops].sort().join(',') !== 'fwd-1080,fwd-8080') {
        failures.push(`current forwardings: duplicate/missing stops after completion (${JSON.stringify(stops)})`)
    }
    if (sftpCloses !== 1 || closes !== 1) {
        failures.push(`current forwardings: duplicate cleanup calls sftp=${sftpCloses} close=${closes}`)
    }
    if (session.open !== false) { failures.push('current forwardings: open not false') }
}

async function currentSharedTab (failures) {
    {
        const gate = makeGate()
        const bridge = makeBridge({ 'ssh.close': () => gate.promise })
        const shell = await startCurrentSession(makeProfile(), bridge)
        let closed = 0
        let destroyed = 0
        shell.closed$.subscribe(() => closed++)
        shell.destroyed$.subscribe(() => destroyed++)
        const host = buildTabHost(currentTabMethods, {
            events: [],
            createSession: () => newCurrentSession(makeProfile(), bridge),
            startSession: session => session.start(),
        })
        host.session = shell
        const pending = host.disconnect()
        if (!await settledWithin(pending)) { failures.push('current shared disconnect: did not resolve while native close pending') }
        if (shell.open !== false || closed !== 1 || destroyed !== 1) { failures.push('current shared disconnect: logical end not reached before native cleanup') }
        if (bridge.listeners.size !== 0) { failures.push('current shared disconnect: listeners remain attached before native cleanup') }
        gate.release()
        await pending
        await tick()
        if (shell.open !== false || closed !== 1 || destroyed !== 1) { failures.push('current shared disconnect: logical end not reached') }
    }
    {
        const gate = makeGate()
        const bridge = makeBridge({ 'ssh.close': () => gate.promise })
        const shell = await startCurrentSession(makeProfile(), bridge)
        const oldConnectionId = bridge.calls.find(call => call.name === 'ssh.connect').request.connectionId
        const created = []
        const host = buildTabHost(currentTabMethods, {
            events: [],
            createSession: () => { const session = newCurrentSession(makeProfile(), bridge); created.push(session); return session },
            startSession: session => session.start(),
        })
        host.session = shell
        const pending = host.reconnect()
        if (!await settledWithin(pending)) { failures.push('current shared reconnect: replacement did not complete while native close pending') }
        if (created.length !== 1 || host.session !== created[0]) { failures.push('current shared reconnect: replacement not installed before native cleanup') }
        const replacement = created[0]
        let replacementDestroyed = 0
        const received = []
        const newConnect = bridge.calls.filter(call => call.name === 'ssh.connect')[1]
        if (replacement) {
            replacement.destroyed$.subscribe(() => replacementDestroyed++)
            if (replacement.open !== true) { failures.push('current shared reconnect: replacement not open before native cleanup') }
            replacement.releaseInitialDataBuffer()
            replacement.binaryOutput$.subscribe(data => received.push(Buffer.from(data)))
            replacement.write(Buffer.from('new input'))
            const write = bridge.calls.find(call => call.name === 'ssh.write')
            if (write?.request.id !== 'session-2') { failures.push('current shared reconnect: replacement does not have a distinct native ID') }
            if (newConnect?.request.connectionId === oldConnectionId) { failures.push('current shared reconnect: replacement reuses old event identity') }
            bridge.emit('ssh:output', { connectionId: newConnect?.request.connectionId, data: Array.from(Buffer.from('new before cleanup')), extended: false })
            if (Buffer.concat(received).toString() !== 'new before cleanup') { failures.push('current shared reconnect: replacement output unavailable before native cleanup') }
        }
        gate.release()
        await pending
        await tick()
        await tick()
        bridge.emit('ssh:exit', { connectionId: oldConnectionId, exitCode: 0, signal: null })
        bridge.emit('ssh:output', { connectionId: oldConnectionId, data: Array.from(Buffer.from('old')), extended: false })
        await tick()
        if (replacement && newConnect) {
            bridge.emit('ssh:output', { connectionId: newConnect.request.connectionId, data: Array.from(Buffer.from('new after cleanup')), extended: false })
            if (Buffer.concat(received).toString() !== 'new before cleanupnew after cleanup') { failures.push('current shared reconnect: old cleanup removed replacement output listener') }
        }
        if (replacementDestroyed !== 0) { failures.push('current shared reconnect: old cleanup affected replacement') }
        await replacement?.destroy()
    }
}

async function currentDuplicateDestroy (failures) {
    const gate = makeGate()
    let closes = 0
    const bridge = makeBridge({ 'ssh.close': () => { closes++; return gate.promise } })
    const session = await startCurrentSession(makeProfile(), bridge)
    let closed = 0
    let destroyed = 0
    session.closed$.subscribe(() => closed++)
    session.destroyed$.subscribe(() => destroyed++)
    const first = session.destroy()
    if (!await settledWithin(first)) {
        failures.push('current duplicate destroy: first destroy did not resolve while native close pending')
    }
    await session.destroy()
    if (session.open !== false || closed !== 1 || destroyed !== 1) { failures.push('current duplicate destroy: logical end not reached before native cleanup') }
    gate.release()
    await first
    await tick()
    await session.destroy()
    if (closes !== 1) { failures.push(`current duplicate destroy: native close count ${closes}`) }
    if (closed !== 1) { failures.push(`current duplicate destroy: closed count ${closed}`) }
    if (destroyed !== 1) { failures.push(`current duplicate destroy: destroyed count ${destroyed}`) }
}

async function currentRejectedCleanup (failures) {
    const unhandled = []
    const onUnhandled = reason => unhandled.push(reason)
    process.on('unhandledRejection', onUnhandled)
    try {
        let sftpCloses = 0
        let closes = 0
        const stops = []
        const bridge = makeBridge({
            'ssh.close': () => { closes++; return Promise.reject(new Error('close boom')) },
            'sftp.close': () => { sftpCloses++; return Promise.reject(new Error('sftp boom')) },
            'ssh.forwardingStop': request => { stops.push(request.id); return Promise.reject(new Error('stop boom')) },
        })
        const profile = makeProfile({
            forwardedPorts: [{ type: 'Local', host: '127.0.0.1', port: 8080, targetAddress: 'db', targetPort: 5432 }],
        })
        const session = await startCurrentSession(profile, bridge)
        await session.openSFTP()
        const pending = session.destroy()
        const settled = await settledWithin(pending)
        await pending.catch(() => undefined)
        await tick()
        await tick()
        if (!settled) { failures.push('current rejected cleanup: destroy did not resolve') }
        if (sftpCloses !== 1) { failures.push(`current rejected cleanup: sftp.close count ${sftpCloses}`) }
        if (stops.join(',') !== 'fwd-8080') { failures.push(`current rejected cleanup: stops ${JSON.stringify(stops)}`) }
        if (closes !== 1) { failures.push(`current rejected cleanup: close count ${closes}`) }
        if (session.open !== false) { failures.push('current rejected cleanup: open not false') }
        if (unhandled.length) {
            failures.push(`current rejected cleanup: unhandled rejections ${unhandled.map(error => error?.message).join('; ')}`)
        }
    } finally {
        process.off('unhandledRejection', onUnhandled)
    }
}

async function currentPendingConnect (failures) {
    const connectGate = makeGate()
    const cancelGate = makeGate()
    let cancels = 0
    const closeIds = []
    const bridge = makeBridge({
        'ssh.connect': () => connectGate.promise,
        'ssh.cancelConnect': () => { cancels++; return cancelGate.promise },
        'ssh.close': request => { closeIds.push(request.id); return Promise.resolve() },
    })
    const session = newCurrentSession(makeProfile(), bridge)
    let closed = 0
    let destroyed = 0
    let closedComplete = false
    let destroyedComplete = false
    session.closed$.subscribe({ next: () => closed++, complete: () => { closedComplete = true } })
    session.destroyed$.subscribe({ next: () => destroyed++, complete: () => { destroyedComplete = true } })
    const startPending = session.start()
    if (!await waitFor(() => bridge.calls.some(call => call.name === 'ssh.connect'))) {
        failures.push('current pending connect: ssh.connect was never requested')
    }
    const pending = session.destroy()
    if (!await settledWithin(pending)) { failures.push('current pending connect: destroy did not resolve while connect stays pending') }
    if (cancels !== 1) { failures.push(`current pending connect: cancellation count ${cancels}`) }
    if (!closedComplete || !destroyedComplete) { failures.push('current pending connect: logical streams not completed while cancellation stays pending') }
    const connectionId = bridge.calls.find(call => call.name === 'ssh.connect').request.connectionId
    bridge.emit('ssh:connecting', { connectionId })
    if (cancels !== 2) { failures.push('current pending connect: late registration did not repeat cancellation') }
    cancelGate.release()
    await pending
    // Pre-auth close: BaseSession.open never became true, so upstream semantics
    // leave it undefined and emit no lifecycle next events, only completion.
    if (session.open === true) { failures.push('current pending connect: open became true before connect resolved') }
    connectGate.release({ id: 'session-late', username: 'late', usedPrivateKey: false })
    await startPending
    await tick()
    await tick()
    if (!closeIds.includes('session-late')) {
        failures.push(`current pending connect: late successful connect not immediately closed (${JSON.stringify(closeIds)})`)
    }
    if (session.open === true) { failures.push('current pending connect: destroyed instance opened after late connect') }
    if (closed !== 0 || destroyed !== 0) {
        failures.push(`current pending connect: pre-auth close invented lifecycle events closed=${closed} destroyed=${destroyed}`)
    }
    if (!closedComplete || !destroyedComplete) {
        failures.push(`current pending connect: lifecycle streams not completed closed=${closedComplete} destroyed=${destroyedComplete}`)
    }
}

async function runCloseContract (variant) {
    const failures = []
    const flows = variant === 'reference'
        ? [referenceEstablished, referenceForwardings, referenceSharedTab]
        : [currentEstablished, currentForwardings, currentSharedTab,
            currentDuplicateDestroy, currentRejectedCleanup, currentPendingConnect]
    for (const flow of flows) {
        try {
            await flow(failures)
        } catch (error) {
            failures.push(`${flow.name}: threw ${error?.message ?? error}`)
        } finally {
            releaseAllGates()
        }
    }
    return failures
}

console.log('SSH close lifecycle contract: verbatim upstream 14e2d60 reference and current tabby-tauri.')
const referenceFailures = await runCloseContract('reference')
assert.deepEqual(referenceFailures, [], `reference control must be green:\n${referenceFailures.join('\n')}`)
console.log('reference: established close, forwarding stop ordering, and shared disconnect/reconnect checks passed.')

const currentFailures = await runCloseContract('current')
if (currentFailures.length > 0) {
    console.error(`current: ${currentFailures.length} SSH close-lifecycle regression(s) reproduced:`)
    for (const failure of currentFailures) { console.error(`  - ${failure}`) }
    process.exitCode = 1
} else {
    console.log('current: logical SSH close, cleanup initiation, and replacement isolation checks passed.')
}
