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
    'tabby-core': { LogService: class {}, ProfilesService: class {}, VaultService: class {} },
    'tabby-terminal': { BaseSession: FakeBaseSession, InputProcessor: class {}, UTF8SplitterMiddleware: class {} },
    '../../../tabby-ssh/src/api/interfaces': {},
    '../api/hostBridge': {},
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

const profile = {
    id: 'profile-1',
    options: {
        host: 'example.test', port: 22, user: 'root', input: {}, privateKeys: [],
        forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0,
    },
}

function createSession () {
    const logger = { debug () {}, info () {}, warn () {}, error () {} }
    const session = new TauriSshSession(
        { get: () => ({ create: () => logger }) },
        null,
        { isEnabled: () => false },
        profile,
        { open: () => { throw new Error('Modals are out of scope') } },
    )
    // Only the protocol/lifecycle code under test stays real; auth request building is isolated.
    session.connectRequest = async () => ({ profileId: profile.id })
    session.startForwardings = async () => {}
    return session
}

function createBridge (connect) {
    const calls = []
    return {
        calls,
        callsTo: command => calls.filter(call => call.command === command),
        listen: async () => () => {},
        invoke: async (command, request) => {
            calls.push({ command, request })
            return command === 'ssh.connect' ? connect() : {}
        },
    }
}

const connected = createSession()
connected.bridge = createBridge(async () => ({ id: 'conn-1', username: 'deploy', usedPrivateKey: true }))
await connected.start()
assert.equal(connected.open, true, 'A successful connect must open the session')
assert.equal(connected.authUsername, 'deploy', 'start() must record the authenticated username')
assert.equal(connected.activePrivateKey, true, 'start() must record that a private key authenticated the session')
await connected.destroy()
assert.equal(connected.open, false, 'destroy() must close the base session')
assert.equal(connected.baseCleanups, 1, 'destroy() must release the base session')
assert.equal(connected.bridge.callsTo('ssh.close').length, 1, 'destroy() must still close the native connection')
const keep = 'destroy() must keep it so a file transfer can still be launched after the tab ended'
assert.equal(connected.authUsername, 'deploy', keep)
assert.equal(connected.activePrivateKey, true, keep)
await connected.destroy()
assert.equal(connected.baseCleanups, 1, 'Repeated destroy() must release the base session only once')
assert.equal(connected.bridge.callsTo('ssh.close').length, 1, 'Repeated destroy() must close the native connection only once')

const withoutKey = createSession()
withoutKey.bridge = createBridge(async () => ({ id: 'conn-2', username: 'root' }))
await withoutKey.start()
assert.equal(withoutKey.authUsername, 'root')
assert.equal(withoutKey.activePrivateKey, false, 'A connect result without usedPrivateKey must stay false')
await withoutKey.destroy()
assert.equal(withoutKey.authUsername, 'root', 'destroy() must keep the recorded username')
assert.equal(withoutKey.activePrivateKey, false, 'destroy() must not invent a private key flag')

const cancelled = createSession()
let connectInvoked
let releaseConnect
const connectRequested = new Promise(resolve => { connectInvoked = resolve })
const connectResult = new Promise(resolve => { releaseConnect = resolve })
cancelled.bridge = createBridge(() => (connectInvoked(), connectResult))
const starting = cancelled.start()
await connectRequested
await cancelled.destroy()
releaseConnect({ id: 'conn-3', username: 'deploy', usedPrivateKey: true })
await starting
const lateCloses = cancelled.bridge.callsTo('ssh.close')
assert.equal(lateCloses.length, 1, 'A connect that resolves after destroy() must be closed exactly once')
assert.equal(lateCloses[0].request.id, 'conn-3', 'The late connect must be closed by its own connection id')
assert.equal(cancelled.authUsername, null, 'A cancelled start must not record an authenticated username')
assert.equal(cancelled.activePrivateKey, false, 'A cancelled start must not record a private key flag')
assert.equal(cancelled.open, false, 'A cancelled start must never open the session')

console.log('SSH auth metadata lifecycle contract passed')
