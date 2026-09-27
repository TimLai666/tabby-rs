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
    const handlers = new Map(), calls = [], modals = [], saves = [], messages = []
    const connect = deferred()
    const storage = {
        loadPassword: async () => options.prefill ? options.prefill.promise : 'saved-password',
        savePassword: async (...args) => { saves.push(args); if (options.saveFails) throw new Error('storage failed') },
    }
    const bridge = {
        listen: async (name, callback) => { handlers.set(name, callback); return () => handlers.delete(name) },
        invoke: async (command, request) => {
            calls.push({ command, request })
            if (command === 'ssh.connect') return connect.promise
            if (command === 'ssh.authResponse' && options.acceptDuringResponse) {
                handlers.get('ssh:passwordAccepted')?.({ requestId: request.requestId, connectionId: 'connection-1' })
            }
            return null
        },
    }
    const profile = { id: 'destination', options: { host: 'destination.test', port: 22, user: 'alice', input: {}, forwardedPorts: [] } }
    const session = new loaded.exports.TauriSshSession({ get: token => {
        if (token === fakes['tabby-core'].LogService) return { create: () => ({ debug () {}, warn () {} }) }
        if (token === fakes['../services/passwordStorage.service'].TauriPasswordStorageService) return storage
        if (token === fakes['tabby-core'].ConfigService) return { store: { ssh: {} } }
        throw new Error(`Unexpected token ${token.name}`)
    } }, bridge, { isEnabled: () => false }, profile, {
        open: component => {
            assert.equal(component, fakes['tabby-core'].PromptModalComponent)
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
        session, profile, handlers, calls, modals, saves, messages, started, connect, prompt,
        show: () => handlers.get('ssh:authPrompt')(prompt),
        accepted: (overrides = {}) => handlers.get('ssh:passwordAccepted')({ requestId: prompt.requestId, connectionId: prompt.connectionId, ...overrides }),
        responses: () => calls.filter(c => c.command === 'ssh.authResponse').map(c => plain(c.request)),
        finish: async () => { connect.resolve({ id: 'ssh-1', username: 'alice', usedPrivateKey: false }); await started },
    }
}

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
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [] }])
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
    assert.deepEqual(f.responses(), [{ requestId: 'auth-1', responses: [] }], 'closing a visible prompt cancels once')
    f.connect.reject(new Error('closed')); await assert.rejects(f.started)
}
console.log('SSH password prompt lifecycle, consent, and persistence tests passed')

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
                async initializeSession () {} setSession () {} attachSessionHandler () {}
            },
        },
        '../services/winscp.service': {},
        './authPromptModal.component': {},
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
