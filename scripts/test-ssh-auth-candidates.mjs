import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import ts from 'typescript'
import { loadNativeProfileId } from './lib/native-profile-id.mjs'
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
    '../nativeProfileId': loadNativeProfileId(),
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
    session.bridge = {
        invoke: async (command) => {
            if (command === 'ssh.listPrivateKeys') return ['/keys/discovered-a', '/keys/discovered-b']
            if (command === 'ssh.resolveAgentSocket') return '/run/agent.sock'
            throw new Error(`Unexpected bridge command: ${command}`)
        },
    }
    session.passwordSecretRef = () => ({ id: 'secret' })
    session.jumpChain = async () => []
    return session
}

const summary = auth => [...auth].map(a => {
    if (a.type === 'keyboardInteractive') return a.password !== undefined ? 'keyboardInteractive(password)' : 'keyboardInteractive(secretRef)'
    if (a.type === 'password') return 'password(secretRef)'
    return a.type
})

// Automatic mode order: agent, configured keys, then password candidates.
{
    const auth = await createSession({ privateKeys: ['/keys/a', '/keys/b'], password: 'pw' }).authForOptions(
        { ...baseProfile.options, privateKeys: ['/keys/a', '/keys/b'], password: 'pw' })
    assert.deepEqual(summary(auth), [
        'agent', 'privateKey', 'privateKey', 'providedPassword', 'keyboardInteractive(password)',
        'keyboardInteractive(secretRef)', 'password(secretRef)', 'promptPassword',
    ])
}
{
    const options = { ...baseProfile.options }
    const auth = await createSession().authForOptions(options)
    assert.deepEqual(summary(auth), [
        'agent', 'privateKey', 'privateKey',
        'keyboardInteractive(secretRef)', 'password(secretRef)', 'promptPassword',
    ])
    assert.deepEqual([...auth.filter(a => a.type === 'privateKey').map(a => a.fileRef)], ['/keys/discovered-a', '/keys/discovered-b'])
}

// Blank passwords still produce candidates; absent ones do not.
for (const [mode, expected] of [
    [undefined, ['providedPassword', 'keyboardInteractive(password)']],
    ['password', ['providedPassword']],
    ['keyboardInteractive', ['keyboardInteractive(password)']],
]) {
    for (const [label, password, present] of [['blank', '', true], ['undefined', undefined, false], ['null', null, false]]) {
        const options = { ...baseProfile.options, auth: mode, password }
        const got = summary(await createSession().authForOptions(options))
        for (const entry of expected) {
            assert.equal(got.includes(entry), present, `${mode ?? 'automatic'} ${label} password: ${entry}`)
        }
        if (present) {
            const provided = (await createSession().authForOptions(options)).find(a => a.password !== undefined)
            assert.equal(provided.password, '', 'blank password is preserved verbatim')
        }
    }
}

// Per-profile TERM resolution.
for (const [term, expected] of [
    [undefined, 'xterm-256color'], [null, 'xterm-256color'], ['', 'xterm-256color'],
    ['  ', 'xterm-256color'], [' vt100 ', 'vt100'], [42, 'xterm-256color'],
]) {
    const session = createSession({ term })
    session.authForOptions = async () => []
    const request = await session.connectRequest()
    assert.equal(request.terminal.term, expected, `term ${JSON.stringify(term)}`)
}

// The Tauri SSH profile settings expose the field like upstream
// sshProfileSettings.component.pug ("Terminal type", $TERM placeholder).
const settings = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/profileSettings.component.ts'), 'utf8')
assert.match(settings, /<label>Terminal type<\/label>/, 'settings template labels the TERM field')
assert.match(settings, /placeholder="xterm-256color" \[\(ngModel\)\]="profile\.options\.term"/, 'settings template binds profile.options.term')

console.log('ssh auth candidate and TERM tests passed')
