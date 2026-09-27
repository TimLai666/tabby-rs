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

    // 9. automatic auth: privateKey -> agent -> keyboardInteractive ordering
    {
        const profile = { id: 'target', options: { host: 'h', port: 22, user: 'u', input: {}, privateKeys: ['/key1'], forwardedPorts: [], keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, auth: null } }
        const config = { store: { ssh: { agentType: 'auto', agentPath: null, x11Display: null } } }
        const bridge = { calls: [], invoke: async (cmd, req) => { bridge.calls.push({ cmd, req }); if (cmd === 'ssh.resolveAgentSocket') return '/agent.sock'; if (cmd === 'ssh.listPrivateKeys') return []; if (cmd === 'ssh.connect') return { id: 'conn-1', username: 'deploy', usedPrivateKey: true }; return {} } }
        const session = createSession(profile, config, bridge)
        const auth = await session.authForOptions(profile.options)
        assert.equal(auth[0].type, 'privateKey', 'ordering: first privateKey')
        assert.equal(auth[1].type, 'agent', 'ordering: second agent')
        assert.equal(auth[2].type, 'keyboardInteractive', 'ordering: third keyboardInteractive')
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
        console.log('  PASS: jumpChain invokes resolver per hop')
    }

    console.log('All SSH agent config resolver tests passed')
}

runAllTests().catch(err => { console.error('Test failed:', err); process.exit(1) })