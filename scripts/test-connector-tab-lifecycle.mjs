// Regression test for Telnet/Serial tab disconnect behavior.
//
// Upstream reference (14e2d60):
//   - tabby-telnet/src/components/telnetTab.component.ts
//   - tabby-serial/src/components/serialTab.component.ts
//   - shared tabby-terminal/src/api/{baseTerminalTab,connectableTerminalTab}.component.ts
//
// Current implementation: tabby-tauri/src/{telnet,serial}/tab.component.ts.
//
// Both tab sources are transpiled and executed in a VM. The shared lifecycle methods
// (onSessionDestroyed, offerReconnection, shouldTabBeDestroyedOnSessionClose,
// onSessionClosed, setSession) are extracted verbatim from the real shared sources with
// the TypeScript AST and assembled into a small class, so no lifecycle rule is
// re-implemented here. Only Angular/UI/native boundaries (write/destroy/session/spinner)
// are stubbed. Angular lifecycle rules live in the real extracted methods.
//
// Both variants must preserve the pinned upstream lifecycle, including its colored
// session-closed message, immediate reconnect, and manual-disconnect behavior.

import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const { first, Subject } = require('rxjs')

const here = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(here, '..')

// Resolve the real ansi-colors module the way the Tauri webpack config would
// (resolve.modules), then fall back to the plugin-level installs. A single
// instance is shared by the reference and current variants so byte output is comparable.
function resolveAnsiColors () {
    const candidates = [
        'tabby-tauri',
        'tabby-tauri/src',
        'tabby-tauri/node_modules',
        'app/node_modules',
        'node_modules',
        'tabby-terminal',
        'tabby-telnet',
        'tabby-serial',
    ]
    for (const from of candidates) {
        try {
            return require(require.resolve('ansi-colors', { paths: [path.resolve(root, from)] }))
        } catch {
            // try the next resolver root
        }
    }
    throw new Error('ansi-colors could not be resolved')
}
const colors = resolveAnsiColors()
colors.enabled = true

const upstream = file => execFileSync('git', ['show', `14e2d60:${file}`], { cwd: root, encoding: 'utf8' })
const currentFile = file => fs.readFileSync(path.join(root, file), 'utf8')

const VARIANTS = {
    reference: {
        telnet: {
            source: upstream('tabby-telnet/src/components/telnetTab.component.ts'),
            className: 'TelnetTabComponent',
            construct: Tab => new Tab({}),
        },
        serial: {
            source: upstream('tabby-serial/src/components/serialTab.component.ts'),
            className: 'SerialTabComponent',
            construct: Tab => new Tab({}, { show: async () => 9600 }),
        },
        shared: {
            base: upstream('tabby-terminal/src/api/baseTerminalTab.component.ts'),
            connectable: upstream('tabby-terminal/src/api/connectableTerminalTab.component.ts'),
        },
    },
    current: {
        telnet: {
            source: currentFile('tabby-tauri/src/telnet/tab.component.ts'),
            className: 'TauriTelnetTabComponent',
            construct: Tab => new Tab({}, {}),
        },
        serial: {
            source: currentFile('tabby-tauri/src/serial/tab.component.ts'),
            className: 'TauriSerialTabComponent',
            construct: Tab => new Tab({}, {}, { show: async () => 9600 }),
        },
        shared: {
            base: currentFile('tabby-terminal/src/api/baseTerminalTab.component.ts'),
            connectable: currentFile('tabby-terminal/src/api/connectableTerminalTab.component.ts'),
        },
    },
}

const BASE_METHODS = [
    'setSession',
    'onSessionClosed',
    'shouldTabBeDestroyedOnSessionClose',
    'onSessionDestroyed',
    'isSessionExplicitlyTerminated',
]
const CONNECTABLE_MEMBERS = [
    'reconnectOffered',
    'isDisconnectedByHand',
    'initializeSession',
    'onSessionDestroyed',
    'offerReconnection',
    'shouldTabBeDestroyedOnSessionClose',
    'clearServiceMessagesOnConnect',
    'reconnect',
    'disconnect',
]

function extractMembers (sourceText, className, names) {
    const source = ts.createSourceFile(`${className}.ts`, sourceText, ts.ScriptTarget.Latest, true)
    const cls = source.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    if (!cls) {
        throw new Error(`class ${className} not found`)
    }
    const printer = ts.createPrinter()
    const out = []
    for (const member of cls.members) {
        const name = member.name && (ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)) ? member.name.text : undefined
        if (!names.includes(name)) {
            continue
        }
        if (ts.isMethodDeclaration(member) || ts.isPropertyDeclaration(member)) {
            out.push(printer.printNode(ts.EmitHint.Unspecified, member, source))
        }
    }
    return out.join('\n')
}

// Build the shared base/connectable runtime class from the real extracted methods.
// Only UI/native boundaries are provided by hand; the lifecycle methods are verbatim.
function buildShared ({ base, connectable }) {
    const baseMethods = extractMembers(base, 'BaseTerminalTabComponent', BASE_METHODS)
    const connectableMembers = extractMembers(connectable, 'ConnectableTerminalTabComponent', CONNECTABLE_MEMBERS)
    const code = [
        'class BaseTerminalTabComponent {',
        '    static template = ""',
        '    static styles = []',
        '    static animations = []',
        '    constructor () {',
        '        this.events = []',
        '        this.session = null',
        '        this.sessionChanged = { next: () => {} }',
        '        this.explicitProgressState = false',
        '    }',
        '    get input$ () {',
        '        if (!this.frontend) { throw new Error("Frontend not ready") }',
        '        return this.frontend.input$',
        '    }',
        '    write (data) { this.events.push(["write", data]); return Promise.resolve() }',
        '    destroy () { this.events.push(["tab.destroy"]) }',
        '    attachSessionHandler () {}',
        '    attachSessionHandlers () {}',
        '    detachSessionHandlers () {}',
        '    setProgress () {}',
        '    setProgressState () {}',
        '    startSpinner () {}',
        '    stopSpinner () {}',
        '    ngOnDestroy () {}',
        baseMethods,
        '}',
        'class ConnectableTerminalTabComponent extends BaseTerminalTabComponent {',
        connectableMembers,
        '}',
        'module.exports = { BaseTerminalTabComponent, ConnectableTerminalTabComponent }',
    ].join('\n')

    const sharedModule = { exports: {} }
    vm.runInNewContext(ts.transpileModule(code, { compilerOptions: {
        target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
    } }).outputText, {
        module: sharedModule, exports: sharedModule.exports, first, _: text => text,
        setTimeout, clearTimeout, Promise, console,
    })
    return sharedModule.exports
}

function makeTimers () {
    const records = []
    let nextId = 1
    const setTimeout = (fn, delay, ...args) => {
        const id = nextId++
        records.push({ id, fn, delay, args })
        return id
    }
    const clearTimeout = id => {
        const index = records.findIndex(entry => entry.id === id)
        if (index >= 0) {
            records.splice(index, 1)
        }
    }
    return { records, setTimeout, clearTimeout }
}

function sessionModulesFor (kind, variantName, Session) {
    if (variantName === 'reference') {
        return kind === 'telnet'
            ? { '../session': { TelnetSession: Session } }
            : { '../api': { SerialSession: Session, BAUD_RATES: [] } }
    }
    return kind === 'telnet'
        ? { './session': { TauriTelnetSession: Session }, './profile': {} }
        : { './session': { TauriSerialSession: Session }, './profile': { BAUD_RATES: [] } }
}

// Transpile and evaluate the real tab component, wiring the real shared class as its base.
function buildRuntime (kind, variantName, shared, timers) {
    const variant = VARIANTS[variantName]
    const tab = variant[kind]
    const created = []

    class Session {
        constructor () {
            this.open = true
            this.profile = { options: {} }
            this.tag = `session#${created.length + 1}`
            created.push(this)
        }
        async start () {}
        resize () {}
        emitServiceMessage () {}
        async destroy () { this.open = false }
        releaseInitialDataBuffer () {}
        setBaudRate () { return Promise.resolve() }
    }

    const sessionModules = sessionModulesFor(kind, variantName, Session)
    const loader = dep => {
        if (dep === 'ansi-colors') {
            return colors
        }
        if (dep === '@angular/core') {
            return { Component: () => target => target }
        }
        if (dep === '@biesbjerg/ngx-translate-extract-marker') {
            return { marker: text => text }
        }
        if (dep === 'tabby-core') {
            return { Platform: { macOS: 'macOS', Windows: 'Windows', Linux: 'Linux' }, SelectorService: class {} }
        }
        if (dep === 'tabby-terminal') {
            return shared
        }
        if (dep === '../api/hostBridge') {
            return {}
        }
        if (Object.prototype.hasOwnProperty.call(sessionModules, dep)) {
            return sessionModules[dep]
        }
        if (dep.endsWith('.pug')) {
            return ''
        }
        throw new Error(`Unexpected dependency: ${dep}`)
    }

    const tabModule = { exports: {} }
    vm.runInNewContext(ts.transpileModule(tab.source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
        esModuleInterop: true, experimentalDecorators: true,
    } }).outputText, {
        module: tabModule, exports: tabModule.exports, require: loader,
        setTimeout: timers.setTimeout, clearTimeout: timers.clearTimeout,
        setImmediate: cb => cb(), Promise, console,
    })
    const Tab = tabModule.exports[tab.className]
    if (!Tab) {
        throw new Error(`${tab.className} not exported`)
    }
    return { Tab, created, construct: () => tab.construct(Tab) }
}

function expectedClosed (kind, host) {
    return kind === 'telnet'
        ? '\r\n' + colors.black.bgWhite(' TELNET ') + ` ${host}: session closed\r\n`
        : '\r\n' + colors.black.bgWhite(' SERIAL ') + ' session closed\r\n'
}

const PROMPT = 'Press any key to reconnect\r\n'

function configure (tab, { behavior, frontend, manual, recentInputs = '' }) {
    tab.profile = { behaviorOnSessionEnd: behavior, clearServiceMessagesOnConnect: false, name: 'fixture' }
    tab.translate = { instant: text => text }
    tab.notifications = { notice () {}, error () {} }
    tab.size = { columns: 80, rows: 24 }
    tab.recentInputs = recentInputs
    const input$ = new Subject()
    tab.frontend = frontend
        ? { input$, resetTerminalModes () {}, clear () {} }
        : null
    tab.isDisconnectedByHand = manual
    return input$
}

function snapshot (tab, original, created, timers) {
    const events = Array.from(tab.events)
    return {
        writes: events.filter(event => event[0] === 'write').map(event => event[1]),
        timers: timers.records.map(entry => entry.delay),
        session: tab.session === null ? null : tab.session === original ? 'original' : tab.session.tag,
        newSessions: created.length,
    }
}

const settle = async () => {
    await new Promise(resolve => setImmediate(resolve))
    await new Promise(resolve => setImmediate(resolve))
}

async function runDestroy (kind, variantName, shared, options) {
    const timers = makeTimers()
    const { created, construct } = buildRuntime(kind, variantName, shared, timers)
    const tab = construct()
    const input$ = configure(tab, options)
    const original = {
        open: false,
        profile: { options: { host: options.host ?? 'fixture.test' } },
        destroy: async () => tab.events.push(['session.destroy']),
        releaseInitialDataBuffer () {},
    }
    tab.session = original
    tab.onSessionDestroyed()
    const immediate = snapshot(tab, original, created, timers)
    await settle()
    const settled = snapshot(tab, original, created, timers)
    return { tab, input$, created, timers, immediate, settled }
}

function runClosed (kind, variantName, shared, { behavior, manual, recentInputs = '' }) {
    const timers = makeTimers()
    const { created, construct } = buildRuntime(kind, variantName, shared, timers)
    const tab = construct()
    configure(tab, { behavior, frontend: true, manual, recentInputs })
    tab.session = { open: true, profile: { options: {} } }
    tab.onSessionClosed(false)
    return tab.events.some(event => event[0] === 'tab.destroy')
}

const BEHAVIORS = ['reconnect', 'keep', 'close', 'auto']

async function verifyImplementation (variantName, kind) {
    const shared = SHARED[variantName]
    const label = `${variantName}/${kind}`
    const failures = []
    const expect = (condition, message) => {
        if (!condition) {
            failures.push(message)
        }
    }
    const host = 'fixture.test'

    // 1. Abrupt destroy with a frontend: all four behaviorOnSessionEnd values.
    for (const behavior of BEHAVIORS) {
        const run = await runDestroy(kind, variantName, shared, { behavior, frontend: true, manual: false, host })
        const closed = expectedClosed(kind, host)
        expect(run.immediate.writes[0] === closed,
            `${label} abrupt ${behavior}: first write ${JSON.stringify(run.immediate.writes[0])}` +
            ` != upstream session-closed ${JSON.stringify(closed)}`)
        expect(run.immediate.timers.length === 0,
            `${label} abrupt ${behavior}: scheduled reconnect timers ${JSON.stringify(run.immediate.timers)}` +
            ' (upstream reconnects without a timer)')
        if (behavior === 'reconnect') {
            expect(run.settled.session === 'session#1',
                `${label} abrupt reconnect: settled session ${run.settled.session} != session#1`)
            expect(run.settled.newSessions === 1,
                `${label} abrupt reconnect: newSessions ${run.settled.newSessions} != 1`)
            expect(run.settled.writes.filter(write => write === PROMPT).length === 0,
                `${label} abrupt reconnect: unexpected key prompt before reconnecting`)
        } else {
            expect(run.immediate.session === null,
                `${label} abrupt ${behavior}: session ${run.immediate.session} != null on destroy`)
            expect(run.settled.session === null,
                `${label} abrupt ${behavior}: settled session ${run.settled.session} != null`)
            expect(run.settled.newSessions === 0,
                `${label} abrupt ${behavior}: unexpected automatic reconnect (${run.settled.newSessions})`)
        }
        const prompts = run.settled.writes.filter(write => write === PROMPT).length
        if (behavior === 'keep' || behavior === 'auto') {
            expect(prompts === 1, `${label} abrupt ${behavior}: prompts ${prompts} != 1`)
        } else {
            expect(prompts === 0, `${label} abrupt ${behavior}: unexpected prompts ${prompts}`)
        }
    }

    // 2. Abrupt destroy with no frontend: upstream leaves the session untouched.
    for (const behavior of BEHAVIORS) {
        const run = await runDestroy(kind, variantName, shared, { behavior, frontend: false, manual: false, host })
        expect(run.settled.session === 'original',
            `${label} no-frontend ${behavior}: session changed to ${run.settled.session}, expected unchanged`)
        expect(run.settled.writes.length === 0,
            `${label} no-frontend ${behavior}: wrote ${JSON.stringify(run.settled.writes)} without a frontend`)
        expect(run.settled.timers.length === 0,
            `${label} no-frontend ${behavior}: scheduled timers ${JSON.stringify(run.settled.timers)} without a frontend`)
    }

    // 3. Manual disconnect: still reports the closed message, never auto-reconnects.
    const manual = await runDestroy(kind, variantName, shared, { behavior: 'reconnect', frontend: true, manual: true, host })
    expect(manual.immediate.writes[0] === expectedClosed(kind, host),
        `${label} manual reconnect: first write ${JSON.stringify(manual.immediate.writes[0])}` +
        ` != upstream session-closed ${JSON.stringify(expectedClosed(kind, host))}`)
    expect(manual.immediate.timers.length === 0,
        `${label} manual reconnect: scheduled timers ${JSON.stringify(manual.immediate.timers)}`)
    expect(manual.settled.newSessions === 0,
        `${label} manual reconnect: reconnected automatically (${manual.settled.newSessions})`)
    expect(manual.settled.writes.filter(write => write === PROMPT).length === 1,
        `${label} manual reconnect: expected one reconnection prompt`)

    // 4. onSessionClosed: close/auto destroy rules with explicit quit/close and manual flag.
    for (const behavior of BEHAVIORS) {
        for (const manualFlag of [false, true]) {
            for (const recentInputs of ['', 'quit\r', 'close\r', 'xclose\r', 'quit']) {
                const explicit = recentInputs.endsWith('close\r') || recentInputs.endsWith('quit\r')
                const expected = !manualFlag &&
                    (behavior === 'close' || (behavior === 'auto' && explicit))
                const destroyed = runClosed(kind, variantName, shared, {
                    behavior, manual: manualFlag, recentInputs,
                })
                expect(destroyed === expected,
                    `${label} onSessionClosed ${behavior} manual=${manualFlag} inputs=${JSON.stringify(recentInputs)}:` +
                    ` destroyed=${destroyed} expected=${expected}`)
            }
        }
    }

    // 5. Single reconnection prompt and the next key reconnects once.
    const offer = await runDestroy(kind, variantName, shared, { behavior: 'keep', frontend: true, manual: false, host })
    expect(offer.settled.writes.filter(write => write === PROMPT).length === 1,
        `${label} keep: expected exactly one reconnection prompt`)
    offer.tab.offerReconnection()
    expect(Array.from(offer.tab.events).filter(event => event[0] === 'write' && event[1] === PROMPT).length === 1,
        `${label} keep: reconnection prompt repeated`)
    const beforeKey = offer.created.length
    offer.input$.next('fixture-key')
    await settle()
    expect(offer.created.length === beforeKey + 1,
        `${label} keep: next key created ${offer.created.length - beforeKey} sessions, expected 1`)
    expect(offer.tab.session !== null, `${label} keep: reconnected session is not installed`)

    if (failures.length > 0) {
        throw new Error(failures.join('\n  '))
    }
}

const SHARED = {
    reference: buildShared(VARIANTS.reference.shared),
    current: buildShared(VARIANTS.current.shared),
}

console.log('Loaded upstream reference (14e2d60) and current (tabby-tauri) tab components.')

for (const kind of ['telnet', 'serial']) {
    await verifyImplementation('reference', kind)
}
console.log('PASS reference: upstream tab lifecycle checks all passed.')

// Evidence for the report: the abrupt reconnect transcript for both variants.
async function reconnectTranscript (variantName, kind) {
    const run = await runDestroy(kind, variantName, SHARED[variantName], { behavior: 'reconnect', frontend: true, manual: false })
    return { writes: run.immediate.writes, timers: run.immediate.timers, session: run.immediate.session, settledSession: run.settled.session }
}
console.log('reference reconnect transcript:', JSON.stringify(await reconnectTranscript('reference', 'telnet')))
console.log('current   reconnect transcript:', JSON.stringify(await reconnectTranscript('current', 'telnet')))

let failed = false
for (const kind of ['telnet', 'serial']) {
    try {
        await verifyImplementation('current', kind)
        console.log(`PASS current/${kind}: matches upstream`)
    } catch (error) {
        failed = true
        console.error(`FAIL current/${kind}: lifecycle regression reproduced`)
        console.error(`  ${error.message}`)
    }
}

if (failed) {
    console.error('RESULT: current Telnet/Serial tab lifecycle does not match upstream 14e2d60.')
    process.exitCode = 1
} else {
    console.log('RESULT: current Telnet/Serial tab lifecycle matches upstream 14e2d60.')
}
