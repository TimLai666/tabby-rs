import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import ts from 'typescript'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/session.ts'), 'utf8')
const hostRequire = createRequire(import.meta.url)

class FakeBaseSession {
    constructor (logger) {
        this.logger = logger
        this.middleware = []
        this.open = false
        this.baseCleanups = 0
    }
    setLoginScriptsOptions () {}
    emitOutput () {}
    async destroy () {
        this.open = false
        this.baseCleanups += 1
    }
}

const fakes = {
    '@angular/core': { Injector: class {} },
    '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
    'rxjs': hostRequire('rxjs'),
    'tabby-core': {
        LogService: class {},
        ProfilesService: class {},
        VaultService: class {},
        ConfigService: class {},
    },
    'tabby-terminal': { BaseSession: FakeBaseSession, InputProcessor: class {}, UTF8SplitterMiddleware: class {} },
    '../../../tabby-ssh/src/api/interfaces': {},
    '../api/hostBridge': {},
    '../services/passwordStorage.service': { TauriPasswordStorageService: class {} },
    './hostKeyPromptModal.component': { TauriSshHostKeyPromptModalComponent: class {} },
    './sftp': { TauriSftpSession: class { static async open () { throw new Error('SFTP is out of scope') } } },
}

const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
}).outputText
const loaded = { exports: {} }
vm.runInNewContext(compiled, {
    exports: loaded.exports,
    require: name => {
        if (!(name in fakes)) {
            throw new Error(`Unexpected dependency: ${name}`)
        }
        return fakes[name]
    },
    console,
    window: { crypto: { randomUUID: () => 'connection-uuid' } },
    TextEncoder,
    btoa: value => Buffer.from(value, 'binary').toString('base64'),
})
const { TauriSshSession } = loaded.exports

const baseProfile = {
    id: 'profile-1',
    options: {
        host: 'example.test', port: 22, user: 'root', input: {}, privateKeys: [],
        forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0,
    },
}

function createSession (overrides = {}) {
    const config = { store: { ssh: { x11Display: null } } }
    const logger = { debug () {}, info () {}, warn () {}, error () {} }
    const profile = { ...baseProfile, options: { ...baseProfile.options, ...overrides } }
    const session = new TauriSshSession(
        {
            get: (token) => {
                if (token === fakes['tabby-core'].ConfigService) return config
                if (token === fakes['tabby-core'].LogService) return { create: () => logger }
                if (token === fakes['tabby-core'].ProfilesService) return { getProfiles: () => [] }
                if (token === fakes['tabby-core'].VaultService) return { isEnabled: () => false }
                throw new Error(`Unexpected dependency: ${token}`)
            },
        },
        null,
        { isEnabled: () => false },
        profile,
        { open: () => { throw new Error('Modals are out of scope') } },
    )
    session.authForOptions = async () => []
    session.jumpChain = async () => []
    session.startForwardings = async () => {}
    return { session, config }
}

function createBridge (connect) {
    const calls = []
    return {
        calls,
        callsTo: command => calls.filter(call => call.command === command),
        listen: async () => () => {},
        invoke: async (command, request) => {
            calls.push({ command, request })
            return command === 'ssh.connect' ? connect(request) : {}
        },
    }
}

async function runX11Tests () {
    console.log('Running x11Display tests...')

    await runTest('x11Display: undefined (absent config)', undefined, null)
    await runTest('x11Display: explicit null', null, null)
    await runTest('x11Display: non-empty TCP display', 'localhost:10.0', 'localhost:10.0')
    await runTest('x11Display: absolute socket path', '/tmp/.X11-unix/X0', '/tmp/.X11-unix/X0')
    await runTest('x11Display: empty string', '', null)

    const { session, config } = createSession()
    const bridge = createBridge(async (request) => {
        return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }
    })
    session.bridge = bridge
    await session.start()
    let connectCall = bridge.callsTo('ssh.connect')[0]
    assert.ok(connectCall, 'x11Display first connection must be called')
    assert.equal(connectCall.request.x11Display, null, 'x11Display must be null initially')
    assert.equal(connectCall.request.x11, false, 'x11 must be false when profile has no x11')
    await session.destroy()

    config.store.ssh.x11Display = 'localhost:11.0'
    const session2 = new TauriSshSession(
        {
            get: (token) => {
                if (token === fakes['tabby-core'].ConfigService) return config
                if (token === fakes['tabby-core'].LogService) return { create: () => ({ debug () {}, info () {}, warn () {}, error () {} }) }
                if (token === fakes['tabby-core'].ProfilesService) return { getProfiles: () => [] }
                if (token === fakes['tabby-core'].VaultService) return { isEnabled: () => false }
                throw new Error(`Unexpected dependency: ${token}`)
            },
        },
        null,
        { isEnabled: () => false },
        { ...baseProfile, options: { ...baseProfile.options, x11: true } },
        { open: () => { throw new Error('Modals are out of scope') } },
    )
    session2.authForOptions = async () => []
    session2.jumpChain = async () => []
    session2.startForwardings = async () => {}
    const bridge2 = createBridge(async (request) => {
        return { id: 'conn-2', username: 'deploy', usedPrivateKey: true }
    })
    session2.bridge = bridge2
    await session2.start()
    connectCall = bridge2.callsTo('ssh.connect')[0]
    assert.ok(connectCall, 'x11Display second connection must be called')
    assert.equal(connectCall.request.x11Display, 'localhost:11.0', 'x11Display must reflect live config change')
    assert.equal(connectCall.request.x11, true, 'x11 must be true when profile has x11')
    await session2.destroy()

    console.log('  PASS: x11Display tests (including live config reading)')
}

async function runTest (name, x11DisplayValue, expectedDisplay) {
    const { session, config } = createSession()
    config.store.ssh.x11Display = x11DisplayValue
    const bridge = createBridge(async (request) => {
        return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }
    })
    session.bridge = bridge
    await session.start()
    const connectCall = bridge.callsTo('ssh.connect')[0]
    assert.ok(connectCall, `${name}: ssh.connect must be called`)
    assert.equal(connectCall.request.x11Display, expectedDisplay, `${name}: x11Display must be ${expectedDisplay}`)
    await session.destroy()
    console.log(`  PASS: ${name}`)
}

async function runLifecycleTests () {
    const { session, config } = createSession()
    const bridge = createBridge(async () => ({ id: 'conn-1', username: 'deploy', usedPrivateKey: true }))
    session.bridge = bridge
    await session.start()
    assert.equal(session.open, true, 'A successful connect must open the session')
    assert.equal(session.authUsername, 'deploy', 'start() must record the authenticated username')
    assert.equal(session.activePrivateKey, true, 'start() must record that a private key authenticated the session')
    await session.destroy()
    assert.equal(session.open, false, 'destroy() must close the base session')
    assert.equal(session.baseCleanups, 1, 'destroy() must release the base session')
    assert.equal(session.bridge.callsTo('ssh.close').length, 1, 'destroy() must still close the native connection')
    const keep = 'destroy() must keep it so a file transfer can still be launched after the tab ended'
    assert.equal(session.authUsername, 'deploy', keep)
    assert.equal(session.activePrivateKey, true, keep)
    await session.destroy()
    assert.equal(session.baseCleanups, 1, 'Repeated destroy() must release the base session only once')
    assert.equal(session.bridge.callsTo('ssh.close').length, 1, 'Repeated destroy() must close the native connection only once')

    const withoutKey = createSession()
    withoutKey.session.bridge = createBridge(async () => ({ id: 'conn-2', username: 'root' }))
    await withoutKey.session.start()
    assert.equal(withoutKey.session.authUsername, 'root')
    assert.equal(withoutKey.session.activePrivateKey, false, 'A connect result without usedPrivateKey must stay false')
    await withoutKey.session.destroy()
    assert.equal(withoutKey.session.authUsername, 'root', 'destroy() must keep the recorded username')
    assert.equal(withoutKey.session.activePrivateKey, false, 'destroy() must not invent a private key flag')

    const cancelled = createSession()
    let connectInvoked
    let releaseConnect
    const connectRequested = new Promise(resolve => { connectInvoked = resolve })
    const connectResult = new Promise(resolve => { releaseConnect = resolve })
    cancelled.session.bridge = createBridge(() => (connectInvoked(), connectResult))
    const starting = cancelled.session.start()
    await connectRequested
    await cancelled.session.destroy()
    releaseConnect({ id: 'conn-3', username: 'deploy', usedPrivateKey: true })
    await starting
    const lateCloses = cancelled.session.bridge.callsTo('ssh.close')
    assert.equal(lateCloses.length, 1, 'A connect that resolves after destroy() must be closed exactly once')
    assert.equal(lateCloses[0].request.id, 'conn-3', 'The late connect must be closed by its own connection id')
    assert.equal(cancelled.session.authUsername, null, 'A cancelled start must not record an authenticated username')
    assert.equal(cancelled.session.activePrivateKey, false, 'A cancelled start must not record a private key flag')
    assert.equal(cancelled.session.open, false, 'A cancelled start must never open the session')

    console.log('  PASS: SSH auth metadata lifecycle contract')
}

await runX11Tests()
await runLifecycleTests()

console.log('All tests passed')
