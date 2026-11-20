// Executes the actual TauriSshSession stored-password prompt handler and checks the
// ssh.authResponse protocol: nothing stored sends no responses, a stored blank
// password sends one empty response, and a stored value sends that value.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import ts from 'typescript'
import { loadNativeProfileId } from './lib/native-profile-id.mjs'

const require = createRequire(import.meta.url)
const sessionSource = fs.readFileSync(new URL('../tabby-tauri/src/ssh/session.ts', import.meta.url), 'utf8')
const dependencies = {
    '@angular/core': { Injector: class {} }, '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
    rxjs: require('rxjs'),
    'tabby-core': { ConfigService: class {}, LogService: class {}, ProfilesService: class {}, VaultService: class {}, PromptModalComponent: class {} },
    'tabby-terminal': { BaseSession: class { constructor (logger) { this.logger = logger; this.middleware = []; this.open = false } setLoginScriptsOptions () {} emitOutput () {} async destroy () { this.open = false } }, InputProcessor: class {}, UTF8SplitterMiddleware: class {} },
    '../../../tabby-ssh/src/api/interfaces': {}, '../api/hostBridge': {},
    '../services/passwordStorage.service': { TauriPasswordStorageService: class {} },
    '../nativeProfileId': loadNativeProfileId(),
    './hostKeyPromptModal.component': { TauriSshHostKeyPromptModalComponent: class {} }, './sftp': { TauriSftpSession: class {} },
}
const compiled = ts.transpileModule(sessionSource, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
const moduleExports = {}
const context = { exports: moduleExports, require: name => { assert.ok(name in dependencies, name); return dependencies[name] }, window: { crypto: { randomUUID: () => 'fixture-connection' } }, console, TextEncoder, Buffer, setTimeout, clearTimeout }
vm.runInNewContext(compiled, context)
const { TauriSshSession } = moduleExports

const profile = { id: 'target-profile', options: { host: 'target.test', port: 22, user: 'default-user', auth: 'password', privateKeys: [], forwardedPorts: [], input: {}, keepaliveInterval: 0, keepaliveCountMax: 0 } }

function deferredConnect () { return new Promise(() => {}) }

async function runStoredPrompt (load) {
    const listeners = new Map()
    const invocations = []
    const storage = {
        loads: [],
        async loadPassword (target, username) {
            this.loads.push({ host: target.options.host, port: target.options.port, username })
            return load()
        },
        async savePassword () {}, async deletePassword () {},
        async savePrivateKeyPassword () {}, async deletePrivateKeyPassword () {}, async loadPrivateKeyPassword () { return null },
    }
    const bridge = {
        async listen (name, callback) { listeners.set(name, callback); return () => listeners.delete(name) },
        async invoke (command, request) {
            invocations.push({ command, request: JSON.parse(JSON.stringify(request)) })
            if (command === 'ssh.connect') return deferredConnect()
            return command === 'ssh.listPrivateKeys' ? [] : null
        },
    }
    const injector = { get (token) {
        if (token === dependencies['tabby-core'].LogService) return { create: () => ({ debug () {}, warn () {}, info () {} }) }
        if (token === dependencies['../services/passwordStorage.service'].TauriPasswordStorageService) return storage
        if (token === dependencies['tabby-core'].ConfigService) return { store: { ssh: {} } }
        throw new Error(`unexpected injector token ${token.name}`)
    } }
    const session = new TauriSshSession(injector, bridge, { isEnabled: () => false }, profile, { open: () => ({ componentInstance: {}, result: Promise.resolve(null), dismiss () {} }) })
    session.authForOptions = async () => [{ type: 'password' }]
    session.jumpChain = async () => []
    session.startForwardings = async () => {}
    void session.start().catch(() => {})
    for (let i = 0; i < 200 && !listeners.has('ssh:authPrompt'); i++) await new Promise(resolve => setTimeout(resolve, 0))
    assert.ok(listeners.has('ssh:authPrompt'), 'session registers its authentication prompt listener')
    listeners.get('ssh:authPrompt')({
        requestId: 'stored-request', id: 'target-profile', connectionId: 'fixture-connection', name: '', instructions: '', prompts: [], username: false,
        storedPassword: { host: 'resolved.test', port: 2202, username: 'resolved-user' },
    })
    let response
    for (let i = 0; i < 200 && !response; i++) {
        await new Promise(resolve => setTimeout(resolve, 0))
        response = invocations.find(call => call.command === 'ssh.authResponse')
    }
    assert.ok(response, 'stored-password prompt receives one authResponse')
    assert.deepEqual(storage.loads, [{ host: 'resolved.test', port: 2202, username: 'resolved-user' }], 'stored password is read for the resolved identity')
    void session.destroy().catch(() => {})
    return response.request
}

const cases = [
    ['nothing stored sends no responses', null, { requestId: 'stored-request', responses: [] }],
    ['stored blank password sends one empty response', '', { requestId: 'stored-request', responses: [''] }],
    ['stored password sends its value', 'synthetic-stored-value', { requestId: 'stored-request', responses: ['synthetic-stored-value'] }],
]
for (const [name, stored, expected] of cases) {
    const request = await runStoredPrompt(async () => stored)
    assert.deepEqual(request, expected, name)
    console.log(`PASS ${name}`)
}

const failed = await runStoredPrompt(async () => { throw new Error('synthetic storage failure') })
assert.deepEqual(failed, { requestId: 'stored-request', responses: [], abort: true }, 'storage read failure keeps the abort path')
console.log('PASS storage read failure aborts the native waiter')
console.log('ssh stored-password protocol tests passed')
