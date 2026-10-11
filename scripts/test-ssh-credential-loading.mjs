import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import ts from 'typescript'
import { execFileSync } from 'node:child_process'

const require = createRequire(import.meta.url)
const root = new URL('../', import.meta.url)
const sessionSource = fs.readFileSync(new URL('tabby-tauri/src/ssh/session.ts', root), 'utf8')
const calls = []
const listeners = new Map()
const passwordStorage = {
    loads: [], value: 'vault-fixture-secret', failure: null,
    async loadPassword (profile, username) {
        this.loads.push({ host: profile.options.host, port: profile.options.port, username })
        if (this.failure) throw this.failure
        return this.value
    },
    async savePassword () {}, async deletePassword () {},
    async savePrivateKeyPassword () {}, async deletePrivateKeyPassword () {}, async loadPrivateKeyPassword () { return null },
}
const dependencies = {
    '@angular/core': { Injector: class {} }, '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
    rxjs: require('rxjs'),
    'tabby-core': { ConfigService: class {}, LogService: class {}, ProfilesService: class {}, VaultService: class {}, PromptModalComponent: class {} },
    'tabby-terminal': { BaseSession: class { constructor (logger) { this.logger = logger; this.middleware = []; this.open = false } setLoginScriptsOptions () {} emitOutput () {} async destroy () { this.open = false } }, InputProcessor: class {}, UTF8SplitterMiddleware: class {} },
    '../../../tabby-ssh/src/api/interfaces': {}, '../api/hostBridge': {},
    '../services/passwordStorage.service': { TauriPasswordStorageService: class {} },
    './hostKeyPromptModal.component': { TauriSshHostKeyPromptModalComponent: class {} }, './sftp': { TauriSftpSession: class {} },
}
const compiled = ts.transpileModule(sessionSource, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
const module = { exports: {} }
vm.runInNewContext(compiled, { exports: module.exports, require: name => { assert.ok(name in dependencies, name); return dependencies[name] }, window: { crypto: { randomUUID: () => 'fixture-connection' } }, console, TextEncoder, Buffer })
const TauriSshSession = module.exports.TauriSshSession
const profile = { id: 'target-profile', options: { host: 'target.test', port: 22, user: 'default-user', auth: 'agent', privateKeys: [], forwardedPorts: [], input: {}, keepaliveInterval: 0, keepaliveCountMax: 0 } }
const bridge = {
    async listen (name, callback) { listeners.set(name, callback); return () => listeners.delete(name) },
    async invoke (command, request) { calls.push({ command, request: structuredClone(request) }); return command === 'ssh.connect' ? { id: 'connected', username: 'resolved-user' } : command === 'ssh.listPrivateKeys' ? [] : null },
}
const injector = { get (token) {
    if (token === dependencies['tabby-core'].LogService) return { create: () => ({ debug () {}, warn () {}, info () {} }) }
    if (token === dependencies['../services/passwordStorage.service'].TauriPasswordStorageService) return passwordStorage
    if (token === dependencies['tabby-core'].ConfigService) return { store: { ssh: {} } }
    if (token === dependencies['tabby-core'].ProfilesService) return { getProfiles: async () => [] }
    throw new Error(`unexpected injector token ${token.name}`)
} }
const session = new TauriSshSession(injector, bridge, { isEnabled: () => true }, profile, { open: component => ({ componentInstance: {}, result: Promise.resolve(null), dismiss () {} }) })
session.authForOptions = async options => [{ type: options.auth ?? 'password' }]
session.jumpChain = async () => []
session.startForwardings = async () => {}
const connectStarted = session.start()
await connectStarted
const promptEvent = listeners.get('ssh:authPrompt')
const checks = []
async function check (name, fn) {
    try { await fn(); checks.push({ name, ok: true }); console.log(`PASS ${name}`) }
    catch (error) { checks.push({ name, ok: false }); console.log(`FAIL ${name}: ${error.message}`) }
}

// Extract and execute the actual baseline method body, rather than checking source text.
const baseline = execFileSync('git', ['show', '14e2d60b9b6dee84a53c37f05eefeb803787de04:tabby-ssh/src/session/ssh.ts'], { encoding: 'utf8' })
const baselineSource = ts.createSourceFile('baseline.ts', baseline, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
let method
function visit (node) { if (ts.isMethodDeclaration(node) && node.name.getText(baselineSource) === 'populateStoredPasswordsForResolvedUsername') method = node; ts.forEachChild(node, visit) }
visit(baselineSource)
assert.ok(method, 'baseline method must exist')
const methodClass = ts.transpileModule(`class Baseline { ${method.getText(baselineSource)} }`, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText
const baselineModule = { exports: {} }
vm.runInNewContext(methodClass + '\nexports.Baseline = Baseline', { exports: baselineModule.exports })
const Baseline = baselineModule.exports.Baseline
for (const auth of [undefined, 'password', 'publicKey', 'agent', 'keyboardInteractive']) {
    await check(`baseline resolves stored password under auth=${auth ?? 'default'}`, async () => {
        let loadCount = 0
        const receiver = Object.assign(Object.create(Baseline.prototype), {
            profile: { options: { auth } }, authUsername: 'resolved-user', allAuthMethods: [],
            passwordStorage: { loadPassword: async (_profile, username) => { assert.equal(username, 'resolved-user'); loadCount++; return 'saved' } },
        })
        await receiver.populateStoredPasswordsForResolvedUsername()
        assert.equal(loadCount, 1)
        let failedLoadCount = 0
        const failedReceiver = Object.assign(Object.create(Baseline.prototype), {
            profile: { options: { auth } }, authUsername: 'resolved-user', allAuthMethods: [],
            passwordStorage: { loadPassword: async () => { failedLoadCount++; throw new Error('baseline storage failure') } },
        })
        await assert.rejects(failedReceiver.populateStoredPasswordsForResolvedUsername(), /baseline storage failure/)
        assert.equal(failedLoadCount, 1, 'baseline propagates a storage load failure for every auth mode')
    })
}

function loadTypeScript (url, modules) {
    const source = fs.readFileSync(url, 'utf8')
    const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
    const result = { exports: {} }
    vm.runInNewContext(js, { exports: result.exports, require: name => { assert.ok(name in modules, name); return modules[name] }, setTimeout, clearTimeout, console })
    return result.exports
}

const injectable = _metadata => target => target
const sharedPassword = loadTypeScript(new URL('../tabby-ssh/src/services/passwordStorage.service.ts', import.meta.url), {
    '@angular/core': { Injectable: injectable },
})
const vaultExports = loadTypeScript(new URL('../tabby-tauri/src/services/vault.service.ts', import.meta.url), {
    '@angular/core': { Injectable: injectable }, '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} }, rxjs: require('rxjs'),
    'tabby-core': { UnlockVaultModalComponent: class {} }, '../api/hostBridge': {},
})
const storageExports = loadTypeScript(new URL('../tabby-tauri/src/services/passwordStorage.service.ts', import.meta.url), {
    '@angular/core': { Injectable: injectable }, 'tabby-core': {},
    '../../../tabby-ssh/src/services/passwordStorage.service': sharedPassword,
    '../api/hostBridge': {}, '../api/keychain': {},
})
await check('real storage and Vault service unlock then read the resolved account', async () => {
    const trace = []
    const vaultBridge = { async invoke (command, request) {
        trace.push({ command, request })
        if (command === 'vault.unlock') return { config: {}, secrets: [] }
        if (command === 'vault.snapshot') return { config: {}, secrets: [] }
        if (command === 'vault.getSecret') return 'fixture-vault-secret'
        if (command === 'vault.status') return { unlocked: true }
        throw new Error(`unexpected Vault command ${command}`)
    } }
    const vault = new vaultExports.TauriVaultService(vaultBridge, { open: () => ({ result: Promise.resolve({ passphrase: 'synthetic-unlock', rememberFor: 1 }) }) })
    vault.setStore({ version: 1, keySalt: 'salt', iv: 'iv', contents: 'ciphertext' })
    const keychainBridge = { invoke: async () => null }
    const storage = new storageExports.TauriPasswordStorageService(vault, keychainBridge)
    const secret = await storage.loadPassword({ options: { host: 'resolved.test', port: 2202, user: 'fallback' } }, 'resolved-user')
    assert.equal(secret, 'fixture-vault-secret')
    assert.deepEqual(trace.map(item => item.command), ['vault.unlock', 'vault.snapshot', 'vault.getSecret'])
    assert.deepEqual(JSON.parse(JSON.stringify(trace[2].request.selector)), { type: 'ssh:password', key: { host: 'resolved.test', port: 2202, user: 'resolved-user' } })
    vault.forgetPassphrase()
})
await check('real Vault unlock cancellation rejects the password read', async () => {
    const vault = new vaultExports.TauriVaultService({ invoke: async command => {
        if (command === 'vault.unlock') throw new Error('must not unlock after cancellation')
        throw new Error(`unexpected Vault command ${command}`)
    } }, { open: () => ({ result: Promise.resolve(null) }) })
    vault.setStore({ version: 1, keySalt: 'salt', iv: 'iv', contents: 'ciphertext' })
    const storage = new storageExports.TauriPasswordStorageService(vault, { invoke: async () => null })
    await assert.rejects(storage.loadPassword({ options: { host: 'h', port: 22, user: 'u' } }), /cancelled/)
})

await check('connect request opts into stored-password loading for direct and jump hosts', async () => {
    const authModes = [undefined, 'password', 'publicKey', 'agent', 'keyboardInteractive']
    const profileService = {
        async getProfiles () { return [
            { id: 'hop-a', type: 'ssh' }, { id: 'hop-b', type: 'ssh' },
        ] },
        getConfigProxyForProfile (p) { return { ...p, options: p.id === 'hop-a'
            ? { host: 'hop-a.test', port: 2201, user: 'hop-a-user', auth: 'publicKey', jumpHost: 'hop-b', privateKeys: [], forwardedPorts: [] }
            : { host: 'hop-b.test', port: 2202, user: 'hop-b-user', auth: 'agent', jumpHost: null, privateKeys: [], forwardedPorts: [] } } },
    }
    const testInjector = { get (token) {
        if (token === dependencies['tabby-core'].LogService) return { create: () => ({ debug () {}, warn () {} }) }
        if (token === dependencies['tabby-core'].ConfigService) return { store: { ssh: {} } }
        if (token === dependencies['tabby-core'].ProfilesService) return profileService
        throw new Error(`unexpected injector token ${token.name}`)
    } }
    const observed = []
    for (const auth of authModes) {
        const options = { ...profile.options, auth, jumpHost: 'hop-a', privateKeys: [] }
        const candidate = new TauriSshSession(testInjector, bridge, { isEnabled: () => true }, { ...profile, options }, { open: () => ({ componentInstance: {}, result: Promise.resolve(null), dismiss () {} }) })
        const request = await candidate.connectRequest()
        observed.push({ auth: auth ?? 'default', target: request.loadStoredPassword, hops: request.jumpChain.map(hop => [hop.host, hop.port, hop.username, hop.loadStoredPassword]) })
        await candidate.destroy()
    }
    assert.deepEqual(JSON.parse(JSON.stringify(observed)), authModes.map(auth => ({ auth: auth ?? 'default', target: true, hops: [
        ['hop-b.test', 2202, 'hop-b-user', true], ['hop-a.test', 2201, 'hop-a-user', true],
    ] })))
})
function deferred () {
    let resolve, reject
    const promise = new Promise((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}
async function tick () { for (let i = 0; i < 20; i++) await Promise.resolve() }
async function storedPromptFixture (connectPromise = Promise.resolve({ id: 'stored-connect', username: 'alice' }), storageOverride = null) {
    const eventHandlers = new Map(), invocations = [], loads = [], reads = [], failures = []
    const responseReceived = deferred()
    const store = { async loadPassword (target, username) { loads.push([target.options.host, target.options.port, username]); if (failures.length) throw failures.shift(); const read = reads.shift(); return read ? read.promise : 'stored-fixture-secret' } }
    const fixtureProfile = { id: 'fixture', options: { ...profile.options, auth: 'agent' } }
    const fixtureBridge = {
        async listen (name, callback) { eventHandlers.set(name, callback); return () => eventHandlers.delete(name) },
        async invoke (command, request) {
            invocations.push({ command, request: structuredClone(request) })
            if (command === 'ssh.authResponse') responseReceived.resolve()
            if (command === 'ssh.connect') return connectPromise
            return null
        },
    }
    const fixtureInjector = { get (token) {
        if (token === dependencies['tabby-core'].LogService) return { create: () => ({ debug () {}, warn () {}, info () {} }) }
        if (token === dependencies['../services/passwordStorage.service'].TauriPasswordStorageService) return storageOverride ?? store
        if (token === dependencies['tabby-core'].ConfigService) return { store: { ssh: {} } }
        throw new Error(`unexpected injector token ${token.name}`)
    } }
    const modalCalls = []
    const candidate = new TauriSshSession(fixtureInjector, fixtureBridge, { isEnabled: () => true }, fixtureProfile, { open: component => { modalCalls.push(component); return { componentInstance: {}, result: Promise.resolve(null), dismiss () {} } } })
    candidate.authForOptions = async () => [{ type: 'agent' }]
    candidate.jumpChain = async () => []
    candidate.startForwardings = async () => {}
    let uiPrompts = 0
    candidate.authPrompt$.subscribe(() => { uiPrompts++ })
    const started = candidate.start()
    started.catch(() => {})
    await tick()
    return { candidate, started, eventHandlers, invocations, loads, reads, failures, modalCalls, responseReceived, uiPrompts: () => uiPrompts }
}
async function waitForResponse (fixture) {
    let timer
    try {
        await Promise.race([
            fixture.responseReceived.promise,
            new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('stored-password response was not sent')), 1000) }),
        ])
    } finally {
        clearTimeout(timer)
    }
}
const storedPrompt = (requestId, connectionId = 'fixture-connection', identity = { host: 'resolved.test', port: 2202, username: 'resolved-user' }) => ({
    requestId, connectionId, id: 'fixture', name: '', instructions: '', prompts: [], storedPassword: identity,
})
await check('full stored-password event unlocks the real Vault service and replies through the bridge', async () => {
    const trace = []
    const vault = new vaultExports.TauriVaultService({ async invoke (command, request) {
        trace.push({ command, request })
        if (command === 'vault.unlock') return { config: {}, secrets: [] }
        if (command === 'vault.snapshot') return { config: {}, secrets: [] }
        if (command === 'vault.getSecret') return 'full-flow-secret'
        throw new Error(`unexpected Vault command ${command}`)
    } }, { open: () => ({ result: Promise.resolve({ passphrase: 'synthetic-unlock', rememberFor: 1 }) }) })
    vault.setStore({ version: 1, keySalt: 'salt', iv: 'iv', contents: 'ciphertext' })
    const storage = new storageExports.TauriPasswordStorageService(vault, { invoke: async () => null })
    const f = await storedPromptFixture(Promise.resolve({ id: 'connected', username: 'alice' }), storage)
    try {
        f.eventHandlers.get('ssh:authPrompt')(storedPrompt('full-flow'))
        await waitForResponse(f)
        assert.deepEqual(trace.map(item => item.command), ['vault.unlock', 'vault.snapshot', 'vault.getSecret'])
        assert.deepEqual(JSON.parse(JSON.stringify(trace[2].request.selector)), { type: 'ssh:password', key: { host: 'resolved.test', port: 2202, user: 'resolved-user' } })
        assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'full-flow', responses: ['full-flow-secret'] }])
        assert.equal(f.uiPrompts(), 0)
        assert.equal(f.modalCalls.length, 0)
    } finally {
        vault.forgetPassphrase()
        await f.candidate.destroy()
    }
})
await check('stored-password event uses the real Keychain storage service and resolved account', async () => {
    const reads = []
    const storage = new storageExports.TauriPasswordStorageService({ isEnabled: () => false }, { async invoke (command, request) {
        reads.push({ command, request: structuredClone(request) })
        assert.equal(command, 'keychain.get')
        return 'synthetic-keychain-secret'
    } })
    const f = await storedPromptFixture(Promise.resolve({ id: 'connected', username: 'alice' }), storage)
    try {
        f.eventHandlers.get('ssh:authPrompt')(storedPrompt('keychain-flow'))
        await waitForResponse(f)
        assert.deepEqual(reads, [{ command: 'keychain.get', request: { service: 'ssh@resolved.test:2202', account: 'resolved-user' } }])
        assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'keychain-flow', responses: ['synthetic-keychain-secret'] }])
        assert.equal(f.uiPrompts(), 0)
        assert.equal(f.modalCalls.length, 0)
    } finally { await f.candidate.destroy() }
})
await check('real Keychain read errors retain the upstream missing-password contract', async () => {
    const reads = []
    const storage = new storageExports.TauriPasswordStorageService({ isEnabled: () => false }, { async invoke (command, request) {
        reads.push({ command, request: structuredClone(request) })
        throw new Error('synthetic Keychain read failure')
    } })
    const f = await storedPromptFixture(Promise.resolve({ id: 'connected', username: 'alice' }), storage)
    try {
        f.eventHandlers.get('ssh:authPrompt')(storedPrompt('keychain-unreadable'))
        await waitForResponse(f)
        assert.deepEqual(reads, [{ command: 'keychain.get', request: { service: 'ssh@resolved.test:2202', account: 'resolved-user' } }])
        assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'keychain-unreadable', responses: [''] }])
        assert.equal(f.uiPrompts(), 0)
    } finally { await f.candidate.destroy() }
})
await check('stored-password event aborts on real Vault unlock cancellation without reading or deleting credentials', async () => {
    const trace = []
    const vault = new vaultExports.TauriVaultService({ async invoke (command) { trace.push(command); throw new Error('native Vault must not be called after unlock cancellation') } }, { open: () => ({ result: Promise.resolve(null) }) })
    vault.setStore({ version: 1, keySalt: 'salt', iv: 'iv', contents: 'ciphertext' })
    const storage = new storageExports.TauriPasswordStorageService(vault, { async invoke () { throw new Error('Keychain must not be called when Vault is enabled') } })
    const f = await storedPromptFixture(Promise.resolve({ id: 'connected', username: 'alice' }), storage)
    try {
        f.eventHandlers.get('ssh:authPrompt')(storedPrompt('vault-cancel'))
        await waitForResponse(f)
        assert.deepEqual(trace, [])
        assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'vault-cancel', responses: [], abort: true }])
        assert.equal(f.uiPrompts(), 0)
    } finally { vault.forgetPassphrase(); await f.candidate.destroy() }
})
await check('stored-password event loads by resolved identity, ignores duplicate/foreign events, and replies exactly once', async () => {
    const f = await storedPromptFixture()
    const read = deferred(); f.reads.push(read)
    const listener = f.eventHandlers.get('ssh:authPrompt')
    listener(storedPrompt('stored-1'))
    listener(storedPrompt('stored-1'))
    listener(storedPrompt('foreign', 'another-connection'))
    await tick()
    assert.deepEqual(f.loads, [['resolved.test', 2202, 'resolved-user']])
    assert.equal(f.uiPrompts(), 0, 'stored-password metadata must bypass the inline/modal password UI')
    assert.equal(f.modalCalls.length, 0)
    assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse'), [])
    read.resolve('stored-fixture-secret')
    await tick()
    assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'stored-1', responses: ['stored-fixture-secret'] }])
    await f.candidate.destroy()
    await f.started
})
await check('empty stored password is returned as one empty response', async () => {
    const f = await storedPromptFixture()
    f.reads.push({ promise: Promise.resolve('') })
    f.eventHandlers.get('ssh:authPrompt')(storedPrompt('empty'))
    await tick()
    assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'empty', responses: [''] }])
    await f.candidate.destroy()
})
await check('missing stored password is returned as one empty response', async () => {
    const f = await storedPromptFixture()
    f.reads.push({ promise: Promise.resolve(null) })
    try {
        f.eventHandlers.get('ssh:authPrompt')(storedPrompt('missing'))
        await waitForResponse(f)
        assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'missing', responses: [''] }])
    } finally { await f.candidate.destroy() }
})
await check('storage read failure aborts the native waiter', async () => {
    const f = await storedPromptFixture()
    f.failures.push(new Error('fixture password storage failure'))
    f.eventHandlers.get('ssh:authPrompt')(storedPrompt('read-fail'))
    await tick()
    assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'read-fail', responses: [], abort: true }])
    await f.candidate.destroy()
})
await check('closing during deferred storage read aborts once and prevents a late secret reply', async () => {
    const f = await storedPromptFixture()
    const read = deferred(); f.reads.push(read)
    f.eventHandlers.get('ssh:authPrompt')(storedPrompt('close-load'))
    await tick()
    await f.candidate.destroy()
    read.resolve('late-secret')
    await tick()
    assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'close-load', responses: [], abort: true }])
    await f.started
})
await check('connect failure during deferred storage read aborts once and prevents late secret reply', async () => {
    const connect = deferred(), f = await storedPromptFixture(connect.promise)
    const read = deferred(); f.reads.push(read)
    f.eventHandlers.get('ssh:authPrompt')(storedPrompt('connect-fail'))
    await tick()
    connect.reject(new Error('fixture connect failed'))
    await f.started.catch(() => {})
    read.resolve('late-secret')
    await tick()
    assert.deepEqual(f.invocations.filter(call => call.command === 'ssh.authResponse').map(call => call.request), [{ requestId: 'connect-fail', responses: [], abort: true }])
})
await check('legacy ordinary password prompt still uses interactive modal', async () => {
    assert.equal(typeof promptEvent, 'function')
    promptEvent({ requestId: 'legacy', connectionId: 'fixture-connection', name: 'Password', prompts: [{ text: 'Password', echo: false }], password: { host: 'target.test', port: 22, username: 'alice' } })
    await new Promise(resolve => setTimeout(resolve, 0))
    assert.ok(calls.some(call => call.command === 'ssh.authResponse' && call.request.requestId === 'legacy'))
})
await session.destroy()
const failed = checks.filter(check => !check.ok).length
console.log(`Credential-loading regression cases: ${checks.length - failed} passed, ${failed} failed, ${checks.length} total`)
if (failed) process.exitCode = 1
