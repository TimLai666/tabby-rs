import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'
import ts from 'typescript'

// Regression contract for the restored 14e2d60 SSH toolbar and Ports controls.
//
// The real TauriSshSession, TauriSshPortForwardingModalComponent and
// TauriSshTabComponent run here together with the real shared
// SSHPortForwardingConfigComponent. Only the native HostBridge IPC, auth
// preparation, and Angular UI dependencies are substituted. The fixed original
// config component and the verbatim upstream addPortForward/removePortForward
// methods provide the reference controls. The reference controls and all
// behavior is checked alongside add/remove/startup/destruction regressions.
//
// Synthetic IPC checks prove wiring and diagnostics only. They do not establish
// a native listener data path, a rendered desktop, or Windows/Linux acceptance.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const cache = new Map()
const logger = { info () {}, debug () {}, warn () {}, error () {} }

function defaultDependency (name) {
    if (name === '@angular/core') {
        return {
            Injector: class {}, InjectionToken: class {}, Injectable: () => target => target,
            Component: () => target => target, Input: () => () => {}, Output: () => () => {},
            EventEmitter: class { emit () {} },
        }
    }
    if (name === '@ng-bootstrap/ng-bootstrap') return { NgbModal: class {}, NgbModalRef: class {} }
    if (name === 'tabby-core') {
        return {
            ...load('tabby-core/src/components/base.component.ts'),
            ...load('tabby-core/src/utfSplitter.ts'),
            LogService: class {}, ConfigService: class {}, ProfilesService: class {},
            PromptModalComponent: class {}, VaultService: class {}, Logger: class {},
            Platform: { Web: 'web' },
        }
    }
    if (name === 'tabby-terminal') {
        return Object.assign({}, ...[
            'session', 'api/middleware', 'middleware/inputProcessing', 'middleware/streamProcessing', 'middleware/utf8Splitter',
        ].map(part => load(`tabby-terminal/src/${part}.ts`)))
    }
    if (name === './hostKeyPromptModal.component') return { TauriSshHostKeyPromptModalComponent: class {} }
    if (name === './sftp') return { TauriSftpSession: class { static async open () { return new this() } async close () {} } }
    if (name === '../services/passwordStorage.service') return { TauriPasswordStorageService: class {} }
    if (name === '../api/hostBridge') return { HostBridge: class {}, TAURI_RUNTIME_INFO: {} }
    return undefined
}

function load (file, overrides) {
    file = path.resolve(root, file)
    const cacheable = !overrides
    if (cacheable && cache.has(file)) return cache.get(file).exports
    const module = { exports: {} }
    if (cacheable) cache.set(file, module)
    const require = name => {
        if (overrides && Object.prototype.hasOwnProperty.call(overrides, name)) return overrides[name]
        const dependency = defaultDependency(name)
        if (dependency !== undefined) return dependency
        if (name.endsWith('.pug')) return ''
        if (name.startsWith('.')) {
            const resolved = path.resolve(path.dirname(file), name)
            return load(resolved.endsWith('.ts') ? resolved : `${resolved}.ts`)
        }
        return createRequire(file)(name)
    }
    const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, esModuleInterop: true, experimentalDecorators: true },
    }).outputText
    vm.runInNewContext(code, {
        module, exports: module.exports, require, Buffer, TextDecoder, TextEncoder, console,
        setTimeout, clearTimeout, setInterval, clearInterval, process, btoa: value => Buffer.from(value, 'binary').toString('base64'),
        window: { crypto: { randomUUID } },
    }, { filename: file })
    return module.exports
}

const { TauriSshSession } = load('tabby-tauri/src/ssh/session.ts')
const { TauriSshPortForwardingModalComponent } = load('tabby-tauri/src/ssh/portForwardingModal.component.ts')
const { PortForwardType } = load('tabby-ssh/src/api/interfaces.ts')
const { BaseSession } = load('tabby-terminal/src/session.ts')
const { Subject } = createRequire(import.meta.url)('rxjs')
const stripAnsi = createRequire(import.meta.url)('strip-ansi')
const colors = createRequire(path.join(root, 'tabby-ssh/package.json'))('ansi-colors')

function gitShow (spec) {
    return execFileSync('git', ['show', spec], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
}

function extractMethod (source, className, methodName) {
    const file = ts.createSourceFile('source.ts', source, ts.ScriptTarget.Latest, true)
    let text = null
    file.forEachChild(node => {
        if (!ts.isClassDeclaration(node) || node.name?.text !== className) { return }
        for (const member of node.members) {
            if (ts.isMethodDeclaration(member) && member.name.getText(file) === methodName) { text = member.getText(file) }
        }
    })
    if (!text) { throw new Error(`could not extract ${className}.${methodName}`) }
    return text
}

function extractClass (source, className) {
    const file = ts.createSourceFile('source.ts', source, ts.ScriptTarget.Latest, true)
    let text = null
    file.forEachChild(node => {
        if (ts.isClassDeclaration(node) && node.name?.text === className) { text = node.getText(file) }
    })
    if (!text) { throw new Error(`could not extract class ${className}`) }
    return text
}

// Verbatim upstream sources; never rewritten below. The whole ForwardedPort
// class is extracted so its field defaults stay exact, and both addPortForward
// and removePortForward are the byte-identical upstream methods.
const upstreamSsh = gitShow('14e2d60:tabby-ssh/src/session/ssh.ts')
const addPortForwardSource = extractMethod(upstreamSsh, 'SSHSession', 'addPortForward')
const removePortForwardSource = extractMethod(upstreamSsh, 'SSHSession', 'removePortForward')
const forwardedPortClassSource = extractClass(gitShow('14e2d60:tabby-ssh/src/session/forwards.ts'), 'ForwardedPort')
const upstreamConfigClassSource = extractClass(gitShow('14e2d60:tabby-ssh/src/components/sshPortForwardingConfig.component.ts'), 'SSHPortForwardingConfigComponent')

const referenceSessionBody = ts.transpileModule(`
${forwardedPortClassSource.replace(/\bexport\s+/g, '')}
class ReferenceSshSession extends BaseSession {
    constructor (profile, transport) {
        super(logger)
        this.profile = profile
        this.transport = transport
        this.serviceMessage = new Subject()
        this.forwardedPorts = []
        this.ssh = new russh.AuthenticatedSSHClient()
        this.ssh.forwardTCPPort = (host, port) => this.transport.forwardTCPPort(host, port)
        this.ssh.stopForwardingTCPPort = (host, port) => this.transport.stopForwardingTCPPort(host, port)
    }
    get serviceMessage$ () { return this.serviceMessage.asObservable() }
    emitServiceMessage (message) { this.serviceMessage.next(message) }
    setupSocketChannelEvents () {}
    ${addPortForwardSource}
    ${removePortForwardSource}
}
`, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.None } }).outputText

const { ReferenceSshSession, ReferenceForwardedPort } = new Function(
    'BaseSession', 'Subject', 'colors', 'russh', 'PortForwardType', 'logger',
    `${referenceSessionBody}\nreturn { ReferenceSshSession, ReferenceForwardedPort: ForwardedPort }`,
)(
    BaseSession, Subject, colors, { AuthenticatedSSHClient: class AuthenticatedSSHClient {} },
    { Local: 'Local', Remote: 'Remote', Dynamic: 'Dynamic' }, logger,
)

// Reference control for the shared config component, compiled from the exact
// 14e2d60 class with its decorator stubbed out.
function compileReferenceConfig () {
    const code = ts.transpileModule(upstreamConfigClassSource.replace(/\bexport\s+/g, ''), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.None, experimentalDecorators: true },
    }).outputText
    class EventEmitter { constructor () { this.value = undefined } emit (value) { this.value = value } }
    return new Function(
        'Component', 'Input', 'Output', 'EventEmitter', 'PortForwardType',
        `${code}\nreturn SSHPortForwardingConfigComponent`,
    )(() => target => target, () => () => {}, () => () => {}, EventEmitter, PortForwardType)
}
const ReferenceConfigComponent = compileReferenceConfig()

const plain = value => JSON.parse(JSON.stringify(value))
const descriptionOf = config => Object.assign(new ReferenceForwardedPort(), config).toString()
const arrowOf = config => config.type === 'Remote' ? ' <- ' : ' -> '
const failurePhrase = config => config.type === 'Remote' ? 'Remote rejected port forwarding for' : 'Failed to forward port'
const expectedForwarded = config => colors.bgGreen.black(arrowOf(config)) + ` Forwarded ${descriptionOf(config)}`
const expectedFailed = (config, reason) => colors.bgRed.black(' X ') + ` ${failurePhrase(config)} ${descriptionOf(config)}: ${reason}`
const expectedStopped = config => `Stopped forwarding ${descriptionOf(config)}`

const local = { type: 'Local', host: '127.0.0.1', port: 8080, targetAddress: 'db.internal', targetPort: 5432, description: 'database' }
const dynamic = { type: 'Dynamic', host: '127.0.0.1', port: 1080, description: 'socks proxy' }
const remote = { type: 'Remote', host: '127.0.0.1', port: 9090, targetAddress: 'files.internal', targetPort: 22, description: 'files' }

function defer () {
    let resolve, reject
    const promise = new Promise((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}

async function settleTo (predicate, label, timeout = 5000) {
    const start = Date.now()
    while (!predicate()) {
        if (Date.now() - start > timeout) { throw new Error(`timed out waiting for ${label}`) }
        await new Promise(resolve => setImmediate(resolve))
    }
}

function makeProfile (forwardedPorts) {
    return { id: 'test', options: {
        host: 'example.test', port: 22, user: 'alice', auth: 'password', password: 'secret',
        privateKeys: [], forwardedPorts, environment: {}, x11: false, agentForward: false,
        keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null, input: { backspace: 'backspace' }, scripts: [],
    } }
}

function injectorFor () {
    return { get: token => token?.name === 'ConfigService'
        ? { store: { ssh: { x11Display: null, agentType: null, agentPath: null } } }
        : { create: () => logger, isEnabled: () => false } }
}

const modals = { open: () => ({ componentInstance: {}, result: Promise.resolve(null), dismiss () {} }) }

function makeBridge (options = {}) {
    const bridge = { calls: [], listeners: new Map(), forwardStarts: [], startCount: 0 }
    bridge.listen = async (name, callback) => { bridge.listeners.set(name, callback); return () => bridge.listeners.delete(name) }
    bridge.invoke = async (name, request) => {
        bridge.calls.push({ name, request })
        if (name === 'ssh.connect') return { id: 'session-1', username: 'alice', usedPrivateKey: true }
        if (name === 'ssh.forwardingStart') {
            const index = bridge.forwardStarts.push(request) - 1
            if (options.startGate) { await options.startGate }
            const failure = options.startFailure ? options.startFailure(request, index) : null
            if (failure) { throw typeof failure === 'string' ? new Error(failure) : failure }
            return { id: options.idFor ? options.idFor(request, index) : `fwd-${++bridge.startCount}` }
        }
        if (name === 'ssh.forwardingStop') {
            if (options.stopGate) { await options.stopGate }
            const failure = options.stopFailure ? options.stopFailure(request) : null
            if (failure) { throw typeof failure === 'string' ? new Error(failure) : failure }
            return {}
        }
        return {}
    }
    return bridge
}

async function boot (forwardedPorts, bridgeOptions) {
    const bridge = makeBridge(bridgeOptions)
    const session = new TauriSshSession(injectorFor(), bridge, { isEnabled: () => false }, makeProfile(forwardedPorts), modals)
    const messages = []
    session.serviceMessage$.subscribe(message => messages.push(message))
    await session.start()
    return { bridge, session, messages }
}

// Array.from re-creates the list in this realm; the session getter returns an
// array whose prototype belongs to the module's vm context, which deepStrictEqual
// would otherwise reject even when the contents match.
const ports = session => Array.from(session.forwardedPorts)
const stops = bridge => bridge.calls.filter(call => call.name === 'ssh.forwardingStop').map(call => call.request.id)
const starts = bridge => bridge.calls.filter(call => call.name === 'ssh.forwardingStart').map(call => call.request)
const serviceMessages = (messages, pattern) => messages.filter(message => stripAnsi(message).includes(stripAnsi(pattern)))

async function ignore (promise) {
    return promise.then(() => null, error => error)
}

function makeRecordingAngular () {
    class EventEmitter { constructor () { this.value = undefined } emit (value) { this.value = value } }
    return {
        Injector: class {}, InjectionToken: class {}, Injectable: () => target => target,
        Component: () => target => target, Input: () => () => {}, Output: () => () => {}, EventEmitter,
    }
}

async function runReference () {
    const results = []
    const check = async (name, fn) => {
        try { await fn(); console.log(`  reference ${name}: pass`) } catch (error) { results.push(`[reference] ${name}: ${error.message}`) }
    }

    await check('original-config-component-emissions', async () => {
        const component = new ReferenceConfigComponent()
        assert.deepEqual(plain(component.newForward), {
            type: 'Local', host: '127.0.0.1', port: 8000, targetAddress: '127.0.0.1', targetPort: 80, description: '',
        }, 'original reset default')
        await component.addForward()
        assert.deepEqual(plain(component.forwardAdded.value), {
            type: 'Local', host: '127.0.0.1', port: 8000, targetAddress: '127.0.0.1', targetPort: 80, description: '',
        }, 'original addForward emits the pending forward')
        assert.deepEqual(plain(component.newForward), {
            type: 'Local', host: '127.0.0.1', port: 8000, targetAddress: '127.0.0.1', targetPort: 80, description: '',
        }, 'original addForward resets the pending forward')
        const removed = { ...local }
        component.remove(removed)
        assert.equal(component.forwardRemoved.value, removed, 'original remove emits the given object')
        assert.equal(component.newForward, removed, 'original remove restores the removed object as pending')
    })

    await check('shared-config-component-matches-original', async () => {
        const { SSHPortForwardingConfigComponent } = load('tabby-ssh/src/components/sshPortForwardingConfig.component.ts', { '@angular/core': makeRecordingAngular() })
        const actual = new SSHPortForwardingConfigComponent()
        const original = new ReferenceConfigComponent()
        assert.deepEqual(plain(actual.newForward), plain(original.newForward), 'shared reset matches original')
        await actual.addForward()
        await original.addForward()
        assert.deepEqual(plain(actual.forwardAdded.value), plain(original.forwardAdded.value), 'shared addForward matches original')
        const fw = { ...dynamic }
        actual.remove(fw)
        original.remove(fw)
        assert.equal(actual.forwardRemoved.value, fw, 'shared remove emits the given object')
        assert.equal(actual.newForward, fw, 'shared remove restores the removed object')
    })

    const transport = () => {
        const attempted = []
        return {
            attempted,
            startLocalListener: fw => { attempted.push({ type: fw.type, port: fw.port }); return Promise.resolve() },
            forwardTCPPort: (host, port) => { attempted.push({ type: 'Remote', port }); return Promise.resolve() },
            stopForwardingTCPPort: (host, port) => { attempted.push({ type: 'Remote-stop', port }); return Promise.resolve() },
        }
    }
    const referenceAdd = async (config, reason) => {
        const send = transport()
        if (reason) {
            send.startLocalListener = fw => { send.attempted.push({ type: fw.type, port: fw.port }); return Promise.reject(new Error(reason)) }
            send.forwardTCPPort = (host, port) => { send.attempted.push({ type: 'Remote', port }); return Promise.reject(new Error(reason)) }
        }
        const session = new ReferenceSshSession(makeProfile([]), send)
        const messages = []
        session.serviceMessage$.subscribe(message => messages.push(message))
        const fw = Object.assign(new ReferenceForwardedPort(), config)
        fw.startLocalListener = () => send.startLocalListener(fw)
        const error = await ignore(session.addPortForward(fw))
        return { session, messages, fw, error }
    }
    const referenceRemove = async config => {
        const send = transport()
        const session = new ReferenceSshSession(makeProfile([]), send)
        const messages = []
        session.serviceMessage$.subscribe(message => messages.push(message))
        const fw = Object.assign(new ReferenceForwardedPort(), config)
        await session.removePortForward(fw)
        return { session, messages, fw }
    }

    for (const config of [local, dynamic]) {
        await check(`original-${config.type.toLowerCase()}-add-forwarded-diagnostic`, async () => {
            const result = await referenceAdd(config)
            assert.equal(result.error, null, 'original local/dynamic add resolves')
            assert.equal(result.messages.at(-1), expectedForwarded(config), 'original forwards with the upstream green arrow and description')
        })
        await check(`original-${config.type.toLowerCase()}-add-failed-diagnostic`, async () => {
            const result = await referenceAdd(config, 'boom')
            assert.ok(result.error, 'original local/dynamic add rejects on listener failure')
            assert.equal(result.messages.at(-1), expectedFailed(config, 'Error: boom'), 'original failure carries the upstream red badge and reason')
        })
        await check(`original-${config.type.toLowerCase()}-remove-diagnostic`, async () => {
            const result = await referenceRemove(config)
            assert.equal(result.messages.at(-1), expectedStopped(config), 'original removal emits the upstream Stopped description')
        })
    }

    await check('original-remote-add-forwarded-diagnostic', async () => {
        const result = await referenceAdd(remote)
        assert.equal(result.error, null, 'original remote add resolves')
        assert.equal(result.messages.at(-1), expectedForwarded(remote), 'original remote forwards with the upstream left arrow')
    })
    await check('original-remote-add-failed-diagnostic', async () => {
        const result = await referenceAdd(remote, 'refused')
        assert.equal(result.messages.at(-1), expectedFailed(remote, 'Error: refused'), 'original remote rejection uses the upstream phrase')
    })
    await check('original-remote-remove-diagnostic', async () => {
        const result = await referenceRemove(remote)
        assert.equal(result.messages.at(-1), expectedStopped(remote), 'original remote removal emits the upstream Stopped description')
    })

    return results
}

async function runCurrent () {
    const results = []
    const check = async (name, fn) => {
        try { await fn(); console.log(`  current ${name}: pass`) } catch (error) { results.push(`[current] ${name}: ${error.message}`) }
    }

    // ---- Existing behavior and reference-derived controls (must be green) ----

    await check('toolbar-pug-matches-fixed-original-prefix', async () => {
        const reference = gitShow('14e2d60:tabby-ssh/src/components/sshTab.component.pug').split('\n').slice(0, 38).join('\n')
        const actual = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/toolbar.component.pug'), 'utf8').split('\n').slice(0, 38).join('\n')
        assert.equal(actual, reference, 'toolbar Pug prefix must match the fixed original byte for byte')
    })

    await check('ports-modal-pug-matches-fixed-original', async () => {
        const reference = gitShow('14e2d60:tabby-ssh/src/components/sshPortForwardingModal.component.pug')
        const actual = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/portForwardingModal.component.pug'), 'utf8')
        assert.equal(actual, reference, 'Ports modal Pug must match the fixed original byte for byte')
    })

    await check('ports-config-pug-matches-fixed-original', async () => {
        const reference = gitShow('14e2d60:tabby-ssh/src/components/sshPortForwardingConfig.component.pug')
        const actual = fs.readFileSync(path.join(root, 'tabby-ssh/src/components/sshPortForwardingConfig.component.pug'), 'utf8')
        assert.equal(actual, reference, 'shared Ports config Pug must match the fixed original byte for byte')
    })

    await check('module-declares-shared-config-and-native-modal', async () => {
        const indexSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/index.ts'), 'utf8')
        assert.match(indexSource, /import \{ SSHPortForwardingConfigComponent \} from '\.\.\/\.\.\/tabby-ssh\/src\/components\/sshPortForwardingConfig\.component'/, 'module must import the shared config component')
        assert.match(indexSource, /import \{ TauriSshPortForwardingModalComponent \} from '\.\/ssh\/portForwardingModal\.component'/, 'module must import the native modal')
        assert.match(indexSource, /\bSSHPortForwardingConfigComponent,/, 'module must declare the shared config component')
        assert.match(indexSource, /\bTauriSshPortForwardingModalComponent,/, 'module must declare the native modal')
    })

    await check('modal-interface-import-does-not-load-upstream-ssh', async () => {
        const modalSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/portForwardingModal.component.ts'), 'utf8')
        assert.match(modalSource, /from '\.\.\/\.\.\/\.\.\/tabby-ssh\/src\/api\/interfaces'/, 'modal must import the forwarding config type from the native interface module')
        assert.doesNotMatch(modalSource, /session\/ssh|session\/forwards/, 'modal must not import the upstream russh/SSH session module')
        const Modal = load('tabby-tauri/src/ssh/portForwardingModal.component.ts', {
            '@angular/core': { Component: () => target => target },
            '../../../tabby-ssh/src/api/interfaces': { },
            './session': { get TauriSshSession () { throw new Error('modal must not load the native session at import time') } },
        }).TauriSshPortForwardingModalComponent
        assert.equal(typeof Modal, 'function', 'modal must load with only its UI dependency substituted')
    })

    await check('tab-exposes-original-toolbar-controls', async () => {
        let metadata = null
        class ConnectableTerminalTabComponent {
            constructor (injector) { this.injector = injector }
            async initializeSession () {}
            setSession (session) { this.session = session }
            attachSessionHandler () {}
            async disconnect () {}
            async destroy () {}
            onSessionDestroyed () {}
        }
        const { TauriSshTabComponent } = load('tabby-tauri/src/ssh/tab.component.ts', {
            '@angular/core': { Component: value => { metadata = value; return target => target } },
            'tabby-terminal': { BaseTerminalTabComponent: { template: '<terminal />', styles: [], animations: [] }, ConnectableTerminalTabComponent },
        })
        const tab = new TauriSshTabComponent({}, {}, {}, {}, {})
        assert.equal(tab.enableToolbar, true, 'the SSH tab must enable the terminal toolbar')
        assert.ok(tab.Platform, 'the SSH tab must expose Platform for the Ports visibility guard')
        assert.match(metadata.template, /keyboard-interactive-auth-panel/, 'the tab template must retain the inline keyboard-interactive panel')
    })

    await check('fresh-session-has-no-forwarded-ports', async () => {
        const { bridge, session } = await boot([])
        assert.deepEqual(ports(session), [], 'a fresh session lists no forwarded ports')
        await session.destroy()
        assert.equal(bridge.listeners.size, 0, 'no listeners remain after teardown')
    })

    await check('profile-startup-attempts-each-forward', async () => {
        const { bridge, session, messages } = await boot([local, dynamic, remote])
        assert.deepEqual(starts(bridge).map(request => request.bindPort), [local.port, dynamic.port, remote.port], 'profile startup attempts every entry')
        assert.deepEqual(messages, [expectedForwarded(local), expectedForwarded(dynamic), expectedForwarded(remote)],
            'profile startup describes Local, Dynamic and Remote forwards exactly like the fixed original')
        await session.destroy()
    })

    await check('profile-startup-failure-isolation', async () => {
        const { bridge, session, messages } = await boot([local, dynamic], { startFailure: request => request.bindPort === local.port ? 'boom' : null })
        assert.equal(session.open, true, 'a rejected profile forward does not close the session')
        assert.ok(session.authUsername, 'the authenticated username survives a profile forward failure')
        assert.deepEqual(serviceMessages(messages, expectedFailed(local, 'Error: boom')), [messages[0]], 'the failure carries the upstream red badge and description')
        await session.destroy()
    })

    await check('destroy-stops-started-ids-once-and-keeps-credentials', async () => {
        const { bridge, session } = await boot([local, remote])
        const startedIds = starts(bridge).map((_, index) => `fwd-${index + 1}`)
        await session.destroy()
        assert.deepEqual(stops(bridge).sort(), startedIds.sort(), 'destroy stops every profile-started forward exactly once')
        assert.equal(new Set(stops(bridge)).size, stops(bridge).length, 'no profile-started id is stopped twice')
        assert.equal(session.authUsername, 'alice', 'destroy preserves the authenticated username')
        assert.equal(session.activePrivateKey, true, 'destroy preserves the private-key flag')
        assert.equal(bridge.listeners.size, 0, 'no listeners remain after teardown')
    })

    await check('ports-modal-opens-for-an-open-session-only', async () => {
        const opened = []
        class ConnectableTerminalTabComponent {
            constructor (injector) { this.injector = injector }
            async initializeSession () {}
            setSession (session) { this.session = session }
            attachSessionHandler () {}
            async disconnect () {}
            async destroy () {}
            onSessionDestroyed () {}
        }
        const { TauriSshTabComponent } = load('tabby-tauri/src/ssh/tab.component.ts', {
            '@angular/core': { Component: () => target => target },
            'tabby-terminal': { BaseTerminalTabComponent: { template: '', styles: [], animations: [] }, ConnectableTerminalTabComponent },
        })
        const modalHost = { open: component => { const instance = {}; opened.push({ component, instance }); return { componentInstance: instance } } }
        const { bridge, session } = await boot([])
        const tab = new TauriSshTabComponent({}, bridge, {}, modalHost, {})
        tab.session = session
        tab.showPortForwarding()
        assert.equal(opened.length, 1, 'an open session opens the native Ports modal')
        assert.equal(opened[0].component, TauriSshPortForwardingModalComponent, 'the tab opens the native forwarding modal component')
        assert.equal(opened[0].instance.session, session, 'the modal is bound to the current session')
        tab.session = { open: false }
        tab.showPortForwarding()
        assert.equal(opened.length, 1, 'a closed session does not open the Ports modal')
        await session.destroy()
    })

    await check('modal-clones-each-add-and-passes-removal-identity', async () => {
        const modal = new TauriSshPortForwardingModalComponent()
        const added = []
        const removed = []
        modal.session = {
            addPortForward (forwarding) { added.push(forwarding); return Promise.resolve() },
            removePortForward (forwarding) { removed.push(forwarding); return Promise.resolve() },
        }
        const original = { ...local }
        modal.onForwardAdded(original)
        assert.equal(added.length, 1, 'the modal submits each add exactly once')
        assert.notEqual(added[0], original, 'the modal clones the added configuration')
        assert.deepEqual(plain(added[0]), plain(original), 'the clone preserves every configured field')
        modal.onForwardRemoved(original)
        assert.equal(removed.length, 1, 'the modal submits each removal exactly once')
        assert.equal(removed[0], original, 'removal passes the exact row object so its native id can be found')
    })

    await check('modal-add-failure-is-contained', async () => {
        const modal = new TauriSshPortForwardingModalComponent()
        modal.session = { addPortForward () { return Promise.reject(new Error('boom')) }, removePortForward () { return Promise.reject(new Error('boom')) } }
        modal.onForwardAdded({ ...local })
        modal.onForwardRemoved({ ...local })
        await new Promise(resolve => setImmediate(resolve))
    })

    // ---- Forwarding add/remove/registry regressions ----

    for (const config of [local, dynamic, remote]) {
        await check(`add-${config.type.toLowerCase()}-forward-becomes-visible`, async () => {
            const { bridge, session, messages } = await boot([])
            try {
                const forwarding = { ...config }
                await ignore(session.addPortForward(forwarding))
                assert.deepEqual(plain(ports(session)), [config], `${config.type}: the row is visible only after a successful start`)
                assert.equal(ports(session)[0], forwarding, `${config.type}: the row is the exact emitted configuration object`)
                const request = starts(bridge)[0]
                assert.deepEqual(plain(request), {
                    sessionId: 'session-1',
                    kind: config.type.toLowerCase(),
                    bindHost: config.host || '127.0.0.1',
                    bindPort: config.port || 0,
                    targetAddress: config.targetAddress || '',
                    targetPort: config.targetPort || 0,
                }, `${config.type}: forwardingStart carries the full configuration for the authenticated session`)
                assert.equal(messages.at(-1), expectedForwarded(config), `${config.type}: the add emits the upstream green diagnostic`)
            } finally { await session.destroy() }
        })
    }

    await check('explicit-empty-bind-host-falls-back-to-loopback', async () => {
        const { bridge, session } = await boot([])
        try {
            await ignore(session.addPortForward({ type: 'Local', host: '', port: 8080, targetAddress: 'db.internal', targetPort: 5432, description: '' }))
            const request = starts(bridge)[0]
            assert.ok(request, 'an explicit empty bind host must reach the bridge')
            assert.equal(request.bindHost, '127.0.0.1', 'an explicit empty bind host falls back to loopback')
        } finally { await session.destroy() }
    })

    await check('failed-add-creates-no-phantom-row', async () => {
        const { bridge, session, messages } = await boot([], { startFailure: () => 'boom' })
        try {
            await ignore(session.addPortForward({ ...local }))
            assert.deepEqual(ports(session), [], 'a failed start creates no phantom row')
            assert.equal(session.open, true, 'a failed start keeps the terminal usable')
            assert.equal(messages.at(-1), expectedFailed(local, 'Error: boom'), 'a failed start emits the upstream red diagnostic')
        } finally { await session.destroy() }
    })

    await check('remove-stops-the-exact-native-id', async () => {
        const { bridge, session, messages } = await boot([])
        try {
            const forwarding = { ...local }
            await ignore(session.addPortForward(forwarding))
            assert.deepEqual(ports(session), [forwarding], 'the added forward must be registered before removal')
            await ignore(session.removePortForward(forwarding))
            assert.deepEqual(ports(session), [], 'a successful removal clears exactly that row')
            assert.deepEqual(stops(bridge), ['fwd-1'], 'removal stops the exact native id belonging to the emitted object')
            assert.equal(messages.at(-1), expectedStopped(local), 'removal emits the upstream Stopped forwarding description')
        } finally { await session.destroy() }
    })

    await check('repeat-port-zero-configs-remove-by-object-identity', async () => {
        const { bridge, session } = await boot([])
        try {
            const first = { type: 'Local', host: '127.0.0.1', port: 0, targetAddress: '127.0.0.1', targetPort: 80, description: '' }
            const second = { type: 'Local', host: '127.0.0.1', port: 0, targetAddress: '127.0.0.1', targetPort: 80, description: '' }
            await ignore(session.addPortForward(first))
            await ignore(session.addPortForward(second))
            assert.equal(ports(session).length, 2, 'two identical port-zero configurations stay separately visible')
            assert.deepEqual(ports(session).map(row => row.port), [0, 0], 'the displayed rows keep the configured port zero')
            assert.notEqual(ports(session)[0], ports(session)[1], 'the two rows are distinct objects')
            await ignore(session.removePortForward(first))
            assert.deepEqual(ports(session), [second], 'removing one object leaves its sibling intact')
            assert.deepEqual(stops(bridge), ['fwd-1'], 'removing one port-zero row stops only its native id')
            await ignore(session.removePortForward(second))
            assert.deepEqual(ports(session), [], 'removing the sibling clears the last row')
            assert.deepEqual(stops(bridge), ['fwd-1', 'fwd-2'], 'each identity stops its own native id')
        } finally { await session.destroy() }
    })

    await check('concurrent-remove-shares-one-stop', async () => {
        const gate = defer()
        const { bridge, session } = await boot([], { stopGate: gate.promise })
        try {
            const forwarding = { ...local }
            await ignore(session.addPortForward(forwarding))
            assert.deepEqual(ports(session), [forwarding], 'the added forward must be registered before concurrent removal')
            const first = ignore(session.removePortForward(forwarding))
            const second = ignore(session.removePortForward(forwarding))
            await settleTo(() => stops(bridge).length >= 1, 'the first removal reaches the bridge')
            assert.deepEqual(stops(bridge), ['fwd-1'], 'duplicate concurrent removal shares one native stop')
            gate.resolve()
            await Promise.all([first, second])
            assert.deepEqual(ports(session), [], 'the shared removal clears the row once')
            assert.deepEqual(stops(bridge), ['fwd-1'], 'the shared removal never issues a second stop')
        } finally { gate.resolve(); await session.destroy() }
    })

    await check('failed-removal-retains-row-for-retry', async () => {
        let fail = true
        const { bridge, session, messages } = await boot([], { stopFailure: () => fail ? 'stop failed' : null })
        try {
            const forwarding = { ...local }
            await ignore(session.addPortForward(forwarding))
            assert.deepEqual(ports(session), [forwarding], 'the added forward must be registered before removal')
            const before = messages.length
            await ignore(session.removePortForward(forwarding))
            assert.deepEqual(ports(session), [forwarding], 'a failed removal retains its active row')
            assert.ok(messages.length > before, 'a failed removal is visible in a service message')
            assert.ok(stripAnsi(messages.at(-1)).includes(descriptionOf(local)), 'the failed removal message carries the forward description')
            fail = false
            await ignore(session.removePortForward(forwarding))
            assert.deepEqual(ports(session), [], 'a retry after a failed removal clears the row')
            assert.deepEqual(stops(bridge), ['fwd-1', 'fwd-1'], 'the retry issues a fresh stop for the retained id')
        } finally { await session.destroy() }
    })

    await check('profile-started-forwards-become-visible', async () => {
        const { session } = await boot([local, dynamic, remote])
        try {
            assert.deepEqual(plain(ports(session).map(row => row.port)), [local.port, dynamic.port, remote.port],
                'profile-started forwarding appears in the same list as modal adds')
        } finally { await session.destroy() }
    })

    await check('profile-rows-do-not-edit-saved-or-other-tab-configs', async () => {
        const configured = { ...local }
        const original = plain(configured)
        const first = await boot([configured])
        const second = await boot([configured])
        try {
            const row = ports(first.session)[0]
            assert.notEqual(row, configured, 'editing a session row must not edit the saved profile')
            assert.notEqual(row, ports(second.session)[0], 'two tabs must not share editable row objects')
            row.host = 'edited.test'
            row.port = 1234
            row.targetAddress = 'edited.target'
            row.targetPort = 4321
            row.description = 'edited row'
            assert.deepEqual(plain(configured), original, 'every saved forwarding field remains unchanged')
            assert.deepEqual(plain(ports(second.session)[0]), original, 'another tab keeps its original row')
        } finally { await first.session.destroy(); await second.session.destroy() }
    })

    await check('profile-row-default-host-matches-upstream', async () => {
        const configured = { ...local }
        delete configured.host
        const { session } = await boot([configured])
        try {
            const expected = Object.assign(new ReferenceForwardedPort(), configured)
            assert.equal(ports(session)[0].host, expected.host, 'an omitted host displays the upstream default')
            assert.equal(Object.hasOwn(configured, 'host'), false, 'the default must not modify the saved profile')
        } finally { await session.destroy() }
    })

    await check('native-already-closed-id-removes-stale-row-after-failed-stop', async () => {
        let attempts = 0
        const { bridge, session, messages } = await boot([], { stopFailure: () => ++attempts === 1
            ? { code: 'io', details: 'server rejected cancellation' }
            : { code: 'invalidArgument', details: 'forwarding is unknown or closed' } })
        try {
            const forwarding = { ...remote }
            await session.addPortForward(forwarding)
            assert.ok(await ignore(session.removePortForward(forwarding)), 'the first genuine stop failure is reported')
            assert.deepEqual(ports(session), [forwarding], 'the first stop failure keeps its row available')
            assert.equal(await ignore(session.removePortForward(forwarding)), null, 'an already-closed native id ends the retry')
            assert.deepEqual(ports(session), [], 'the native unknown-id reply removes the stale row')
            assert.deepEqual(stops(bridge), ['fwd-1', 'fwd-1'], 'the retry uses the same native id')
            assert.equal(messages.at(-1), expectedStopped(remote), 'removing the stale row uses the upstream diagnostic')
        } finally { await session.destroy() }
    })

    await check('other-invalid-argument-stop-failures-keep-the-row', async () => {
        const failure = { code: 'invalidArgument', details: 'another invalid forwarding request' }
        const { session } = await boot([], { stopFailure: () => failure })
        try {
            const forwarding = { ...local }
            await session.addPortForward(forwarding)
            assert.equal(await ignore(session.removePortForward(forwarding)), failure, 'another invalid argument remains an error')
            assert.deepEqual(ports(session), [forwarding], 'another invalid argument must not retire the active row')
        } finally { await session.destroy() }
    })

    await check('late-start-reply-after-destroy-stops-the-returned-id', async () => {
        const gate = defer()
        const { bridge, session, messages } = await boot([], { startGate: gate.promise, idFor: () => 'fwd-late' })
        const forwarding = { ...local }
        let settled = false
        const pending = ignore(session.addPortForward(forwarding)).finally(() => { settled = true })
        assert.deepEqual(ports(session), [], 'a pending start must not appear as an active forwarding row')
        await settleTo(() => starts(bridge).length === 1 || settled, 'the add reaches the bridge or reports failure')
        assert.equal(starts(bridge).length, 1, 'the pending start reaches the bridge')
        const beforeDestroy = messages.length
        await session.destroy()
        gate.resolve()
        await pending
        assert.deepEqual(stops(bridge), ['fwd-late'], 'a late start reply stops its returned native id')
        assert.deepEqual(ports(session), [], 'a late start reply never adds a closed-owner row')
        assert.equal(messages.filter(message => stripAnsi(message) === stripAnsi(expectedForwarded(local))).length, 0,
            'a late start reply does not leak a Forwarded diagnostic')
        assert.equal(messages.length, beforeDestroy, 'a late start reply adds no service message')
    })

    await check('pending-removal-is-not-stopped-twice-on-destroy', async () => {
        const gate = defer()
        let failStop = false
        const { bridge, session } = await boot([], { stopGate: gate.promise, stopFailure: () => failStop ? 'stop failed' : null })
        const forwarding = { ...local }
        await ignore(session.addPortForward(forwarding))
        assert.deepEqual(ports(session), [forwarding], 'the added forward must be registered before pending removal')
        const removal = ignore(session.removePortForward(forwarding))
        await settleTo(() => stops(bridge).length === 1, 'the pending removal reaches the bridge')
        let destroyed = false
        const destroying = session.destroy().then(() => { destroyed = true })
        await settleTo(() => destroyed, 'owner destruction before the pending stop reply')
        assert.deepEqual(stops(bridge), ['fwd-1'], 'destruction does not stop a pending removal twice')
        assert.deepEqual(ports(session), [], 'destruction clears the closed-owner row without waiting for the reply')
        failStop = true
        gate.resolve()
        await Promise.all([removal, destroying])
        assert.deepEqual(stops(bridge), ['fwd-1'], 'the rejected pending stop still does not double-stop the id')
        assert.deepEqual(ports(session), [], 'a rejected pending removal cannot restore a closed-owner row')
    })

    return results
}

console.log('reference controls (verbatim upstream 14e2d60 config and forwarding methods):')
const reference = await runReference()
assert.deepEqual(reference, [], `reference controls must pass:\n${reference.join('\n')}`)
console.log('current (tabby-tauri/src/ssh session, modal and tab):')
const current = await runCurrent()
assert.deepEqual(current, [], `SSH toolbar and Ports behavior must match the fixed original:\n${current.join('\n')}`)
console.log('ssh toolbar and Ports controls match the fixed original')
