import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const source = fs.readFileSync(new URL('../tabby-tauri/src/ssh/session.ts', import.meta.url), 'utf8')
const fakes = {
    '@angular/core': { Injector: class {} },
    '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
    rxjs: require('rxjs'),
    'tabby-core': {
        ConfigService: class {}, LogService: class {}, ProfilesService: class {}, VaultService: class {},
        PromptModalComponent: class {},
    },
    'tabby-terminal': {
        BaseSession: class {
            constructor (logger) { this.logger = logger; this.middleware = []; this.open = false }
            setLoginScriptsOptions () {} emitOutput () {} async destroy () { this.open = false }
        },
        InputProcessor: class {}, UTF8SplitterMiddleware: class {},
    },
    '../../../tabby-ssh/src/api/interfaces': {},
    '../api/hostBridge': {},
    '../services/passwordStorage.service': { TauriPasswordStorageService: class {} },
    './hostKeyPromptModal.component': { TauriSshHostKeyPromptModalComponent: class {} },
    './sftp': { TauriSftpSession: class {} },
}
const loaded = { exports: {} }
vm.runInNewContext(ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
}).outputText, {
    exports: loaded.exports,
    require: name => { assert.ok(name in fakes, name); return fakes[name] },
    window: { crypto: { randomUUID: () => 'connection-1' } }, console, TextEncoder,
    btoa: value => Buffer.from(value, 'binary').toString('base64'),
})
const plain = value => JSON.parse(JSON.stringify(value))
function deferred () {
    let resolve, reject
    const promise = new Promise((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}
async function settle () { for (let i = 0; i < 30; i++) await Promise.resolve() }
async function fixture (options = {}) {
    const handlers = new Map(), calls = [], modals = [], saves = [], messages = [], keySaves = [], keyDeletes = [], passwordDeletes = []
    const connect = deferred()
    const storage = {
        loadPassword: async () => options.prefill ? options.prefill.promise : 'saved-password',
        savePassword: async (...args) => { saves.push(args); if (options.saveFails) throw new Error('storage failed') },
        deletePassword: async (...args) => {
            passwordDeletes.push(args)
            if (options.passwordDeleteFails) throw new Error('private storage failure')
        },
        deletePrivateKeyPassword: async hash => {
            keyDeletes.push(hash)
            if (options.deletion) await options.deletion.promise
            if (options.deleteFails) throw new Error('secret deletion failed')
        },
        savePrivateKeyPassword: async (...args) => {
            keySaves.push(args)
            if (options.saveFails) throw new Error('secret save failed')
        },
    }
    const bridge = {
        listen: async (name, callback) => { handlers.set(name, callback); if (options.listening) await options.listening.promise; if (name === 'ssh:connecting' && options.registering) await options.registering.promise; return () => handlers.delete(name) },
        invoke: async (command, request) => {
            calls.push({ command, request })
            if (command === 'ssh.connect') return connect.promise
            if (command === 'ssh.authResponse' && options.acceptDuringResponse) {
                handlers.get('ssh:passwordAccepted')?.({ requestId: request.requestId, connectionId: 'connection-1' })
            }
            if (command === 'ssh.authResponse' && options.unlockDuringResponse) {
                handlers.get('ssh:privateKeyUnlocked')?.({ requestId: request.requestId, connectionId: 'connection-1' })
            }
            if (command === 'ssh.authResponse' && options.responseFails) throw new Error('send failed')
            return null
        },
    }
    const profile = { id: 'destination', options: { host: 'destination.test', port: 22, user: 'alice', input: {}, forwardedPorts: [] } }
    const session = new loaded.exports.TauriSshSession({ get: token => {
        if (token === fakes['tabby-core'].LogService) return { create: () => ({ debug () {}, warn () {} }) }
        if (token === fakes['../services/passwordStorage.service'].TauriPasswordStorageService) return options.storage ?? storage
        if (token === fakes['tabby-core'].ConfigService) return { store: { ssh: {} } }
        throw new Error(`Unexpected token ${token.name}`)
    } }, bridge, { isEnabled: () => false }, profile, {
        open: component => {
            if (options.modalFails) throw new Error('private modal failure')
            assert.equal(component, options.hostKey ? fakes['./hostKeyPromptModal.component'].TauriSshHostKeyPromptModalComponent : fakes['tabby-core'].PromptModalComponent)
            const result = deferred()
            const instance = {}
            let visible = true
            const modal = {
                get componentInstance () { return visible ? instance : undefined },
                result: result.promise,
                dismiss: () => { visible = false; result.reject(new Error('dismissed')) },
                respond: value => { visible = false; result.resolve(value) },
            }
            modals.push(modal)
            return modal
        },
    })
    session.authForOptions = async () => [{ type: 'promptPassword' }]
    session.jumpChain = async () => []
    session.startForwardings = async () => {}
    if (options.preparing) {
        const connectRequest = session.connectRequest.bind(session)
        session.connectRequest = async () => { await options.preparing.promise; return connectRequest() }
    }
    session.serviceMessage$.subscribe(message => messages.push(message))
    const started = session.start()
    started.catch(() => {})
    await settle()
    const prompt = {
        requestId: 'auth-1', connectionId: 'connection-1', id: 'destination#jump-0',
        name: 'Password for resolved-user@jump.test', instructions: '', prompts: [{ text: 'Password', echo: false }],
        password: { host: 'jump.test', port: 2222, username: 'resolved-user' },
    }
    return {
        session, profile, handlers, calls, modals, saves, keySaves, keyDeletes, passwordDeletes, messages, started, connect, prompt,
        show: () => handlers.get('ssh:authPrompt')(prompt),
        accepted: (overrides = {}) => handlers.get('ssh:passwordAccepted')({ requestId: prompt.requestId, connectionId: prompt.connectionId, ...overrides }),
        unlocked: (overrides = {}) => handlers.get('ssh:privateKeyUnlocked')({ requestId: prompt.requestId, connectionId: prompt.connectionId, ...overrides }),
        responses: () => calls.filter(c => c.command === 'ssh.authResponse').map(c => plain(c.request)),
        finish: async () => { connect.resolve({ id: 'ssh-1', username: 'alice', usedPrivateKey: false }); await started },
    }
}

{
    const f = await fixture({ modalFails: true })
    f.show(); await settle()
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [], abort: true }], 'a broken prompt must abort, not skip authentication')
    assert.ok(f.messages.some(message => /prompt could not be opened/.test(message)))
    assert.ok(f.messages.every(message => !message.includes('private modal failure')))
    f.connect.reject({ code: 'io', details: 'SSH session is closed' })
    await f.started.catch(() => {})
    await settle()
    assert.equal(f.passwordDeletes.length, 0)
    await f.session.destroy()
}
{
    const f = await fixture()
    f.show(); await settle(); f.modals[0].dismiss(); await settle()
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [] }], 'Escape only dismisses the current password candidate')
    f.connect.reject({ code: 'permissionDenied', passwordDeletionTarget: f.prompt.password })
    await f.started.catch(() => {})
    await settle()
    assert.equal(f.passwordDeletes.length, 1, 'final exhaustion after Escape removes the failed account password')
    await f.session.destroy()
}

for (const phase of ['listening', 'preparing', 'registering']) {
    const wait = deferred(), f = await fixture({ [phase]: wait })
    await f.session.destroy()
    wait.resolve()
    await settle()
    assert.equal(f.calls.filter(call => call.command === 'ssh.connect').length, 0, `destroy during ${phase} must prevent native connect`)
    await f.started
    assert.equal(f.handlers.size, 0, 'late listener installation must be cleaned up')
}
{
    const f = await fixture()
    await f.session.destroy()
    assert.deepEqual(f.calls.filter(call => call.command === 'ssh.cancelConnect').map(call => plain(call.request)), [{ connectionId: 'connection-1' }])
    assert.equal(f.handlers.size, 1, 'only native registration acknowledgement may remain while closing a pending connection')
    f.handlers.get('ssh:connecting')({ connectionId: 'another-connection' })
    await settle()
    assert.equal(f.calls.filter(call => call.command === 'ssh.cancelConnect').length, 1)
    f.handlers.get('ssh:connecting')({ connectionId: 'connection-1' })
    await settle()
    assert.equal(f.calls.filter(call => call.command === 'ssh.cancelConnect').length, 2, 'native registration after early cancel must trigger cancellation again')
    f.connect.reject({ code: 'io', details: 'SSH session is closed' })
    await f.started.catch(() => {})
    assert.equal(f.handlers.size, 0)
    assert.equal(f.passwordDeletes.length, 0)
}

{
    const f = await fixture({ hostKey: true })
    const handler = f.handlers.get('ssh:hostKeyPrompt')
    handler({ requestId: 'host-1', connectionId: 'connection-1' })
    await settle()
    assert.equal(f.modals.length, 1)
    await f.session.destroy()
    await settle()
    assert.equal(f.modals[0].componentInstance, undefined, 'closing the tab dismisses its host-key dialog')
    assert.equal(f.calls.filter(call => call.command === 'ssh.cancelConnect').length, 1)
    handler({ requestId: 'host-2', connectionId: 'connection-1' })
    await settle()
    assert.equal(f.modals.length, 1, 'a queued host-key event cannot open a dialog after destroy')
    f.connect.reject({ code: 'io', details: 'closed' })
    await f.started.catch(() => {})
    assert.equal(f.handlers.size, 0)
}
{
    const f = await fixture()
    await f.session.destroy()
    await f.finish()
    assert.deepEqual(f.calls.filter(call => call.command === 'ssh.close').map(call => plain(call.request)), [{ id: 'ssh-1' }], 'a connection that wins the cancel race is still closed')
    assert.equal(f.handlers.size, 0)
}

// Only a final native rejection identifies a password to delete, even without a prompt event.
for (const target of [
    { host: 'destination.test', port: 22, username: 'resolved-target' },
    { host: 'jump.test', port: 2222, username: 'resolved-hop' },
]) {
    const f = await fixture()
    const error = { code: 'permissionDenied', details: 'SSH authentication was rejected', passwordDeletionTarget: target }
    f.connect.reject(error)
    await assert.rejects(f.started, candidate => candidate === error)
    await settle()
    assert.equal(f.passwordDeletes.length, 1)
    const [profile, username] = f.passwordDeletes[0]
    assert.deepEqual(plain({ host: profile.options.host, port: profile.options.port, username }), target)
    assert.equal(profile.options.user, target.username)
    assert.equal(f.profile.options.user, 'alice', 'deletion must not mutate profile defaults')
    assert.equal(f.keyDeletes.length, 0)
    await f.session.destroy()
}
for (const error of [
    null, 'SSH authentication was rejected', new Error('closed'),
    { code: 'permissionDenied', details: 'SSH authentication was rejected' },
    { code: 'io', passwordDeletionTarget: { host: 'h', port: 22, username: 'u' } },
    ...[null, {}, { host: 'h', port: 0, username: 'u' }, { host: 'h', port: 22, username: '' }]
        .map(passwordDeletionTarget => ({ code: 'permissionDenied', passwordDeletionTarget })),
]) {
    const f = await fixture()
    f.connect.reject(error)
    await f.started.catch(candidate => assert.equal(candidate, error))
    await settle()
    assert.equal(f.passwordDeletes.length, 0, 'transport, host-key, malformed and legacy errors cannot delete credentials')
    await f.session.destroy()
}
{
    const f = await fixture({ passwordDeleteFails: true })
    const error = { code: 'permissionDenied', passwordDeletionTarget: { host: 'h', port: 22, username: 'u' } }
    f.connect.reject(error)
    await assert.rejects(f.started, candidate => candidate === error)
    await settle()
    assert.equal(f.passwordDeletes.length, 1)
    assert.ok(f.messages.some(message => /password could not be removed/i.test(message)))
    assert.ok(f.messages.every(message => !message.includes('private storage failure')))
    await f.session.destroy()
}
console.log('Final authentication rejection deletes only the resolved target password; unrelated failures preserve credentials')

{
    const deletion = deferred()
    const f = await fixture({ storage: { deletePassword: () => deletion.promise } })
    const error = { code: 'permissionDenied', passwordDeletionTarget: { host: 'h', port: 22, username: 'u' } }
    let reported = false
    f.started.catch(candidate => { assert.equal(candidate, error); reported = true })
    f.connect.reject(error)
    await settle()
    assert.equal(reported, true, 'a locked or slow credential store must not delay the authentication failure')
    deletion.resolve()
    await settle()
    await f.session.destroy()
}

// Exercise the actual storage service, including its Keychain/Vault routing and selectors.
function loadStorageModule (path, dependencies) {
    const exports = {}
    vm.runInNewContext(ts.transpileModule(fs.readFileSync(new URL(path, import.meta.url), 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
    }).outputText, {
        exports,
        require: name => { assert.ok(name in dependencies, name); return dependencies[name] },
    })
    return exports
}
const storageDeps = { '@angular/core': { Injectable: () => target => target }, 'tabby-core': {}, '../api/hostBridge': {}, '../api/keychain': {} }
const sharedStorage = loadStorageModule('../tabby-ssh/src/services/passwordStorage.service.ts', storageDeps)
const { TauriPasswordStorageService } = loadStorageModule('../tabby-tauri/src/services/passwordStorage.service.ts', {
    ...storageDeps, '../../../tabby-ssh/src/services/passwordStorage.service': sharedStorage,
})
for (const vaultEnabled of [false, true]) {
    const records = new Map(), operations = []
    const vault = {
        isEnabled: () => vaultEnabled,
        addSecret: async secret => records.set(JSON.stringify([secret.type, secret.key]), secret.value),
        removeSecret: async (type, key) => { operations.push([type, plain(key)]); records.delete(JSON.stringify([type, key])) },
        getSecret: async (type, key) => ({ value: records.get(JSON.stringify([type, key])) ?? null }),
    }
    const storage = new TauriPasswordStorageService(vault, { invoke: async (command, request) => {
        const key = JSON.stringify([request.service, request.account])
        if (command === 'keychain.put') return records.set(key, request.value)
        if (command === 'keychain.get') return records.get(key) ?? null
        assert.equal(command, 'keychain.delete')
        operations.push(plain(request)); return records.delete(key)
    } })
    const profiles = [
        { host: 'hop.test', port: 2222, user: 'resolved-hop' },
        { host: 'hop.test', port: 2222, user: 'other-user' },
        { host: 'hop.test', port: 22, user: 'resolved-hop' },
        { host: 'destination.test', port: 2222, user: 'resolved-hop' },
    ].map(options => ({ options }))
    for (const profile of profiles) await storage.savePassword(profile, 'synthetic-password')
    await storage.savePrivateKeyPassword('synthetic-key', 'synthetic-passphrase')
    const f = await fixture({ storage })
    f.connect.reject({ code: 'permissionDenied', passwordDeletionTarget: { host: 'hop.test', port: 2222, username: 'resolved-hop' } })
    await assert.rejects(f.started)
    assert.equal(await storage.loadPassword(profiles[0]), null)
    for (const profile of profiles.slice(1)) assert.equal(await storage.loadPassword(profile), 'synthetic-password')
    assert.equal(await storage.loadPrivateKeyPassword('synthetic-key'), 'synthetic-passphrase')
    assert.deepEqual(operations, vaultEnabled
        ? [[sharedStorage.VAULT_SECRET_TYPE_PASSWORD, { user: 'resolved-hop', host: 'hop.test', port: 2222 }]]
        : [{ service: 'ssh@hop.test:2222', account: 'resolved-hop' }])
    await f.session.destroy()
}
{
    const f = await fixture()
    await f.session.destroy()
    f.connect.reject({ code: 'permissionDenied', passwordDeletionTarget: { host: 'h', port: 22, username: 'u' } })
    await assert.rejects(f.started)
    assert.equal(f.passwordDeletes.length, 0, 'late failure after tab closure cannot delete credentials')
}
console.log('Real password storage routes rejection to one Keychain or Vault entry and preserves other accounts, ports, hosts and private keys')

// A native acceptance event can race the authResponse promise. Only matching success saves.
{
    const f = await fixture({ acceptDuringResponse: true })
    f.show(); await settle()
    assert.equal(f.modals.length, 1, 'native password prompt opens the shared modal')
    assert.deepEqual(plain(f.modals[0].componentInstance), {
        prompt: f.prompt.name, password: true, showRememberCheckbox: true, remember: false, value: 'saved-password',
    })
    f.accepted({ connectionId: 'other' }); f.accepted({ requestId: 'other' })
    assert.equal(f.saves.length, 0)
    f.modals[0].respond({ value: 'typed-password', remember: true }); await settle()
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: ['typed-password'] }])
    assert.equal(f.saves.length, 1)
    const [profile, password, username] = f.saves[0]
    assert.equal(profile.options.host, 'jump.test'); assert.equal(profile.options.port, 2222)
    assert.equal(password, 'typed-password'); assert.equal(username, 'resolved-user')
    assert.equal(f.profile.options.host, 'destination.test')
    f.accepted(); await settle(); assert.equal(f.saves.length, 1, 'duplicate success cannot save twice')
    await f.finish(); await f.session.destroy()
    assert.equal(f.passwordDeletes.length, 0, 'successful authentication must preserve stored passwords')
}
for (const result of [null, { value: '', remember: false }, { value: 'wrong', remember: true }]) {
    const f = await fixture()
    f.show(); await settle(); f.modals[0].respond(result); await settle()
    assert.deepEqual(f.responses()[0].responses, result ? [result.value] : [])
    assert.equal(f.saves.length, 0, 'cancellation, unchecked Remember, and rejected password cannot save')
    f.connect.reject(new Error('authentication rejected')); await assert.rejects(f.started)
    f.accepted(); await settle(); assert.equal(f.saves.length, 0, 'failed connection clears pending secrets')
    await f.session.destroy()
}
{
    const prefill = deferred(), f = await fixture({ prefill })
    f.show(); f.show(); await settle()
    await f.session.destroy()
    prefill.resolve('saved'); await settle()
    assert.equal(f.modals.length, 0, 'destroy during prefill must not resurrect a modal')
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [], abort: true }])
    f.connect.reject(new Error('closed')); await assert.rejects(f.started)
}
{
    const f = await fixture({ saveFails: true, acceptDuringResponse: true })
    f.show(); await settle(); f.modals[0].respond({ value: 'typed-password', remember: true }); await settle()
    await f.finish()
    assert.equal(f.session.open, true, 'storage failure does not undo successful authentication')
    assert.ok(f.messages.some(m => /could not be saved/i.test(m)))
    assert.ok(f.messages.every(m => !m.includes('typed-password')))
    await f.session.destroy()
}
{
    const f = await fixture()
    f.show(); await settle(); await f.session.destroy(); await settle()
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [], abort: true }], 'closing a visible prompt aborts once')
    f.connect.reject(new Error('closed')); await assert.rejects(f.started)
}
console.log('SSH password prompt lifecycle, consent, and persistence tests passed')

function keyPrompt (f) {
    delete f.prompt.password
    f.prompt.privateKeyHash = 'fixture-key-hash'
    f.prompt.name = 'Private key passphrase'
}
{
    const f = await fixture()
    keyPrompt(f); f.show(); await settle()
    assert.equal(f.modals.length, 1, 'private key uses shared prompt')
    assert.deepEqual(plain(f.modals[0].componentInstance), {
        prompt: 'Private key passphrase', password: true, showRememberCheckbox: true, remember: false, value: '',
    })
    assert.deepEqual(f.keyDeletes, ['fixture-key-hash'])
    f.modals[0].respond({ value: 'wrong-passphrase', remember: true }); await settle()
    assert.equal(f.keySaves.length, 0)
    f.prompt.requestId = 'auth-2'; f.show(); await settle()
    f.unlocked({ requestId: 'auth-1' }); await settle()
    assert.equal(f.keySaves.length, 0, 'retry discards rejected passphrase')
    f.modals[1].respond({ value: 'right-passphrase', remember: true }); await settle()
    f.unlocked({ connectionId: 'other' }); f.unlocked({ requestId: 'other' }); await settle()
    assert.equal(f.keySaves.length, 0)
    f.unlocked(); f.unlocked(); await settle()
    assert.deepEqual(f.keySaves, [['fixture-key-hash', 'right-passphrase']])
    assert.equal(f.saves.length, 0, 'private-key secret never uses connection-password storage')
    await f.finish(); await f.session.destroy()
}
for (const result of [null, { value: 'passphrase', remember: false }]) {
    const f = await fixture({ unlockDuringResponse: true })
    keyPrompt(f); f.show(); await settle(); f.modals[0].respond(result); await settle()
    assert.deepEqual(f.responses()[0].responses, result ? [result.value] : [])
    assert.equal(f.keySaves.length, 0)
    await f.finish(); await f.session.destroy()
}
{
    const deletion = deferred(), f = await fixture({ deletion })
    keyPrompt(f); f.show(); f.show(); await settle(); await f.session.destroy()
    deletion.resolve(); await settle()
    assert.equal(f.modals.length, 0)
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [], abort: true }])
    f.connect.reject(new Error('closed')); await assert.rejects(f.started)
}
for (const options of [{ saveFails: true, unlockDuringResponse: true }, { deleteFails: true }, { responseFails: true }]) {
    const f = await fixture(options)
    keyPrompt(f); f.show(); await settle()
    f.modals[0].respond({ value: 'private-secret', remember: true }); await settle()
    if (options.responseFails) { f.unlocked(); await settle(); assert.equal(f.keySaves.length, 0) }
    assert.ok(f.messages.every(message => !message.includes('private-secret')))
    await f.finish(); assert.equal(f.session.open, true)
    await f.session.destroy()
}
console.log('SSH private-key prompt retry, consent, and cleanup tests passed')

for (const response of [{ value: 'bob', remember: true }, null]) {
    const f = await fixture({ acceptDuringResponse: true, unlockDuringResponse: true })
    delete f.prompt.password
    f.prompt.username = true
    f.prompt.name = 'Username for jump.test'
    f.prompt.prompts = [{ text: 'Username', echo: true }]
    f.show(); f.show(); await settle()
    assert.equal(f.modals.length, 1, 'username prompt uses the shared modal only once')
    assert.deepEqual(plain(f.modals[0].componentInstance), {
        prompt: f.prompt.name, password: false, showRememberCheckbox: false, remember: false, value: '',
    })
    f.modals[0].respond(response); await settle()
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: response ? ['bob'] : [] }])
    assert.deepEqual(f.saves, []); assert.deepEqual(f.keySaves, []); assert.deepEqual(f.keyDeletes, [])
    await f.finish(); await f.session.destroy()
}
{
    const f = await fixture()
    delete f.prompt.password
    f.prompt.username = true
    f.handlers.get('ssh:authPrompt')({ ...f.prompt, connectionId: 'another-connection' })
    await settle(); assert.equal(f.modals.length, 0)
    f.show(); await settle(); await f.session.destroy(); await settle()
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [], abort: true }])
    f.connect.reject(new Error('closed')); await assert.rejects(f.started)
}
console.log('SSH username prompt visibility, cancellation, and connection isolation passed')

// Native command errors carry text in AppError.details, not Error.toString().
{
    const tabSource = fs.readFileSync(new URL('../tabby-tauri/src/ssh/tab.component.ts', import.meta.url), 'utf8')
    let failure
    const tabFakes = {
        ...fakes,
        '@angular/core': { ...fakes['@angular/core'], Component: () => target => target },
        'tabby-terminal': {
            BaseTerminalTabComponent: {},
            ConnectableTerminalTabComponent: class {
                async initializeSession () {} setSession (session) { this.session = session } attachSessionHandler () {}
            },
        },
        '../services/winscp.service': {},
        '../../../tabby-ssh/src/api/keyboardInteractivePrompt': {},
        './session': { TauriSshSession: class {
            async start () { throw failure }
            async destroy () {}
        } },
    }
    const tabLoaded = { exports: {} }
    vm.runInNewContext(ts.transpileModule(tabSource, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
    }).outputText, {
        exports: tabLoaded.exports,
        require: name => { assert.ok(name in tabFakes, name); return tabFakes[name] },
        clearTimeout,
    })
    for (const [error, expected] of [
        [{ code: 'io', details: 'SSH session was closed' }, 'SSH session was closed'],
        [{ code: 'permissionDenied', details: 'SSH authentication was rejected' }, 'SSH authentication was rejected'],
        ['connection failed', 'connection failed'],
    ]) {
        failure = error
        const tab = new tabLoaded.exports.TauriSshTabComponent({}, {}, {}, {}, {})
        const output = []
        tab.write = text => output.push(text)
        await tab.initializeSession()
        assert.deepEqual(output, [`\r\nSSH connection failed: ${expected}\r\n`])
    }
}
console.log('SSH native connection errors render readable messages')
