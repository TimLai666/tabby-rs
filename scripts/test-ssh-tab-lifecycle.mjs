// Regression test for the SSH tab session-end policy.
//
// Upstream reference (14e2d60):
//   - tabby-ssh/src/components/sshTab.component.ts
//   - shared tabby-terminal/src/api/{baseTerminalTab,connectableTerminalTab}.component.ts
//
// Current implementation: tabby-tauri/src/ssh/tab.component.ts.
//
// The reference/current tab sources and the shared base/connectable sources are
// transpiled and executed in a VM. The lifecycle methods under test are extracted
// verbatim with the TypeScript AST from BOTH the reference and the current sources,
// so no lifecycle rule is re-implemented here. Only UI/native boundaries (write,
// frontend, destroy, initializeSession) are stubbed. The real offerReconnection and
// reconnect methods are the extracted ones, never fakes.
//
// Signatures under test: onSessionDestroyed(): void, isSessionExplicitlyTerminated(): boolean.
// Expected transcript (upstream): colored session-closed message first (host from the
// session profile), then behavior policy; reconnect is immediate (no timer); shared
// code clears the session; a reconnection prompt appears at most once and the next key
// reconnects once. The test reproduces the former delayed native-tab reconnect.

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

function resolveAnsiColors () {
    for (const from of ['tabby-tauri', 'tabby-tauri/src', 'node_modules', 'tabby-terminal', 'tabby-ssh']) {
        try {
            return require(require.resolve('ansi-colors', { paths: [path.resolve(root, from)] }))
        } catch { /* next root */ }
    }
    throw new Error('ansi-colors could not be resolved')
}
const colors = resolveAnsiColors()
colors.enabled = true

const upstream = file => execFileSync('git', ['show', `14e2d60:${file}`], { cwd: root, encoding: 'utf8' })
const currentFile = file => fs.readFileSync(path.join(root, file), 'utf8')

const VARIANTS = {
    reference: {
        className: 'SSHTabComponent',
        source: upstream('tabby-ssh/src/components/sshTab.component.ts'),
        tabMembers: ['onSessionDestroyed', 'isSessionExplicitlyTerminated'],
        base: upstream('tabby-terminal/src/api/baseTerminalTab.component.ts'),
        connectable: upstream('tabby-terminal/src/api/connectableTerminalTab.component.ts'),
    },
    current: {
        className: 'TauriSshTabComponent',
        source: currentFile('tabby-tauri/src/ssh/tab.component.ts'),
        tabMembers: ['onSessionDestroyed', 'isSessionExplicitlyTerminated', 'clearAuthPrompt', 'cancelReconnectTimer', 'lastAuthenticatedSession', 'retainAuthenticatedSession'],
        base: currentFile('tabby-terminal/src/api/baseTerminalTab.component.ts'),
        connectable: currentFile('tabby-terminal/src/api/connectableTerminalTab.component.ts'),
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

// Assemble the reference OR current shared classes plus the real tab methods into one
// runtime class. Shared lifecycle methods are verbatim; only boundaries are hand-written.
function assemble (variantName, timers, state) {
    const v = VARIANTS[variantName]
    const baseMethods = extractMembers(v.base, 'BaseTerminalTabComponent', BASE_METHODS)
    const connectableMembers = extractMembers(v.connectable, 'ConnectableTerminalTabComponent', CONNECTABLE_MEMBERS)
    const tabMethods = extractMembers(v.source, v.className, v.tabMembers)
    const code = [
        'class Session {',
        '  constructor () { this.open = true; this.profile = { options: {} }; this.tag = "session#" + (state.created.length + 1); state.created.push(this) }',
        '  async destroy () { this.open = false }',
        '  releaseInitialDataBuffer () {}',
        '}',
        'class BaseTerminalTabComponent {',
        '  constructor () { this.events = []; this.session = null; this.sessionChanged = { next: () => {} }; this.explicitProgressState = false }',
        '  get input$ () { if (!this.frontend) { throw new Error("Frontend not ready") } return this.frontend.input$ }',
        '  write (data) { this.events.push(["write", data]); return Promise.resolve() }',
        '  destroy () { this.events.push(["tab.destroy"]) }',
        '  attachSessionHandlers () {}',
        '  detachSessionHandlers () {}',
        '  setProgress () {}',
        baseMethods,
        '}',
        'class ConnectableTerminalTabComponent extends BaseTerminalTabComponent {',
        connectableMembers,
        '}',
        `class ${v.className} extends ConnectableTerminalTabComponent {`,
        '  constructor () {',
        '    super()',
        '    this.frontend = null; this.profile = { options: {} }; this.recentInputs = ""; this.translate = { instant: t => t }',
        '    this.isDisconnectedByHand = false; this.reconnectOffered = false',
        '    this.reconnectAttempts = 0; this.reconnectTimer = null',
        '    this.activeKIPrompt = null; this.activeKIProfile = null; this.activeKIRequestId = null; this.sshSession = null',
        '    this.initializeSession = async () => { this.events.push(["initializeSession"]); const s = new Session(); this.setSession(s); return s }',
        '  }',
        tabMethods,
        '}',
        `module.exports = { Tab: ${v.className} }`,
    ].join('\n')
    const mod = { exports: {} }
    vm.runInNewContext(ts.transpileModule(code, { compilerOptions: {
        target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, esModuleInterop: true,
    } }).outputText, {
        module: mod, exports: mod.exports, Promise, console, first, colors, state, _: text => text,
        setTimeout: timers.setTimeout, clearTimeout: timers.clearTimeout,
    })
    return mod.exports.Tab
}

function makeTimers () {
    const records = []
    let nextId = 1
    return {
        records,
        setTimeout: (fn, delay, ...args) => { const id = nextId++; records.push({ id, fn, delay, args }); return id },
        clearTimeout: id => { const i = records.findIndex(e => e.id === id); if (i >= 0) { records.splice(i, 1) } },
    }
}

const PROMPT = 'Press any key to reconnect\r\n'
const BEHAVIORS = ['reconnect', 'keep', 'close', 'auto']
const expectedClosed = host => '\r\n' + colors.black.bgWhite(' SSH ') + ` ${host}: session closed\r\n`

function configure (tab, { behavior, frontend, manual, recentInputs = '', host = 'fixture.test' }) {
    tab.profile = { behaviorOnSessionEnd: behavior, name: 'fixture', options: { host } }
    tab.translate = { instant: t => t }
    tab.recentInputs = recentInputs
    const input$ = new Subject()
    tab.frontend = frontend ? { input$, resetTerminalModes () {}, clear () {} } : null
    tab.isDisconnectedByHand = manual
    return input$
}

function snapshot (tab, original, timers, state) {
    const writes = tab.events.filter(e => e[0] === 'write').map(e => e[1])
    return {
        writes,
        timers: timers.records.map(e => e.delay),
        session: tab.session === null ? null : (tab.session === original ? 'original' : tab.session.tag),
        newSessions: state.created.length,
    }
}

const settle = async () => {
    await new Promise(resolve => setImmediate(resolve))
    await new Promise(resolve => setImmediate(resolve))
}

async function runDestroy (variantName, options) {
    const timers = makeTimers()
    const state = { created: [] }
    const Tab = assemble(variantName, timers, state)
    const tab = new Tab()
    const input$ = configure(tab, options)
    const original = { open: false, profile: { options: { host: options.host } }, destroy: async () => {} }
    tab.session = original
    tab.sshSession = { profile: original.profile }
    tab.onSessionDestroyed()
    const immediate = snapshot(tab, original, timers, state)
    await settle()
    return { tab, input$, timers, state, original, immediate, settled: snapshot(tab, original, timers, state) }
}

function runClosed (variantName, { behavior, manual, recentInputs }) {
    const timers = makeTimers()
    const state = { created: [] }
    const Tab = assemble(variantName, timers, state)
    const tab = new Tab()
    configure(tab, { behavior, frontend: true, manual, recentInputs })
    tab.session = { open: true, profile: { options: {} } }
    tab.sshSession = { profile: tab.session.profile }
    tab.onSessionClosed(false)
    return tab.events.some(e => e[0] === 'tab.destroy')
}

// Upstream 14e2d60 SSH semantics: Ctrl+D (0x04) or a literal "exit\r" is explicit termination.
const EXPLICIT_CASES = [
    { inputs: '', explicit: false },
    { inputs: 'exit\r', explicit: true },
    { inputs: '\x04', explicit: true },
    { inputs: 'c\x04', explicit: true },
    { inputs: 'quit\r', explicit: false },
    { inputs: 'close\r', explicit: false },
    { inputs: 'exit', explicit: false },
    { inputs: 'exit\n', explicit: false },
    { inputs: 'exit\r\n', explicit: false },
    { inputs: 'exitx\r', explicit: false },
    { inputs: '\x04abc', explicit: false },
]

async function verify (variantName) {
    const failures = []
    const expect = (cond, msg) => { if (!cond) { failures.push(msg) } }
    const host = 'fixture.test'
    const closed = expectedClosed(host)

    // 1. Abrupt destroy with a frontend: all four behaviorOnSessionEnd values.
    for (const behavior of BEHAVIORS) {
        const run = await runDestroy(variantName, { behavior, frontend: true, manual: false, host })
        expect(run.immediate.writes[0] === closed,
            `abrupt ${behavior}: first write ${JSON.stringify(run.immediate.writes[0])} != session-closed ${JSON.stringify(closed)}`)
        expect(run.immediate.timers.length === 0,
            `abrupt ${behavior}: scheduled timers ${JSON.stringify(run.immediate.timers)} (upstream reconnects without a timer)`)
        if (behavior === 'reconnect') {
            expect(run.settled.session === 'session#1', `abrupt reconnect: settled session ${run.settled.session} != session#1`)
            expect(run.settled.newSessions === 1, `abrupt reconnect: newSessions ${run.settled.newSessions} != 1`)
            expect(run.settled.writes.filter(w => w === PROMPT).length === 0, 'abrupt reconnect: unexpected key prompt')
        } else {
            expect(run.immediate.session === null, `abrupt ${behavior}: session ${run.immediate.session} != null on destroy`)
            expect(run.settled.session === null, `abrupt ${behavior}: settled session ${run.settled.session} != null`)
            expect(run.settled.newSessions === 0, `abrupt ${behavior}: unexpected reconnect (${run.settled.newSessions})`)
        }
        const prompts = run.settled.writes.filter(w => w === PROMPT).length
        expect(prompts === (behavior === 'keep' || behavior === 'auto' ? 1 : 0),
            `abrupt ${behavior}: reconnection prompts ${prompts}`)
    }

    // 2. Abrupt destroy with no frontend: upstream leaves the session untouched.
    for (const behavior of BEHAVIORS) {
        const run = await runDestroy(variantName, { behavior, frontend: false, manual: false, host })
        expect(run.settled.session === 'original', `no-frontend ${behavior}: session changed to ${run.settled.session}`)
        expect(run.settled.writes.length === 0, `no-frontend ${behavior}: wrote ${JSON.stringify(run.settled.writes)} without a frontend`)
        expect(run.settled.timers.length === 0, `no-frontend ${behavior}: timers ${JSON.stringify(run.settled.timers)} without a frontend`)
    }

    // 3. Manual disconnect: still reports the closed message, never auto-reconnects.
    const manual = await runDestroy(variantName, { behavior: 'reconnect', frontend: true, manual: true, host })
    expect(manual.immediate.writes[0] === closed, `manual reconnect: first write ${JSON.stringify(manual.immediate.writes[0])} != session-closed`)
    expect(manual.immediate.timers.length === 0, `manual reconnect: scheduled timers ${JSON.stringify(manual.immediate.timers)}`)
    expect(manual.settled.newSessions === 0, `manual reconnect: reconnected automatically (${manual.settled.newSessions})`)
    expect(manual.settled.writes.filter(w => w === PROMPT).length === 1, 'manual reconnect: expected one reconnection prompt')

    // 4. onSessionClosed: close/auto destroy rules across explicit and near-miss input.
    for (const behavior of BEHAVIORS) {
        for (const manualFlag of [false, true]) {
            for (const c of EXPLICIT_CASES) {
                const expected = !manualFlag && (behavior === 'close' || (behavior === 'auto' && c.explicit))
                const destroyed = runClosed(variantName, { behavior, manual: manualFlag, recentInputs: c.inputs })
                expect(destroyed === expected,
                    `onSessionClosed ${behavior} manual=${manualFlag} inputs=${JSON.stringify(c.inputs)}: destroyed=${destroyed} expected=${expected}`)
            }
        }
    }

    // 5. Single reconnection prompt and the next key reconnects exactly once.
    const offer = await runDestroy(variantName, { behavior: 'keep', frontend: true, manual: false, host })
    expect(offer.settled.writes.filter(w => w === PROMPT).length === 1, 'keep: expected exactly one reconnection prompt')
    offer.tab.offerReconnection()
    expect(offer.tab.events.filter(e => e[0] === 'write' && e[1] === PROMPT).length === 1, 'keep: reconnection prompt repeated')
    const before = offer.state.created.length
    offer.input$.next('fixture-key')
    await settle()
    expect(offer.state.created.length === before + 1,
        `keep: next key created ${offer.state.created.length - before} sessions, expected 1`)
    expect(offer.tab.session !== null, 'keep: reconnected session is not installed')

    // 6. Current must clear a pending auth prompt when the session is destroyed.
    if (variantName === 'current') {
        for (const frontend of [false, true]) {
            const timers = makeTimers()
            const state = { created: [] }
            const Tab = assemble(variantName, timers, state)
            const tab = new Tab()
            configure(tab, { behavior: 'keep', frontend, manual: false })
            let rejected = false
            tab.activeKIPrompt = { reject: () => { rejected = true } }
            tab.activeKIProfile = {}
            tab.activeKIRequestId = 'req'
            tab.session = { open: true, profile: { options: { host } } }
            tab.onSessionDestroyed()
            expect(rejected, `session-destroyed frontend=${frontend}: pending auth prompt was not rejected`)
            expect(tab.activeKIPrompt === null && tab.activeKIProfile === null && tab.activeKIRequestId === null,
                `session-destroyed frontend=${frontend}: pending auth state was not cleared`)
        }
    }

    if (failures.length > 0) {
        throw new Error(failures.join('\n  '))
    }
}

console.log('Loaded upstream reference (14e2d60) and current (tabby-tauri) SSH tab components.')

await verify('reference')
console.log('PASS reference: upstream SSH tab lifecycle checks all passed.')

let failed = false
try {
    await verify('current')
    console.log('PASS current: matches upstream')
} catch (error) {
    failed = true
    console.error('FAIL current: SSH tab lifecycle regressions reproduced')
    console.error(`  ${error.message}`)
}

if (failed) {
    console.error('RESULT: current SSH tab lifecycle does not match upstream 14e2d60.')
    process.exitCode = 1
} else {
    console.log('RESULT: current SSH tab lifecycle matches upstream 14e2d60.')
}
