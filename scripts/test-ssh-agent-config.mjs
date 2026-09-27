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

const fakes = {
    '@angular/core': { Injector: class {} },
    '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
    'rxjs': hostRequire('rxjs'),
    'tabby-core': {
        ConfigService: class {},
        LogService: class { debug () {}; info () {}; warn () {}; error () {} },
        ProfilesService: class {},
        VaultService: class { isEnabled () { return false } },
    },
    'tabby-terminal': {
        BaseSession: class { constructor () { this.middleware = []; this.open = false } setLoginScriptsOptions () {} emitOutput () {} async destroy () { this.open = false } },
        InputProcessor: class {},
        UTF8SplitterMiddleware: class {},
    },
    '../../../tabby-ssh/src/api/interfaces': {},
    '../api/hostBridge': {},
    '../services/passwordStorage.service': { TauriPasswordStorageService: class {} },
    './hostKeyPromptModal.component': { TauriSshHostKeyPromptModalComponent: class {} },
    './sftp': { TauriSftpSession: class { static async open () { throw new Error('SFTP is out of scope') } } },
}

const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
const loaded = { exports: {} }
vm.runInNewContext(compiled, {
    exports: loaded.exports,
    require: name => { if (!(name in fakes)) throw new Error(`Unexpected dependency: ${name}`); return fakes[name] },
    console, window: { crypto: { randomUUID: () => 'connection-uuid' } }, TextEncoder, btoa: v => Buffer.from(v, 'binary').toString('base64'),
})
const { TauriSshSession } = loaded.exports

function createSession (profile, config, bridge, profiles = []) {
    const vault = { isEnabled: () => false }
    const session = Object.create(TauriSshSession.prototype)
    session.profile = profile
    session.bridge = bridge
    session.vault = vault
    session.injector = {
        get: (token) => {
            if (token.name === 'ConfigService') return config
            if (token.name === 'ProfilesService') return { getProfiles: async () => profiles }
            if (token.name === 'LogService') return { create: () => ({ debug: () => {}, info: () => {}, warn: () => {}, error: () => {} }) }
            if (token.name === 'VaultService') return vault
            throw new Error(`Unexpected token: ${token.name}`)
        },
    }
    session.middleware = []
    session.open = false
    session.unlisteners = []
    session.authPrompt = { next: () => {}, complete: () => {} }
    session.serviceMessage = { next: () => {}, complete: () => {} }
    session.forwardingIds = []
    session.connectionId = 'test-uuid'
    session.destroying = false
    return session
}

function normalize (v) { return JSON.parse(JSON.stringify(v)) }

async function runAllTests () {
    console.log('Running SSH agent config resolver tests (real-method harness)...')

    // 1. automatic auth: resolver invoked with current config (auto, null)
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        let resolvedSocket = '/run/user/1000/ssh-agent.sock'
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return resolvedSocket; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 1, 'auto auth: resolver called once')
        assert.deepEqual(normalize(resolveCalls[0].req), { agentType: 'auto', agentPath: null }, 'auto auth: resolver args')
        const agentAuth = auth.find(a => a.type === 'agent')
        assert.equal(agentAuth.socket, resolvedSocket, 'auto auth: returned socket used')
        console.log('  PASS: auto auth resolver invoked with config')
    }

    // 2. automatic auth: resolver returns null
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return null; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const agentAuth = auth.find(a => a.type === 'agent')
        assert.equal(agentAuth.socket, null, 'auto auth: null socket when resolver returns null')
        console.log('  PASS: auto auth resolver returns null')
    }

    // 3. explicit agent auth: resolver invoked
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: 'agent' } }
        const config = { store: { ssh: { agentType: 'pageant', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return 'pageant://'; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 1, 'explicit agent: resolver called once')
        assert.deepEqual(normalize(resolveCalls[0].req), { agentType: 'pageant', agentPath: null }, 'explicit agent: resolver args')
        const agentAuth = auth.find(a => a.type === 'agent')
        assert.equal(agentAuth.socket, 'pageant://', 'explicit agent: returned socket used')
        console.log('  PASS: explicit agent auth invokes resolver')
    }

    // 4. explicit agent auth: custom pipe path
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: 'agent' } }
        const config = { store: { ssh: { agentType: 'pipe', agentPath: '/custom/agent.sock', x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return '/custom/agent.sock'; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.deepEqual(normalize(resolveCalls[0].req), { agentType: 'pipe', agentPath: '/custom/agent.sock' }, 'explicit agent: custom path')
        console.log('  PASS: explicit agent auth custom pipe path')
    }

    // 5. password auth: zero resolver
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: 'password' } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') throw new Error('should not call'); if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 0, 'password auth: zero resolver')
        console.log('  PASS: password auth no resolver')
    }

    // 6. publicKey auth: zero resolver
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: 'publicKey' } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') throw new Error('should not call'); if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 0, 'publicKey auth: zero resolver')
        console.log('  PASS: publicKey auth no resolver')
    }

    // 7. keyboardInteractive auth: zero resolver
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: 'keyboardInteractive' } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') throw new Error('should not call'); if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 0, 'keyboardInteractive auth: zero resolver')
        console.log('  PASS: keyboardInteractive auth no resolver')
    }

    // 8. config mutation: second call uses latest config
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: '/first.sock', x11Display: null } } }
        let resolvedSocket = '/first.sock'
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return resolvedSocket; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        await session.authForOptions(profile.options)
        config.store.ssh.agentPath = '/second.sock'
        resolvedSocket = '/second.sock'
        await session.authForOptions(profile.options)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 2, 'config mutation: two calls')
        assert.deepEqual(normalize(resolveCalls[1].req), { agentType: 'auto', agentPath: '/second.sock' }, 'config mutation: second call uses latest')
        console.log('  PASS: config mutation second call uses latest')
    }

    // 9. automatic auth: keys, agent, keyboard-interactive, then stored password
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: ['/key1'], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return '/agent.sock'; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        assert.equal(auth[0].type, 'privateKey', 'ordering: first privateKey')
        assert.equal(auth[1].type, 'agent', 'ordering: second agent')
        assert.equal(auth[2].type, 'keyboardInteractive', 'ordering: third keyboardInteractive')
        assert.ok(auth[3], 'automatic auth includes a stored password candidate')
        assert.deepEqual(normalize(auth[3]), { type: 'password', secretRef: 'ssh-password://keychain' })
        assert.deepEqual(normalize(auth[4]), { type: 'promptPassword' })
        assert.equal(auth.length, 5)
        session.vault.isEnabled = () => true
        const vaultAuth = await session.authForOptions(profile.options)
        const vaultRef = vaultAuth[3].secretRef
        assert.equal(vaultRef, 'ssh-password://vault', 'native lookup must wait for the resolved username')
        console.log('  PASS: automatic auth ordering preserved')
    }

    // 10. automatic auth: configured keys no discovery
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: ['/custom/key'], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return '/agent.sock'; if (cmd === 'ssh.listPrivateKeys') throw new Error('should not call'); if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const pkAuth = auth.find(a => a.type === 'privateKey')
        assert.equal(pkAuth.fileRef, '/custom/key', 'configured key used')
        assert.equal(bridge.calls.filter(c => c.cmd === 'ssh.listPrivateKeys').length, 0, 'no discovery when keys configured')
        console.log('  PASS: configured keys no discovery')
    }

    // 11. automatic auth: discovery when no keys configured
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return '/agent.sock'; if (cmd === 'ssh.listPrivateKeys') return ['/discovered/key']; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        const pkAuth = auth.find(a => a.type === 'privateKey')
        assert.equal(pkAuth.fileRef, '/discovered/key', 'discovered key used')
        assert.equal(bridge.calls.filter(c => c.cmd === 'ssh.listPrivateKeys').length, 1, 'discovery called')
        console.log('  PASS: discovery when no keys configured')
    }

    // 12. rejection propagates
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') throw new Error('Resolver failed'); if (cmd === 'ssh.listPrivateKeys') return []; return {} } }
        const session = createSession(profile, config, bridge)
        await assert.rejects(session.authForOptions(profile.options), /Resolver failed/, 'rejection propagates')
        console.log('  PASS: rejection propagates')
    }

    // 13. jumpChain: each hop invokes resolver for 'agent' auth
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: 'jump-1', auth: null } }
        const jumpProfile = { id: 'jump-1', type: 'ssh', options: { host: 'jump', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: 'agent' } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return '/agent.sock'; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge, [jumpProfile])
        await session.authForOptions(profile.options) // main connection auth
        const chain = await session.jumpChain(profile.options.jumpHost)
        const resolveCalls = bridge.calls.filter(c => c.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, 2, 'jumpChain: resolver for main + jump')
        assert.deepEqual(normalize(resolveCalls[0].req), { agentType: 'auto', agentPath: null })
        assert.deepEqual(normalize(resolveCalls[1].req), { agentType: 'auto', agentPath: null })
        const jumpAuth = chain[0].auth.find(a => a.type === 'agent')
        assert.equal(jumpAuth.socket, '/agent.sock', 'jump hop uses resolved socket')
        assert.equal(chain[0].auth.some(a => a.type === 'password'), false, 'explicit agent has no password fallback')
        jumpProfile.options.auth = null
        jumpProfile.options.port = 2222
        jumpProfile.options.user = 'hop-user'
        const automaticChain = await session.jumpChain(profile.options.jumpHost)
        assert.deepEqual(normalize(automaticChain[0].auth.at(-2)), {
            type: 'password', secretRef: 'ssh-password://keychain',
        }, 'automatic hop uses its own stored password')
        console.log('  PASS: jumpChain invokes resolver per hop')
    }

    console.log('All SSH agent config resolver tests passed')

    // ===== AGENT FORWARDING TESTS (real connectRequest) =====

    const forwardingOptions = (auth, agentForward) => ({ host: 'h', port: 22, user: 'u', input: {}, privateKeys: [], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth, ...agentForward === undefined ? {} : { agentForward } })
    const passwordAuth = [{ type: 'password', secretRef: 'ssh-password://keychain' }, { type: 'promptPassword' }]
    const keyAuth = { type: 'privateKey', fileRef: '/k', passphraseRef: null }
    const customAgent = { agentType: 'pipe', agentPath: '/custom.sock' }
    const autoAgent = { agentType: 'auto', agentPath: null }
    const forwardingCases = [
        { name: 'password+forward', auth: 'password', forward: true, config: customAgent, resolved: '/custom.sock', expectedAuth: passwordAuth, forwarding: { socket: '/custom.sock' }, resolverCalls: 1 },
        { name: 'publicKey+forward', auth: 'publicKey', forward: true, config: customAgent, resolved: '/custom.sock', expectedAuth: [keyAuth], forwarding: { socket: '/custom.sock' }, resolverCalls: 1 },
        { name: 'keyboardInteractive+forward', auth: 'keyboardInteractive', forward: true, config: customAgent, resolved: '/custom.sock', expectedAuth: [{ type: 'keyboardInteractive' }], forwarding: { socket: '/custom.sock' }, resolverCalls: 1 },
        { name: 'auto+forward reuses socket', auth: null, forward: true, config: autoAgent, resolved: '/auto.sock', expectedAuth: [keyAuth, { type: 'agent', socket: '/auto.sock' }, { type: 'keyboardInteractive' }, ...passwordAuth], forwarding: { socket: '/auto.sock' }, resolverCalls: 1 },
        { name: 'auto+forward reuses null', auth: null, forward: true, config: autoAgent, resolved: null, expectedAuth: [keyAuth, { type: 'agent', socket: null }, { type: 'keyboardInteractive' }, ...passwordAuth], forwarding: { socket: null }, resolverCalls: 1 },
        { name: 'agent(Pageant)+forward reuses null', auth: 'agent', forward: true, config: { agentType: 'pageant', agentPath: null }, resolved: null, expectedAuth: [{ type: 'agent', socket: null }], forwarding: { socket: null }, resolverCalls: 1 },
        { name: 'password+forward false', auth: 'password', forward: false, config: autoAgent, expectedAuth: passwordAuth, forwarding: null, resolverCalls: 0 },
        { name: 'password+forward absent', auth: 'password', forward: undefined, config: autoAgent, expectedAuth: passwordAuth, forwarding: null, resolverCalls: 0 },
    ]
    for (const c of forwardingCases) {
        const profile = { id: 'target', options: forwardingOptions(c.auth, c.forward) }
        const config = { store: { ssh: { ...c.config, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') { if (!c.resolverCalls) throw new Error('should not call'); return c.resolved } if (cmd === 'ssh.listPrivateKeys') return ['/k']; return {} } }
        const request = await createSession(profile, config, bridge).connectRequest()
        const resolveCalls = bridge.calls.filter(x => x.cmd === 'ssh.resolveAgentSocket')
        assert.equal(resolveCalls.length, c.resolverCalls, `${c.name}: resolver call count`)
        if (c.resolverCalls) assert.deepEqual(normalize(resolveCalls[0].req), c.config, `${c.name}: resolver args`)
        assert.deepEqual(normalize(request.agentForwarding), c.forwarding, `${c.name}: agentForwarding`)
        assert.deepEqual(normalize(request.auth), c.expectedAuth, `${c.name}: auth unchanged`)
        assert.equal(request.agentForward, !!c.forward, `${c.name}: agentForward flag`)
        console.log(`  PASS: ${c.name}`)
    }

    // forwarding resolver rejection propagates from connectRequest
    {
        const profile = { id: 'target', options: forwardingOptions('password', true) }
        const config = { store: { ssh: { ...autoAgent, x11Display: null } } }
        const bridge = { invoke: async cmd => { if (cmd === 'ssh.resolveAgentSocket') throw new Error('Resolver failed'); return {} } }
        await assert.rejects(createSession(profile, config, bridge).connectRequest(), /Resolver failed/, 'forwarding rejection propagates')
        console.log('  PASS: forwarding rejection propagates')
    }

    console.log('All SSH agent forwarding tests passed')

    // The configured password precedes keyboard-interactive and stored passwords.
    for (const mode of [null, 'password', 'agent', 'publicKey', 'keyboardInteractive']) {
        for (const password of ['configured-password', '', undefined]) {
            const options = { ...forwardingOptions(mode, false), password }
            const profile = { id: 'target', options }
            const config = { store: { ssh: { ...autoAgent, x11Display: null } } }
            const bridge = { invoke: async cmd => cmd === 'ssh.listPrivateKeys' ? ['/k'] : null }
            const session = createSession(profile, config, bridge)
            const auth = normalize(await session.authForOptions(options))
            const usesPassword = !mode || mode === 'password'
            assert.deepEqual(auth.filter(method => method.type === 'providedPassword'),
                usesPassword && password ? [{ type: 'providedPassword', password }] : [])
            if (usesPassword && password) {
                assert.ok(auth.findIndex(method => method.type === 'providedPassword') < auth.findIndex(method => method.type === 'password'))
                if (!mode) {
                    assert.ok(auth.findIndex(method => method.type === 'providedPassword') > auth.findIndex(method => method.type === 'agent'))
                    assert.ok(auth.findIndex(method => method.type === 'providedPassword') < auth.findIndex(method => method.type === 'keyboardInteractive'))
                }
            }
            assert.equal(options.password, password, 'request construction must not mutate the profile')
        }
    }
    {
        const profile = { id: 'target', options: { ...forwardingOptions('password', false), password: 'target-secret', jumpHost: 'jump' } }
        const jump = { id: 'jump', type: 'ssh', options: { ...forwardingOptions('password', false), host: 'jump.test', password: 'hop-secret' } }
        const config = { store: { ssh: { ...autoAgent, x11Display: null } } }
        const bridge = { invoke: async () => null }
        const request = normalize(await createSession(profile, config, bridge, [jump]).connectRequest())
        assert.deepEqual(request.auth[0], { type: 'providedPassword', password: 'target-secret' })
        assert.deepEqual(request.jumpChain[0].auth[0], { type: 'providedPassword', password: 'hop-secret' })
        assert.ok(!JSON.stringify(request.auth).includes('hop-secret'))
        assert.ok(!JSON.stringify(request.jumpChain).includes('target-secret'))
    }
    console.log('Configured SSH password selection, ordering, and hop isolation passed')
}

runAllTests().catch(err => { console.error('Test failed:', err); process.exit(1) })
