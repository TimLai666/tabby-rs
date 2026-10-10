// WinSCP launch contract for the Tauri SSH tab.
//
// The fixed upstream SSH tab retains its authenticated connection independently of
// the shell session. The native tab must preserve that identity for WinSCP hotkey
// and context-menu launches after shell termination and during failed reconnection.
//
// This file runs three groups:
//   1. Reference control. The verbatim 14e2d60 SSHTabComponent (initializeSessionMaybeMultiplex,
//      the actual ngOnInit/hotkey, onSessionDestroyed) runs with selected upstream
//      base/connectable methods in a VM. setupOneSession and the SSH shell session are
//      simulated boundaries; nothing about the retention rule is re-implemented here.
//      It must PASS: the shell session is cleared but sshSession survives and the hotkey
//      still launches from the same authenticated connection.
//   2. Preserved behavior. The current tab's focus routing, hotkeys, canClose matrix, SFTP
//      panel, context-menu construction, and safe launch-error notification must keep working.
//   3. Tab-retention contract. The current tab (real initializeSession, real TauriSshSession,
//      real hotkey, real TauriSftpContextMenu) is asserted against the retention contract.
//      These checks verify retained identity through connection, cancellation, and teardown.
//      The reference group establishes the expected behavior.
//
// Both tab classes run verbatim. The whole class declaration is extracted with the
// TypeScript AST and spliced onto hand-written boundary base classes; only UI/native
// boundaries (bridge, vault, modals, winscp, frontend, tab destroy) are stubbed. The real
// TauriSshSession and the real KeyboardInteractivePrompt are loaded from source, never faked.
// The reference shell uses the current generic BaseSession. These method controls
// do not establish the original desktop or native-binding runtime.

import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const { Subject, first } = require('rxjs')
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const logger = { info () {}, debug () {}, warn () {}, error () {}, create () { return logger } }

const upstream = spec => execFileSync('git', ['show', `14e2d60:${spec}`], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
const readFile = file => fs.readFileSync(path.join(root, file), 'utf8')

function defer () {
    let resolve
    const promise = new Promise(resolution => { resolve = resolution })
    return { promise, resolve }
}

async function settle (rounds = 4) {
    for (let i = 0; i < rounds; i++) {
        await new Promise(resolve => setImmediate(resolve))
    }
}

async function settleUntil (predicate, label, timeout = 5000) {
    const start = Date.now()
    while (!predicate()) {
        if (Date.now() - start > timeout) {
            throw new Error(`timed out waiting for ${label}`)
        }
        await new Promise(resolve => setImmediate(resolve))
    }
}

// ---------------------------------------------------------------------------------------
// Real source loader (real TauriSshSession and KeyboardInteractivePrompt), mirroring the
// forwarding-startup fixture. Only Angular UI, the native bridge, and auth preparation are
// replaced by the requirer.
// ---------------------------------------------------------------------------------------
const moduleCache = new Map()
function load (file) {
    file = path.resolve(root, file)
    if (moduleCache.has(file)) return moduleCache.get(file).exports
    const module = { exports: {} }
    moduleCache.set(file, module)
    const localRequire = name => {
        if (name === '@angular/core') return { Injector: class {}, InjectionToken: class {}, Component: () => target => target, Injectable: () => target => target }
        if (name === '@ng-bootstrap/ng-bootstrap') return { NgbModal: class {}, NgbModalRef: class {} }
        if (name === 'tabby-core') return {
            ...load('tabby-core/src/components/base.component.ts'), ...load('tabby-core/src/utfSplitter.ts'),
            LogService: class {}, ConfigService: class {}, ProfilesService: class {},
            PromptModalComponent: class {}, VaultService: class {}, Logger: class {},
        }
        if (name === 'tabby-terminal') return Object.assign({}, ...[
            'session', 'api/middleware', 'middleware/inputProcessing', 'middleware/streamProcessing', 'middleware/utf8Splitter',
        ].map(part => load(`tabby-terminal/src/${part}.ts`)))
        if (name === './hostKeyPromptModal.component') return { TauriSshHostKeyPromptModalComponent: class {} }
        if (name === './sftp') return { TauriSftpSession: class { static async open () { return new this() } async close () {} } }
        if (name === '../services/passwordStorage.service') return { TauriPasswordStorageService: class {} }
        if (name === '../api/hostBridge') return { HostBridge: class {}, TAURI_RUNTIME_INFO: {} }
        if (name.startsWith('.')) return load(path.resolve(path.dirname(file), `${name}.ts`))
        return createRequire(file)(name)
    }
    const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, esModuleInterop: true },
    }).outputText
    vm.runInNewContext(code, {
        module, exports: module.exports, require: localRequire, Buffer, TextDecoder, console,
        setTimeout, clearTimeout, setInterval, clearInterval, process, window: { crypto: { randomUUID } },
    }, { filename: file })
    return module.exports
}

const { TauriSshSession } = load('tabby-tauri/src/ssh/session.ts')
const { KeyboardInteractivePrompt } = load('tabby-ssh/src/api/keyboardInteractivePrompt.ts')
const { BaseSession } = load('tabby-terminal/src/session.ts')

// ---------------------------------------------------------------------------------------
// AST extraction. The tab class runs verbatim; only UI/native boundaries are hand-written.
// ---------------------------------------------------------------------------------------
const BASE_MEMBERS = [
    'setSession', 'onSessionClosed', 'shouldTabBeDestroyedOnSessionClose', 'onSessionDestroyed',
    'isSessionExplicitlyTerminated', 'attachSessionHandler', 'attachSessionHandlers', 'detachSessionHandlers',
]
const CONNECTABLE_MEMBERS = [
    'reconnectOffered', 'isDisconnectedByHand', 'initializeSession', 'onSessionDestroyed',
    'offerReconnection', 'shouldTabBeDestroyedOnSessionClose', 'clearServiceMessagesOnConnect',
    'reconnect', 'disconnect', 'getRecoveryToken',
]

function extractMembers (sourceText, className, names) {
    const source = ts.createSourceFile(`${className}.ts`, sourceText, ts.ScriptTarget.Latest, true)
    const cls = source.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    if (!cls) throw new Error(`class ${className} not found`)
    const printer = ts.createPrinter()
    const out = []
    for (const member of cls.members) {
        const name = member.name && (ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)) ? member.name.text : undefined
        if (!names.includes(name)) continue
        if (ts.isMethodDeclaration(member) || ts.isPropertyDeclaration(member)) {
            out.push(printer.printNode(ts.EmitHint.Unspecified, member, source))
        }
    }
    return out.join('\n')
}

// Whole-class extraction. Decorators are stripped by slicing from the `class` keyword, so
// the assembled VM needs no Angular decorator factories or `require()` of templates.
function extractClass (source, className) {
    const file = ts.createSourceFile('source.ts', source, ts.ScriptTarget.Latest, true)
    let cls = null
    file.forEachChild(node => { if (ts.isClassDeclaration(node) && node.name?.text === className) cls = node })
    if (!cls) throw new Error(`class ${className} not found`)
    const text = cls.getText(file)
    const start = text.indexOf(`class ${className}`)
    if (start < 0) throw new Error(`class keyword for ${className} not found`)
    return text.slice(start)
}

// ---------------------------------------------------------------------------------------
// Hand-written boundary base classes. Shared base/connectable members are pulled verbatim
// from the real (or upstream reference) sources.
// ---------------------------------------------------------------------------------------
const HARNESS_HEAD = `
class SubscriptionContainer {
    constructor () { this.subscriptions = [] }
    subscribe (source, next) { const subscription = source.subscribe(next); this.subscriptions.push(subscription); return subscription }
    cancelAll () { for (const subscription of this.subscriptions) { subscription.unsubscribe() }; this.subscriptions = [] }
}
class BaseTabComponent {
    constructor (injector) {
        this.injector = injector
        this.events = []; this.notices = []; this.writes = []
        this.hasFocus = false
        this.hotkeys = { hotkey$: new Subject(), unfilteredHotkey$: new Subject() }
        this.subscriptions = []
        this.session = null
        this.sessionChanged = new Subject()
        this.sessionHandlers = new SubscriptionContainer()
        this.output = new Subject(); this.binaryOutput = new Subject()
        this.enablePassthrough = true
        this.explicitProgressState = false
        this.recentInputs = ''
        this.size = { columns: 120, rows: 30 }
        this.frontend = null
        this.profile = { options: {} }
        this.log = { create: () => logger }; this.logger = logger
        this.notifications = { error: text => this.notices.push(text), notice: text => this.notices.push(text) }
        this.translate = { instant: (text, params) => params ? String(text).replace('{host}', params.host) : String(text) }
        this.platform = { showMessageBox: async () => ({ response: 0 }), setClipboard () {} }
        this.config = { store: { ssh: { warnOnClose: false }, terminal: {} }, enabledServices: () => [] }
        this.zone = { run: work => work(), runOutsideAngular: work => work() }
        this.hostApp = { platform: Platform.macOS }
        this.focused$ = new Subject(); this.blurred$ = new Subject(); this.visibility$ = new Subject()
        this.reconnectOffered = false; this.isDisconnectedByHand = false
        this.baseInits = 0
    }
    get sessionChanged$ () { return this.sessionChanged }
    get input$ () { if (!this.frontend) { throw new Error('Frontend not ready') } return this.frontend.input$ }
    get parent () { return null }
    subscribeUntilDestroyed (source, next) { this.subscriptions.push(source.subscribe(next)) }
    ngOnInit () { this.baseInits++ }
    ngOnDestroy () { for (const subscription of this.subscriptions) { subscription.unsubscribe() } }
    setTitle () {}
    setProgress () {}
    setProgressState () {}
    sendInput (data) { this.events.push(['sendInput', String(data)]) }
    write (data) { this.writes.push(String(data)); this.events.push(['write', String(data)]); return Promise.resolve() }
    async destroy () { this.events.push(['tab.destroy']); if (this.session?.open) { await this.session.destroy() } }
}
`

function assemble (variant, globals, extraClasses = '') {
    const baseMembers = extractMembers(variant.base, 'BaseTerminalTabComponent', BASE_MEMBERS)
    const connectableMembers = extractMembers(variant.connectable, 'ConnectableTerminalTabComponent', CONNECTABLE_MEMBERS)
    const tabClass = extractClass(variant.source, variant.className)
    const code = [
        HARNESS_HEAD,
        extraClasses,
        `class BaseTerminalTabComponent extends BaseTabComponent {\n${baseMembers}\n}`,
        `class ConnectableTerminalTabComponent extends BaseTerminalTabComponent {\n${connectableMembers}\n}`,
        tabClass,
        `module.exports = { Tab: ${variant.className} }`,
    ].join('\n')
    const module = { exports: {} }
    vm.runInNewContext(ts.transpileModule(code, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.None, experimentalDecorators: true },
    }).outputText, {
        module, exports: module.exports, console, Promise, logger,
        Subject, first, Buffer, setTimeout, clearTimeout, setImmediate,
        Platform: { Windows: 'Windows', macOS: 'macOS', Linux: 'Linux' },
        HostListener: () => () => undefined,
        ...globals,
    }, { filename: `${variant.className}.js` })
    return module.exports.Tab
}

// ---------------------------------------------------------------------------------------
// Shared fixtures.
// ---------------------------------------------------------------------------------------
function makeProfile (overrides = {}) {
    return {
        id: 'p1', type: 'ssh', name: 'fixture', behaviorOnSessionEnd: 'keep',
        options: {
            host: 'example.test', port: 22, user: 'alice', auth: 'password', password: 'secret',
            privateKeys: [], forwardedPorts: [], environment: {}, x11: false, agentForward: false,
            keepaliveInterval: 0, keepaliveCountMax: 0, jumpHost: null,
            input: { backspace: 'backspace' }, scripts: [],
            ...overrides,
        },
    }
}

const FORWARD_LOCAL = { type: 'Local', host: '127.0.0.1', port: 8080, targetAddress: 'db.internal', targetPort: 5432 }

function makeBridge ({ specs = [], forwardGate = null } = {}) {
    const listeners = new Map()
    const calls = []
    const forwardStarts = []
    let connectIndex = 0
    const bridge = {
        listeners, calls, connectCount: 0, forwardStarts,
        listen: async (name, callback) => { listeners.set(name, callback); return () => listeners.delete(name) },
        invoke: async (name, request) => {
            calls.push({ name, request })
            if (name === 'ssh.connect') {
                const spec = specs.length ? specs[Math.min(connectIndex, specs.length - 1)] ?? {} : {}
                connectIndex++
                bridge.connectCount++
                if (spec.gate) await spec.gate
                if (spec.reject) throw spec.reject
                return spec.result ?? { id: `session-${connectIndex}`, username: 'alice', usedPrivateKey: true }
            }
            if (name === 'ssh.forwardingStart') {
                forwardStarts.push(request)
                if (forwardGate) await forwardGate
                return { id: `fwd-${request.bindPort}` }
            }
            return {}
        },
        emit (name, event) { const callback = listeners.get(name); if (callback) callback(event) },
    }
    return bridge
}

function injectorFor () {
    return { get: () => ({ create: () => logger, store: { ssh: { x11Display: null, agentType: null, agentPath: null } }, isEnabled: () => false, getProfiles: async () => [], getConfigProxyForProfile: () => ({}) }) }
}

const WinSCP_PATH = 'WinSCP.exe'
const WINDOWS_PLATFORM = { Windows: 'Windows', macOS: 'macOS', Linux: 'Linux' }

// Minimal ansi-colors stand-in for the verbatim upstream source (`colors.bgWhite.black(' X ')`).
const colors = new Proxy(function () {}, {
    get: () => colors,
    apply: (target, thisArg, args) => args.join(''),
})

function frontendStub () {
    return { input$: new Subject(), clear () {}, resetTerminalModes () {}, scrollToBottom () {}, focus () {} }
}

// ---------------------------------------------------------------------------------------
// Part 1 — reference control (verbatim upstream 14e2d60).
// ---------------------------------------------------------------------------------------
const REFERENCE_VARIANT = {
    className: 'SSHTabComponent',
    source: upstream('tabby-ssh/src/components/sshTab.component.ts'),
    base: upstream('tabby-terminal/src/api/baseTerminalTab.component.ts'),
    connectable: upstream('tabby-terminal/src/api/connectableTerminalTab.component.ts'),
}

const REFERENCE_EXTRA = `
class SSHShellSession extends BaseSession {
    constructor (injector, sshSession, profile) {
        super(logger)
        this.sshSession = sshSession
        this.profile = profile
        this.serviceMessage = new Subject()
        this.open = false
    }
    get serviceMessage$ () { return this.serviceMessage.asObservable() }
    async start () { this.open = true }
    resize () {}
    write () {}
    kill () {}
    async gracefullyKillProcess () {}
    supportsWorkingDirectory () { return false }
    async getWorkingDirectory () { return null }
}
`

function buildReferenceTab () {
    return assemble(REFERENCE_VARIANT, {
        BaseSession,
        russh: { AuthenticatedSSHClient: class {} },
        SSHSession: class {},
        KeyboardInteractivePrompt: class {},
        SSHPortForwardingModalComponent: class {},
        ProfilesService: class {},
        SSHMultiplexerService: class {},
        _: text => text,
        colors,
    }, REFERENCE_EXTRA)
}

function makeFakeAuthSession () {
    return {
        profile: { name: 'fixture', options: { host: 'example.test' } },
        open: true,
        serviceMessage$: new Subject(),
        willDestroy$: new Subject(),
        keyboardInteractivePrompt$: new Subject(),
        start () {}, resize () {}, ref () {}, unref () {},
    }
}

async function runReference () {
    const failures = []
    const check = async (name, fn) => {
        try { await fn(); console.log(`  reference ${name}: pass`) } catch (error) { failures.push(`[reference] ${name}: ${error.message}`) }
    }
    const Tab = buildReferenceTab()

    await check('shell-end-retains-authenticated-connection', async () => {
        const launched = []
        const ssh = { launchWinSCP: session => launched.push(session) }
        const tab = new Tab(injectorFor(), ssh, {}, {}, {})
        tab.profile = makeProfile()
        tab.frontend = frontendStub()
        tab.ngOnInit()
        const fake = makeFakeAuthSession()
        tab.setupOneSession = async function () { return fake }
        await tab.initializeSession()
        assert.equal(tab.sshSession, fake, 'reference: the authenticated connection is retained on the tab')
        assert.ok(tab.session, 'reference: a live shell session is set during connection')
        const shell = tab.session
        await shell.destroy()
        assert.equal(tab.session, null, 'reference: the ended shell session is cleared')
        assert.equal(tab.sshSession, fake, 'reference: the authenticated connection survives the shell ending')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === fake, 'reference: the actual hotkey launches from the retained authenticated connection')
    })

    await check('failed-reconnect-keeps-previous-authenticated-connection', async () => {
        const launched = []
        const ssh = { launchWinSCP: session => launched.push(session) }
        const tab = new Tab(injectorFor(), ssh, {}, {}, {})
        tab.profile = makeProfile()
        tab.frontend = frontendStub()
        tab.ngOnInit()
        const fake = makeFakeAuthSession()
        let fail = false
        tab.setupOneSession = async function () { if (fail) { throw new Error('authentication failed') } return fake }
        await tab.initializeSession()
        assert.equal(tab.sshSession, fake, 'reference: the first connection is retained')
        fail = true
        await tab.reconnect()
        assert.equal(tab.sshSession, fake, 'reference: a failed reconnect does not replace the retained connection')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === fake, 'reference: the hotkey still launches after a failed reconnect')
    })

    await check('successful-reconnect-replaces-retained-connection', async () => {
        const launched = []
        const ssh = { launchWinSCP: session => launched.push(session) }
        const tab = new Tab(injectorFor(), ssh, {}, {}, {})
        tab.profile = makeProfile()
        tab.frontend = frontendStub()
        tab.ngOnInit()
        const first = makeFakeAuthSession()
        const second = makeFakeAuthSession()
        const fakes = [first, second]
        tab.setupOneSession = async function () { return fakes.shift() }
        await tab.initializeSession()
        assert.equal(tab.sshSession, first, 'reference: the first authenticated connection is retained')
        await tab.reconnect()
        assert.equal(tab.sshSession, second, 'reference: a successful reconnect replaces the retained identity')
        const shell = tab.session
        assert.ok(shell, 'reference: the reconnect opens a new shell session')
        await shell.destroy()
        assert.equal(tab.session, null, 'reference: the ended shell is cleared')
        assert.equal(tab.sshSession, second, 'reference: the replaced identity survives the shell ending')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === second, 'reference: the hotkey launches the latest retained identity')
    })

    return failures
}

// ---------------------------------------------------------------------------------------
// Part 2 and 3 — the current native tab (real class + real TauriSshSession).
// ---------------------------------------------------------------------------------------
const CURRENT_VARIANT = {
    className: 'TauriSshTabComponent',
    source: readFile('tabby-tauri/src/ssh/tab.component.ts'),
    base: readFile('tabby-terminal/src/api/baseTerminalTab.component.ts'),
    connectable: readFile('tabby-terminal/src/api/connectableTerminalTab.component.ts'),
}

function buildCurrentTab () {
    return assemble(CURRENT_VARIANT, { TauriSshSession, KeyboardInteractivePrompt, _: text => text })
}

function makeCurrentTab (bridge, winscp, Tab = buildCurrentTab()) {
    const tab = new Tab(
        injectorFor(), bridge, { isEnabled: () => false },
        { open: () => ({ componentInstance: {}, result: Promise.resolve(null), dismiss () {} }) }, winscp,
    )
    tab.profile = makeProfile()
    tab.frontend = frontendStub()
    tab.ngOnInit()
    return tab
}

function makeWinSCP (launched, { path = WinSCP_PATH, fail = false } = {}) {
    return {
        getWinSCPPath: () => path,
        launchWinSCP: async session => { if (fail) throw new Error('private-password-must-not-appear'); launched.push(session) },
    }
}

// Load the real sftp context menu instance, wiring its imports to the assembled tab class.
function loadContextMenu (tabClass, winscp, platform = WINDOWS_PLATFORM.Windows) {
    class ContextMenuProvider {}
    const module = { exports: {} }
    vm.runInNewContext(ts.transpileModule(readFile('tabby-tauri/src/sftpContextMenu.ts'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
    }).outputText, {
        module, exports: module.exports, console,
        require: name => {
            if (name === '@angular/core') return { Injectable: () => target => target }
            if (name === 'tabby-core') return { BaseTabComponent: class {}, TabContextMenuItemProvider: ContextMenuProvider, Platform: WINDOWS_PLATFORM }
            if (name === './ssh/tab.component') return { TauriSshTabComponent: tabClass }
            if (name === './services/winscp.service') return { TauriWinSCPService: class {} }
            throw new Error(`unexpected dependency ${name}`)
        },
    })
    const Menu = module.exports.TauriSftpContextMenu
    const menu = new Menu({ platform }, winscp, { instant: text => text })
    assert.ok(menu instanceof ContextMenuProvider, 'the SFTP menu remains a tab context-menu provider')
    return menu
}

async function runCurrentPreserved () {
    const failures = []
    const check = async (name, fn) => {
        try { await fn(); console.log(`  current ${name}: pass`) } catch (error) { failures.push(`[current] ${name}: ${error.message}`) }
    }

    await check('no-authenticated-session-launches-neither-entry-point', async () => {
        const launched = []
        const winscp = makeWinSCP(launched)
        const Tab = buildCurrentTab()
        const tab = makeCurrentTab(makeBridge(), winscp, Tab)
        await tab.launchWinSCP()
        assert.equal(launched.length, 0, 'launchWinSCP without a session launches nothing')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.equal(launched.length, 0, 'the WinSCP hotkey on a fresh tab launches nothing')
        const menu = loadContextMenu(Tab, winscp)
        const items = await menu.getItems(tab)
        assert.deepEqual(Array.from(items, item => item.label), ['Open SFTP panel', 'Launch WinSCP'], 'a fresh SSH tab still offers the menu actions')
        const launch = items.find(item => item.label === 'Launch WinSCP')
        launch.click()
        await settle()
        assert.equal(launched.length, 0, 'the context-menu WinSCP action on a fresh tab launches nothing')
    })

    await check('hotkey-routing-and-unsubscribe', async () => {
        const launched = []
        const winscp = makeWinSCP(launched)
        const Tab = buildCurrentTab()
        const tab = makeCurrentTab(makeBridge(), winscp, Tab)
        assert.equal(tab.baseInits, 1, 'ngOnInit reaches the shared base exactly once')
        let reconnectCalls = 0
        tab.reconnect = async () => { reconnectCalls++ }
        tab.openSFTP = async () => { tab.sftpPanelVisible = true }
        const keys = ['home', 'end', 'restart-ssh-session', 'open-sftp', 'launch-winscp']

        for (const key of keys) tab.hotkeys.hotkey$.next(key)
        await settle()
        assert.deepEqual(Array.from(tab.events).filter(event => event[0] === 'sendInput').map(event => event[1]), [], 'unfocused hotkeys must not send input')
        assert.equal(reconnectCalls, 0, 'unfocused restart hotkey is ignored')
        assert.equal(tab.sftpPanelVisible, false, 'unfocused SFTP hotkey is ignored')
        assert.equal(launched.length, 0, 'unfocused WinSCP hotkey is ignored')

        const session = { open: true, authUsername: 'alice', activePrivateKey: true }
        tab.session = session
        tab.hasFocus = true
        for (const key of keys) tab.hotkeys.hotkey$.next(key)
        await settle()
        assert.deepEqual(Array.from(tab.events).filter(event => event[0] === 'sendInput').map(event => event[1]), ['\x1bOH', '\x1bOF'], 'focused home/end hotkeys send the expected sequences')
        assert.equal(reconnectCalls, 1, 'focused restart hotkey reconnects once')
        assert.equal(tab.sftpPanelVisible, true, 'focused SFTP hotkey opens the panel')
        assert.ok(launched.length === 1 && launched[0] === session, 'focused WinSCP hotkey launches the live session')

        tab.ngOnDestroy()
        for (const key of keys) tab.hotkeys.hotkey$.next(key)
        await settle()
        assert.equal(reconnectCalls, 1, 'ngOnDestroy unsubscribes the hotkeys')
        assert.equal(tab.sftpPanelVisible, true, 'ngOnDestroy keeps the already-open panel')
        assert.equal(launched.length, 1, 'ngOnDestroy unsubscribes the WinSCP hotkey')
    })

    await check('canClose-matrix', async () => {
        const tab = makeCurrentTab(makeBridge(), makeWinSCP([]))
        let promptCalls = 0
        let lastOptions = null
        let result = { response: 0 }
        let error = null
        tab.platform = { showMessageBox: async options => { promptCalls++; lastOptions = options; if (error) throw error; return result } }
        const config = { store: { ssh: { warnOnClose: false } } }
        tab.config = config
        tab.profile = makeProfile()

        tab.session = null
        assert.equal(await tab.canClose(), true, '1) null session returns true')
        assert.equal(promptCalls, 0, '1) null session must not prompt')

        tab.session = { open: false }
        promptCalls = 0
        assert.equal(await tab.canClose(), true, '2) ended session returns true')
        assert.equal(promptCalls, 0, '2) ended session must not prompt')

        tab.session = { open: true }
        tab.profile.options.warnOnClose = null
        config.store.ssh.warnOnClose = false
        promptCalls = 0
        assert.equal(await tab.canClose(), true, '3) config disabled returns true')
        assert.equal(promptCalls, 0, '3) config disabled must not prompt')

        tab.profile.options.warnOnClose = null
        config.store.ssh.warnOnClose = true
        result = { response: 1 }
        promptCalls = 0
        assert.equal(await tab.canClose(), false, '4) null profile inherits global true')
        assert.equal(promptCalls, 1, '4) must prompt once')
        assert.deepEqual([lastOptions.type, lastOptions.buttons.length, lastOptions.defaultId, lastOptions.cancelId], ['warning', 2, 0, 1], '4) dialog shape matches upstream')
        assert.equal(lastOptions.message, 'Disconnect from example.test?', '4) message carries the translated host')

        config.store.ssh.warnOnClose = false
        promptCalls = 0
        assert.equal(await tab.canClose(), true, '5) null profile inherits global false')
        assert.equal(promptCalls, 0, '5) global false must not prompt')

        tab.profile.options.warnOnClose = undefined
        config.store.ssh.warnOnClose = true
        result = { response: 1 }
        promptCalls = 0
        assert.equal(await tab.canClose(), false, '6) undefined profile inherits global true')
        assert.equal(promptCalls, 1, '6) must prompt once')

        tab.profile.options.warnOnClose = true
        config.store.ssh.warnOnClose = false
        result = { response: 1 }
        promptCalls = 0
        assert.equal(await tab.canClose(), false, '7) explicit true overrides global false')
        assert.equal(promptCalls, 1, '7) must prompt once')

        tab.profile.options.warnOnClose = false
        config.store.ssh.warnOnClose = true
        promptCalls = 0
        assert.equal(await tab.canClose(), true, '8) explicit false overrides global true')
        assert.equal(promptCalls, 0, '8) explicit false must not prompt')

        tab.profile.options.warnOnClose = true
        config.store.ssh.warnOnClose = true
        result = { response: 0 }
        promptCalls = 0
        assert.equal(await tab.canClose(), true, '9) confirm allows close')
        assert.equal(promptCalls, 1, '9) must prompt once')

        result = { response: 1 }
        promptCalls = 0
        assert.equal(await tab.canClose(), false, '10) cancel blocks close')
        assert.equal(promptCalls, 1, '10) must prompt once')

        result = { response: 2 }
        promptCalls = 0
        assert.equal(await tab.canClose(), false, '11) unknown response blocks close')
        assert.equal(promptCalls, 1, '11) must prompt once')

        error = new Error('dialog rejected')
        await assert.rejects(tab.canClose(), /dialog rejected/, '12) rejected dialog throws')
    })

    await check('safe-launch-error-notification', async () => {
        const tab = makeCurrentTab(makeBridge(), makeWinSCP([], { fail: true }))
        await tab.initializeSession()
        tab.translate = { instant: text => `translated:${text}` }
        await tab.launchWinSCP()
        assert.deepEqual(Array.from(tab.notices), ['translated:Could not launch WinSCP'], 'a launch failure translates the generic notification without exposing the private error')
        await tab.session.destroy()
    })

    await check('sftp-panel-and-context-menu-matrix', async () => {
        const bridge = makeBridge()
        const launched = []
        const winscp = makeWinSCP(launched)
        const Tab = buildCurrentTab()
        const tab = makeCurrentTab(bridge, winscp, Tab)
        await tab.initializeSession()
        await tab.openSFTP()
        assert.equal(tab.sftpPanelVisible, true, 'open SFTP shows the panel with a live session')
        tab.sftpPanelVisible = false
        const liveMenu = loadContextMenu(Tab, winscp)
        const openPanel = (await liveMenu.getItems(tab)).find(item => item.label === 'Open SFTP panel')
        openPanel.click()
        await settle()
        assert.equal(tab.sftpPanelVisible, true, 'the context-menu SFTP action opens the panel through the actual tab method')
        await tab.session.destroy()

        const menu = loadContextMenu(Tab, winscp)
        assert.deepEqual(Array.from(await menu.getItems({}), item => item.label), [], 'a non-SSH tab yields no menu items')
        assert.deepEqual(Array.from(await menu.getItems(tab), item => item.label), ['Open SFTP panel', 'Launch WinSCP'], 'Windows with a WinSCP path yields both actions')

        for (const platform of [WINDOWS_PLATFORM.macOS, WINDOWS_PLATFORM.Linux]) {
            const other = loadContextMenu(Tab, winscp, platform)
            assert.deepEqual(Array.from(await other.getItems(tab), item => item.label), ['Open SFTP panel'], `${platform} yields only the SFTP action`)
        }

        const noPath = loadContextMenu(Tab, { ...winscp, getWinSCPPath: () => null })
        assert.deepEqual(Array.from(await noPath.getItems(tab), item => item.label), ['Open SFTP panel'], 'a missing WinSCP path hides the WinSCP action')

        tab.session = null
        assert.equal((await menu.getItems(tab)).length, 2, 'menu construction tolerates a cleared SSH session')

        const fresh = makeCurrentTab(bridge, winscp, Tab)
        const launch = (await menu.getItems(fresh)).find(item => item.label === 'Launch WinSCP')
        launch.click()
        await settle()
        assert.equal(launched.length, 0, 'menu click on a fresh tab launches nothing')
    })

    return failures
}

async function runCurrentContract () {
    const results = []
    const defect = async (name, fn) => {
        try {
            await fn()
            console.log(`  contract ${name}: PASS (retention contract holds)`)
            results.push({ name, passed: true })
        } catch (error) {
            console.log(`  contract ${name}: FAIL: ${error.message}`)
            results.push({ name, passed: false, message: error.message })
        }
    }

    await defect('pending-first-authentication-launches-nothing', async () => {
        const launched = []
        const gate = defer()
        const bridge = makeBridge({ specs: [{ gate: gate.promise }] })
        const winscp = makeWinSCP(launched)
        const Tab = buildCurrentTab()
        const tab = makeCurrentTab(bridge, winscp, Tab)
        const initializing = tab.initializeSession()
        await settleUntil(() => bridge.connectCount === 1, 'the first authentication is pending')
        try {
            tab.hasFocus = true
            tab.hotkeys.hotkey$.next('launch-winscp')
            const launch = (await loadContextMenu(Tab, winscp).getItems(tab)).find(item => item.label === 'Launch WinSCP')
            launch.click()
            await settle()
            assert.equal(launched.length, 0, 'neither entry point launches before the first successful authentication')
        } finally {
            gate.resolve(true)
            await initializing
            await tab.destroy()
        }
    })

    await defect('hotkey-launches-after-ended-session', async () => {
        const launched = []
        const tab = makeCurrentTab(makeBridge(), makeWinSCP(launched))
        await tab.initializeSession()
        const connected = tab.session
        assert.equal(connected.open, true, 'setup: a live session exists')
        await connected.destroy()
        assert.equal(tab.session, null, 'setup: the ended session is cleared before launch')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === connected, 'the WinSCP hotkey must launch the same authenticated session from the ended tab')
    })

    await defect('context-menu-launches-after-ended-session', async () => {
        const launched = []
        const winscp = makeWinSCP(launched)
        const Tab = buildCurrentTab()
        const tab = makeCurrentTab(makeBridge(), winscp, Tab)
        await tab.initializeSession()
        const connected = tab.session
        await connected.destroy()
        const menu = loadContextMenu(Tab, winscp)
        const items = await menu.getItems(tab)
        const launch = items.find(item => item.label === 'Launch WinSCP')
        assert.ok(launch, 'setup: the WinSCP menu action is present')
        launch.click()
        await settle()
        assert.ok(launched.length === 1 && launched[0] === connected, 'the context-menu action must launch the same authenticated session from the ended tab')
    })

    await defect('failed-reconnect-keeps-previous-connection', async () => {
        const launched = []
        const bridge = makeBridge({ specs: [undefined, { reject: new Error('authentication failed') }] })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        await tab.initializeSession()
        const connected = tab.session
        await tab.reconnect()
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === connected, 'a failed reconnect must keep the last successful connection for the launch entry points')
    })

    await defect('cancelled-reconnect-keeps-previous-connection', async () => {
        const launched = []
        const gate = defer()
        const bridge = makeBridge({ specs: [undefined, { gate: gate.promise, result: { id: 'session-2', username: 'bob', usedPrivateKey: false } }] })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        await tab.initializeSession()
        const connected = tab.session
        const reconnecting = tab.reconnect()
        await settleUntil(() => bridge.connectCount === 2, 'the reconnect reaches the pending connect')
        await tab.session.destroy()
        gate.resolve(true)
        await reconnecting
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === connected, 'a cancelled reconnect must keep the last successful connection')
        await tab.destroy()
    })

    await defect('pending-reconnect-still-uses-previous-connection', async () => {
        const launched = []
        const gate = defer()
        const bridge = makeBridge({ specs: [undefined, { gate: gate.promise, result: { id: 'session-2', username: 'bob', usedPrivateKey: false } }] })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        await tab.initializeSession()
        const connected = tab.session
        const reconnecting = tab.reconnect()
        await settleUntil(() => bridge.connectCount === 2, 'the reconnect reaches the pending connect')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === connected, 'a still-pending reconnect must not lose the last successful connection')
        gate.resolve(true)
        await reconnecting
        await tab.destroy()
    })

    await defect('successful-reconnect-replaces-retained-identity', async () => {
        const launched = []
        const bridge = makeBridge({ specs: [undefined, { result: { id: 'session-2', username: 'bob', usedPrivateKey: false } }] })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        await tab.initializeSession()
        assert.equal(tab.session.authUsername, 'alice', 'setup: the first session authenticates as alice with a private key')
        assert.equal(tab.session.activePrivateKey, true, 'setup: the first session uses a private key')
        await tab.reconnect()
        const reconnected = tab.session
        assert.equal(reconnected.authUsername, 'bob', 'setup: the reconnect authenticates as bob without a private key')
        assert.equal(reconnected.activePrivateKey, false, 'setup: the reconnect does not use a private key')
        await reconnected.destroy()
        assert.equal(tab.session, null, 'setup: the ended session is cleared before launch')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === reconnected, 'a successful reconnect replaces the retained identity and the ended tab still launches it')
    })

    await defect('late-old-completion-does-not-overwrite-newer-identity', async () => {
        const launched = []
        const bridge = makeBridge({ specs: [undefined, { result: { id: 'session-2', username: 'bob', usedPrivateKey: false } }] })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        await tab.initializeSession()
        const oldConnectionId = bridge.calls.find(call => call.name === 'ssh.connect').request.connectionId
        await tab.reconnect()
        const reconnected = tab.session
        assert.equal(reconnected.authUsername, 'bob', 'setup: the newer session is live')
        bridge.emit('ssh:exit', { connectionId: oldConnectionId, exitCode: 1, signal: null })
        await settle()
        assert.equal(tab.session, reconnected, 'a stale completion from the previous connection must not overwrite the newer session')
        await reconnected.destroy()
        assert.equal(tab.session, null, 'setup: the ended session is cleared before launch')
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === reconnected, 'the retained identity after the stale completion is the newer session')
    })

    await defect('late-initialization-completion-does-not-overwrite-newer-identity', async () => {
        const launched = []
        const gate = defer()
        const bridge = makeBridge({ specs: [undefined, { result: { id: 'session-2', username: 'bob', usedPrivateKey: false } }], forwardGate: gate.promise })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        tab.profile = makeProfile({ forwardedPorts: [FORWARD_LOCAL] })
        const oldInitializing = tab.initializeSession()
        await settleUntil(() => bridge.forwardStarts.length === 1, 'the old initialization waits for forwarding')
        tab.profile = makeProfile()
        try {
            await tab.initializeSession()
            const newer = tab.session
            assert.equal(newer.authUsername, 'bob', 'the replacement authentication completes first')
            gate.resolve(true)
            await oldInitializing
            assert.equal(tab.session, newer, 'the late initialization leaves the newer live session intact')
            await newer.destroy()
            await tab.launchWinSCP()
            assert.ok(launched.length === 1 && launched[0] === newer, 'the late initialization cannot replace the retained newer authenticated identity')
        } finally {
            gate.resolve(true)
            await oldInitializing
            await tab.destroy()
        }
    })

    await defect('early-eof-during-pending-forwarding-retains-authenticated-connection', async () => {
        const launched = []
        const gate = defer()
        const bridge = makeBridge({ forwardGate: gate.promise })
        const tab = makeCurrentTab(bridge, makeWinSCP(launched))
        tab.profile = makeProfile({ forwardedPorts: [FORWARD_LOCAL] })
        const initializing = tab.initializeSession()
        await settleUntil(() => bridge.forwardStarts.length === 1, 'the forward request reaches the bridge')
        const connected = tab.session
        assert.ok(connected, 'setup: the tab holds a live session before the forward reply arrives')
        assert.equal(connected.open, true, 'setup: the session opens before the forward reply arrives')
        const connectionId = bridge.calls.find(call => call.name === 'ssh.connect').request.connectionId
        bridge.emit('ssh:exit', { connectionId, exitCode: 0, signal: null })
        await settleUntil(() => tab.session === null, 'the early EOF clears the live session')
        gate.resolve(true)
        await initializing
        await settle(8)
        tab.hasFocus = true
        tab.hotkeys.hotkey$.next('launch-winscp')
        await settle()
        assert.ok(launched.length === 1 && launched[0] === connected, 'an early EOF during a pending forward must keep the last successful connection')
    })

    await defect('destroy-and-duplicate-destroy-keep-retained-session', async () => {
        const launched = []
        const tab = makeCurrentTab(makeBridge(), makeWinSCP(launched))
        await tab.initializeSession()
        const connected = tab.session
        await tab.destroy()
        await tab.destroy()
        await tab.launchWinSCP()
        assert.ok(launched.length === 1 && launched[0] === connected, 'the retained authenticated connection must survive tab destroy and duplicate destroy')
    })

    return results
}

// ---------------------------------------------------------------------------------------
// Drive.
// ---------------------------------------------------------------------------------------
console.log('reference (verbatim upstream 14e2d60 SSHTabComponent):')
const referenceFailures = await runReference()

console.log('current preserved behavior (tabby-tauri/src/ssh/tab.component.ts):')
const preservedFailures = await runCurrentPreserved()

console.log('current retention contract:')
const contractResults = await runCurrentContract()

if (referenceFailures.length || preservedFailures.length) {
    console.error('\nreference or preserved checks failed (test infrastructure / regression):')
    for (const failure of [...referenceFailures, ...preservedFailures]) console.error(`  ${failure}`)
    process.exitCode = 1
}

const red = contractResults.filter(result => !result.passed)
if (red.length) {
    console.error(`\ntab retention defect present: ${red.length}/${contractResults.length} contract check(s) failed:`)
    for (const result of red) console.error(`  - ${result.name}: ${result.message}`)
    process.exitCode = 1
}

if (!referenceFailures.length && !preservedFailures.length && !red.length) {
    console.log('\nreference control, preserved behavior, and tab retention contract all pass')
}