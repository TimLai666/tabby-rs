import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'
import ts from 'typescript'

// Regression contract for SSH port-forwarding startup, checked against upstream 14e2d60.
// Once ssh.connect succeeds, a rejected ssh.forwardingStart for a Local, Dynamic, or
// Remote forward must not destroy the authenticated session, emit destroyed, or
// re-authenticate. It emits a service message carrying the exact upstream forward
// description (ForwardedPort.toString) and the failure reason. This file asserts failure
// isolation and diagnostic contracts only; it does not establish full forwarding startup
// or timing parity.
//
// The real TauriSshSession.start/startForwardings and the real BaseSession lifecycle run
// here; only Angular UI, the native bridge, and auth preparation are replaced. The
// reference control runs the verbatim upstream addPortForward method and the entire
// verbatim ForwardedPort class declaration, both extracted from git with the TypeScript
// AST; only its per-entry startLocalListener transport and the russh authenticated client
// are stubbed. The reference collector awaits every background forwarding diagnostic
// before asserting, so it is a diagnostic check, not an upstream startup timing test.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const cache = new Map()
const logger = { info () {}, debug () {}, warn () {}, error () {} }

function load (file) {
    file = path.resolve(root, file)
    if (cache.has(file)) return cache.get(file).exports
    const module = { exports: {} }
    cache.set(file, module)
    const require = name => {
        if (name === '@angular/core') return { Injector: class {}, InjectionToken: class {} }
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
        module, exports: module.exports, require, Buffer, TextDecoder, console,
        setTimeout, clearTimeout, setInterval, clearInterval, process, window: { crypto: { randomUUID } },
    }, { filename: file })
    return module.exports
}

const { TauriSshSession } = load('tabby-tauri/src/ssh/session.ts')
const { BaseSession } = load('tabby-terminal/src/session.ts')
const { Subject } = createRequire(import.meta.url)('rxjs')
const stripAnsi = createRequire(import.meta.url)('strip-ansi')
const colors = createRequire(path.join(root, 'tabby-ssh/package.json'))('ansi-colors')
const failureBadge = colors.bgRed.black(' X ')

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
    return text.replace(/^export\s+/, '')
}

// Verbatim upstream source; never rewritten below. The whole ForwardedPort class is
// extracted so its field defaults stay exact: only host initializes to 127.0.0.1 and
// listener to null, while type/port/targetAddress/targetPort stay undefined.
const addPortForwardSource = extractMethod(gitShow('14e2d60:tabby-ssh/src/session/ssh.ts'), 'SSHSession', 'addPortForward')
const forwardedPortClassSource = extractClass(gitShow('14e2d60:tabby-ssh/src/session/forwards.ts'), 'ForwardedPort')

const referenceBody = ts.transpileModule(`
${forwardedPortClassSource}
class ReferenceSshSession extends BaseSession {
    constructor (profile, transport) {
        super(logger)
        this.profile = profile
        this.transport = transport
        this.serviceMessage = new Subject()
        this.forwardedPorts = []
        this.ssh = new russh.AuthenticatedSSHClient()
        this.ssh.forwardTCPPort = (host, port) => this.transport.forwardTCPPort(host, port)
    }
    get serviceMessage$ () { return this.serviceMessage.asObservable() }
    emitServiceMessage (message) { this.serviceMessage.next(message) }
    setupSocketChannelEvents () {}
    async start () {
        // Upstream launches each forward without awaiting a single result. Here every
        // background forwarding diagnostic is collected before asserting, so this is a
        // diagnostic collector, not an upstream startup timing test.
        this.open = true
        const pending = []
        for (const entry of this.profile.options.forwardedPorts) {
            const fw = Object.assign(new ForwardedPort(), entry)
            fw.startLocalListener = () => this.transport.startLocalListener(fw)
            pending.push(this.addPortForward(fw).catch(() => undefined))
        }
        await Promise.all(pending)
    }
    async destroy () {
        if (this.destroying) { return }
        this.destroying = true
        this.serviceMessage.complete()
        await super.destroy()
    }
    async gracefullyKillProcess () {}
    resize () {}
    write () {}
    kill () {}
    supportsWorkingDirectory () { return false }
    async getWorkingDirectory () { return null }
    ${addPortForwardSource}
}
`, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.None } }).outputText

const { ReferenceSshSession, ReferenceForwardedPort } = new Function(
    'BaseSession', 'Subject', 'colors', 'russh', 'PortForwardType', 'logger',
    `${referenceBody}\nreturn { ReferenceSshSession, ReferenceForwardedPort: ForwardedPort }`,
)(
    BaseSession, Subject, colors, { AuthenticatedSSHClient: class AuthenticatedSSHClient {} },
    { Local: 'Local', Remote: 'Remote', Dynamic: 'Dynamic' }, logger,
)

const descriptionOf = fw => Object.assign(new ReferenceForwardedPort(), fw).toString()

const local = { type: 'Local', host: '127.0.0.1', port: 8080, targetAddress: 'db.internal', targetPort: 5432 }
const dynamic = { type: 'Dynamic', host: '127.0.0.1', port: 1080 }
const remote = { type: 'Remote', host: '127.0.0.1', port: 9090, targetAddress: 'files.internal', targetPort: 22 }
const forwards = [local, dynamic, remote]
// Omitted-target diagnostics: these entries carry no targetAddress/targetPort, so the
// upstream description renders the undefined values literally. The cases force a
// transport rejection only to compare the emitted failure description; they do not
// establish actual target validation or startup success parity.
const omittedTargetForwards = [{ type: 'Local', port: 8081 }, { type: 'Remote', port: 9091 }]
const malformedForwards = [local, null, {}, { port: 8081 }, { type: 4 }, { type: 'Unknown' }, { type: 'local' }, dynamic, remote]

function defer () {
    let resolve
    const promise = new Promise(resolution => { resolve = resolution })
    return { promise, resolve }
}

async function settleTo (predicate, label, timeout = 5000) {
    const start = Date.now()
    while (!predicate()) {
        if (Date.now() - start > timeout) {
            throw new Error(`timed out waiting for ${label}`)
        }
        await new Promise(resolve => setImmediate(resolve))
    }
}

function makeProfile (forwardedPorts) {
    return { id: 'test', options: { host: 'example.test', port: 22, user: 'alice', auth: 'password', password: '',
        privateKeys: [], forwardedPorts, environment: {}, x11: false, agentForward: false, keepaliveInterval: 0,
        keepaliveCountMax: 0, jumpHost: null, input: { backspace: 'backspace' }, scripts: [] } }
}

function makeBridge (failures = new Map(), { forwardGate = null } = {}) {
    const bridge = { calls: [], connectCount: 0, forwardStarts: [], listeners: new Map() }
    bridge.listen = async (name, callback) => { bridge.listeners.set(name, callback); return () => bridge.listeners.delete(name) }
    bridge.invoke = async (name, request) => {
        bridge.calls.push({ name, request })
        if (name === 'ssh.connect') { bridge.connectCount++; return { id: 'session-1', username: 'alice', usedPrivateKey: true } }
        if (name === 'ssh.forwardingStart') {
            bridge.forwardStarts.push(request)
            if (forwardGate) { await forwardGate }
            if (failures.has(request.bindPort)) {
                const failure = failures.get(request.bindPort)
                throw typeof failure === 'string' ? new Error(failure) : failure
            }
            return { id: `fwd-${request.bindPort}` }
        }
        return {}
    }
    return bridge
}

function makeTransport (failures = new Map()) {
    const attempted = []
    const reasonFor = port => failures.has(port) ? Promise.reject(new Error(failures.get(port))) : Promise.resolve()
    return { attempted,
        startLocalListener: fw => { attempted.push({ type: fw.type, port: fw.port }); return reasonFor(fw.port) },
        forwardTCPPort: (host, port) => { attempted.push({ type: 'Remote', port }); return reasonFor(port) } }
}

async function run (createSession, forwardedPorts, failures = new Map()) {
    const bridge = makeBridge(failures)
    const session = createSession(makeProfile(forwardedPorts), bridge, failures)
    const messages = []
    const destroyed = []
    session.serviceMessage$.subscribe(message => messages.push(message))
    session.destroyed$.subscribe(() => destroyed.push(true))
    const error = await Promise.resolve().then(() => session.start()).then(() => null, value => value)
    return { bridge, session, messages, destroyed, error }
}

function assertFailureMessage (label, messages, failed, reason) {
    const phrase = failed.type === 'Remote' ? 'Remote rejected port forwarding for' : 'Failed to forward port'
    const stripped = messages.map(value => stripAnsi(value))
    const message = stripped.find(value => value.includes(phrase))
    assert.ok(message, `${label}: emits the failure service message`)
    assert.ok(message.includes(descriptionOf(failed)), `${label}: message carries the forward description`)
    assert.ok(message.includes(reason), `${label}: message carries the failure reason`)
    const raw = messages.find(value => stripAnsi(value).includes(phrase))
    assert.ok(raw.includes(failureBadge), `${label}: message carries the upstream red failure badge`)
}

function checkLive (label, result) {
    assert.equal(result.error, null, `${label}: a rejected forward must not reject or destroy the session`)
    assert.equal(result.session.open, true, `${label}: session stays open`)
    assert.deepEqual(result.destroyed, [], `${label}: destroyed$ must not fire`)
    assert.equal(result.bridge.connectCount, 1, `${label}: no re-authentication`)
    assert.ok(!result.bridge.calls.some(call => call.name === 'ssh.close'), `${label}: SSH must not be closed`)
    assert.equal(result.session.authUsername, 'alice', `${label}: authenticated username preserved`)
    assert.equal(result.session.activePrivateKey, true, `${label}: private-key flag preserved`)
}

async function runReference () {
    const results = []
    const check = async (name, fn) => {
        try { await fn(); console.log(`  reference ${name}: pass`) } catch (error) { results.push(`[reference] ${name}: ${error.message}`) }
    }
    await check('forwarded-port-field-initialization', async () => {
        const fresh = new ReferenceForwardedPort()
        assert.equal(fresh.host, '127.0.0.1', 'reference: host initializes to 127.0.0.1')
        assert.equal(fresh.listener, null, 'reference: listener initializes to null')
        assert.equal(fresh.type, undefined, 'reference: type stays undefined')
        assert.equal(fresh.port, undefined, 'reference: port stays undefined')
        assert.equal(fresh.targetAddress, undefined, 'reference: targetAddress stays undefined')
        assert.equal(fresh.targetPort, undefined, 'reference: targetPort stays undefined')
        const assigned = Object.assign(new ReferenceForwardedPort(), { type: 'Local', port: null, targetAddress: null, targetPort: null })
        assert.equal(assigned.type, 'Local', 'reference: an assigned type is retained')
        assert.equal(assigned.port, null, 'reference: an explicit null port is retained')
        assert.equal(assigned.targetAddress, null, 'reference: an explicit null targetAddress is retained')
        assert.equal(assigned.targetPort, null, 'reference: an explicit null targetPort is retained')
    })
    const cases = [
        ...[local, dynamic, remote, ...omittedTargetForwards].map(fw => ({ name: `${fw.type.toLowerCase()}-forwarding-rejected${omittedTargetForwards.includes(fw) ? '-omitted-target' : ''}`, pf: [fw], failed: fw, reason: 'boom' })),
        ...[0, 1, 2].map(index => ({ name: `mixed-failure-at-${index}`, pf: forwards, failed: forwards[index], reason: 'rejected' })),
    ]
    for (const item of cases) {
        await check(item.name, async () => {
            const result = await run((profile, _bridge, failures) => new ReferenceSshSession(profile, makeTransport(failures)), item.pf, new Map([[item.failed.port, item.reason]]))
            try {
                assert.equal(result.error, null, 'reference: start must resolve')
                assert.equal(result.session.open, true, 'reference: session stays open')
                assert.deepEqual(result.destroyed, [], 'reference: must not destroy on a forwarding failure')
                assert.equal(result.session.transport.attempted.length, item.pf.length, 'reference: every forward is attempted')
                assert.ok(result.session.transport.attempted.some(value => value.port === item.failed.port), 'reference: the failing forward is attempted')
                assertFailureMessage('reference', result.messages, item.failed, item.reason)
            } finally {
                await result.session.destroy()
            }
        })
    }
    await check('malformed-forwarding-entries', async () => {
        const result = await run((profile, _bridge, failures) => new ReferenceSshSession(profile, makeTransport(failures)), malformedForwards)
        try {
            assert.equal(result.error, null, 'reference: malformed entries do not reject startup')
            assert.equal(result.session.open, true, 'reference: authenticated session remains open')
            assert.deepEqual(result.destroyed, [], 'reference: malformed entries do not destroy SSH')
            assert.deepEqual(result.session.transport.attempted.map(value => value.port), forwards.map(value => value.port), 'reference: only valid entries are attempted')
        } finally { await result.session.destroy() }
    })
    return results
}

async function runCurrent () {
    const results = []
    const check = async (name, fn) => {
        try { await fn(); console.log(`  current ${name}: pass`) } catch (error) { results.push(`[current] ${name}: ${error.message}`) }
    }
    const create = (profile, bridge) => new TauriSshSession(injectorFor(), bridge, { isEnabled: () => false }, profile, modals)
    const teardown = async (result, failures) => {
        await result.session.destroy()
        const stopped = result.bridge.calls.filter(call => call.name === 'ssh.forwardingStop').map(call => call.request.id)
        const successful = result.bridge.forwardStarts.filter(start => !failures.has(start.bindPort)).map(start => `fwd-${start.bindPort}`)
        const failed = result.bridge.forwardStarts.filter(start => failures.has(start.bindPort)).map(start => `fwd-${start.bindPort}`)
        assert.deepEqual([...stopped].sort(), [...successful].sort(), 'stopped IDs equal the successful forwarding IDs')
        assert.equal(new Set(stopped).size, stopped.length, 'each successful forwarding ID is stopped exactly once')
        for (const id of failed) { assert.ok(!stopped.includes(id), `failed forwarding ${id} must not be stopped`) }
        assert.equal(result.bridge.calls.filter(call => call.name === 'ssh.close').length, 1, 'SSH is closed exactly once on teardown')
    }
    const guarded = async (result, failures, primaryAssertions) => {
        let primary = null
        try { await primaryAssertions() } catch (error) { primary = error }
        await teardown(result, failures)
        if (primary) { throw primary }
    }

    await check('empty-forwarding-list', async () => {
        const failures = new Map()
        const result = await run(create, [], failures)
        await guarded(result, failures, async () => {
            checkLive('empty', result)
            assert.equal(result.bridge.forwardStarts.length, 0, 'empty: nothing forwarded')
        })
    })

    for (const fw of [local, dynamic, remote, ...omittedTargetForwards]) {
        await check(`${fw.type.toLowerCase()}-forwarding-rejected${omittedTargetForwards.includes(fw) ? '-omitted-target' : ''}`, async () => {
            const failures = new Map([[fw.port, 'boom']])
            const result = await run(create, [fw], failures)
            await guarded(result, failures, async () => {
                checkLive(fw.type, result)
                assert.equal(result.bridge.forwardStarts.length, 1, `${fw.type}: forward attempted`)
                assertFailureMessage(fw.type, result.messages, fw, 'boom')
            })
        })
    }

    for (let index = 0; index < 3; index++) {
        await check(`mixed-failure-at-${index}`, async () => {
            const failures = new Map([[forwards[index].port, 'rejected']])
            const result = await run(create, forwards, failures)
            await guarded(result, failures, async () => {
                checkLive(`mixed-${index}`, result)
                assert.equal(result.bridge.forwardStarts.length, 3, 'mixed: remaining forwards still attempted')
                assertFailureMessage(`mixed-${index}`, result.messages, forwards[index], 'rejected')
            })
        })
    }

    await check('all-forwardings-succeed', async () => {
        const failures = new Map()
        const result = await run(create, forwards, failures)
        await guarded(result, failures, async () => {
            checkLive('success', result)
            assert.equal(result.bridge.forwardStarts.length, 3, 'success: all forwards started')
            assert.deepEqual(result.messages, forwards.map(fw =>
                colors.bgGreen.black(fw.type === 'Remote' ? ' <- ' : ' -> ') + ` Forwarded ${descriptionOf(fw)}`),
            'success: every forward carries the exact upstream green badge, arrow, and description')
            assert.equal(result.messages.filter(value => value.includes('Error')).length, 0, 'success: no failure messages')
        })
    })

    await check('normal-io-after-forwards', async () => {
        const failures = new Map()
        const result = await run(create, [local], failures)
        await guarded(result, failures, async () => {
            checkLive('io', result)
            result.session.write(Buffer.from('hello'))
            result.session.resize(120, 40)
            await Promise.resolve()
            const write = result.bridge.calls.find(call => call.name === 'ssh.write')
            const resize = result.bridge.calls.find(call => call.name === 'ssh.resize')
            assert.equal(write?.request.id, 'session-1', 'io: write targets the authenticated session')
            assert.equal(write?.request.data.join(','), '104,101,108,108,111', 'io: write preserves its bytes')
            assert.equal(resize?.request.id, 'session-1', 'io: resize targets the authenticated session')
            assert.equal(resize?.request.columns, 120, 'io: resize reaches the bridge')
        })
    })

    await check('native-structured-forwarding-error', async () => {
        const details = 'fixture bind rejected'
        const failures = new Map([[local.port, { code: 'io', details }]])
        const result = await run(create, [local, dynamic], failures)
        await guarded(result, failures, async () => {
            checkLive('native error', result)
            assertFailureMessage('native error', result.messages, local, details)
            assert.equal(result.bridge.forwardStarts.length, 2, 'native error: later forward is still attempted')
        })
    })

    await check('output-during-pending-destroy', async () => {
        // Native controls can await a peer reply while close is queued. Late output
        // must not turn a closing session into an unbounded pre-connect buffer.
        const bridge = makeBridge()
        const invoke = bridge.invoke
        let releaseClose, closeStarted
        const closeWait = new Promise(resolve => { releaseClose = resolve })
        const closeEntered = new Promise(resolve => { closeStarted = resolve })
        bridge.invoke = async (name, request) => {
            if (name !== 'ssh.close') { return invoke(name, request) }
            bridge.calls.push({ name, request })
            closeStarted()
            return closeWait
        }
        const session = create(makeProfile([]), bridge)
        await session.start()
        const connectionId = bridge.calls.find(call => call.name === 'ssh.connect').request.connectionId
        const output = bridge.listeners.get('ssh:output')
        const received = []
        session.releaseInitialDataBuffer()
        session.binaryOutput$.subscribe(data => received.push(Buffer.from(data)))
        output({ connectionId, data: Array.from(Buffer.from('before close')), extended: false })
        assert.equal(Buffer.concat(received).toString(), 'before close', 'live output reaches the terminal')
        const closing = session.destroy()
        try {
            await closeEntered
            assert.equal(session.isClosing, true)
            const data = Array.from(Buffer.alloc(4096, 0x61))
            for (let index = 0; index < 256; index++) {
                output({ connectionId, data, extended: index % 2 === 0 })
            }
            assert.equal(session.pendingOutput.length, 0, 'closing output must not accumulate while native close waits')
            assert.equal(Buffer.concat(received).toString(), 'before close', 'closing output must not reach the terminal')
        } finally {
            releaseClose({})
            await closing
        }
        assert.equal(bridge.listeners.size, 0, 'no listeners remain after cleanup')
        assert.equal(session.open, false)
    })

    await check('malformed-forwarding-entries', async () => {
        const failures = new Map()
        const result = await run(create, malformedForwards, failures)
        await guarded(result, failures, async () => {
            checkLive('malformed', result)
            assert.deepEqual(result.bridge.forwardStarts.map(value => value.bindPort), forwards.map(value => value.port), 'malformed: invalid entries skipped and valid later forwards attempted')
        })
    })

    // Native-session premises for the tab session-retention contract. These pass on the
    // current product: the session exposes its authenticated identity as soon as the
    // connection opens (before forwarding resolves), destroy does not deadlock on a gated
    // forwarding reply, and destroy leaves the last authenticated username/key flag intact.
    await check('auth-metadata-available-before-forwarding-resolves', async () => {
        const failures = new Map()
        const gate = defer()
        const bridge = makeBridge(failures, { forwardGate: gate.promise })
        const session = create(makeProfile([local]), bridge)
        const started = session.start()
        await settleTo(() => bridge.forwardStarts.length === 1, 'the forward request reaches the bridge')
        try {
            assert.equal(session.authUsername, 'alice', 'the authenticated username is available while the forward reply is pending')
            assert.equal(session.activePrivateKey, true, 'the private-key flag is available while the forward reply is pending')
            assert.equal(session.open, true, 'the session is marked open while the forward reply is pending')
        } finally {
            gate.resolve(true)
            await started
            await session.destroy()
            assert.equal(session.open, false, 'the session closes after teardown')
            assert.equal(bridge.calls.filter(call => call.name === 'ssh.close').length, 1, 'SSH is closed exactly once')
            assert.deepEqual(bridge.calls.filter(call => call.name === 'ssh.forwardingStop').map(call => call.request.id), [`fwd-${local.port}`], 'the started forward is stopped on teardown')
        }
    })

    await check('destroy-completes-before-gated-forward-reply', async () => {
        const failures = new Map()
        const gate = defer()
        const bridge = makeBridge(failures, { forwardGate: gate.promise })
        const session = create(makeProfile([local]), bridge)
        const started = session.start()
        await settleTo(() => bridge.forwardStarts.length === 1, 'the forward request reaches the bridge')
        let destroyed = false
        const destroying = session.destroy().then(() => { destroyed = true })
        try {
            await settleTo(() => destroyed, 'logical destruction before the pending forwarding reply')
            assert.equal(session.open, false, 'the session is closed without waiting for the reply')
        } finally {
            gate.resolve(true)
            await started
            await destroying
        }
        assert.equal(bridge.calls.filter(call => call.name === 'ssh.close').length, 1, 'SSH is closed exactly once')
        // The delayed mock reply returns an ID. Real native listener cleanup after that
        // race requires a separate wire test; this premise verifies logical destruction.
    })

    await check('destroy-preserves-auth-metadata', async () => {
        const failures = new Map()
        const bridge = makeBridge(failures)
        const session = create(makeProfile([]), bridge)
        await session.start()
        assert.equal(session.authUsername, 'alice', 'setup: the session authenticated as alice')
        assert.equal(session.activePrivateKey, true, 'setup: the session used a private key')
        await session.destroy()
        assert.equal(session.open, false, 'destroy closes the session')
        assert.equal(session.authUsername, 'alice', 'destroy preserves the authenticated username for a retained connection')
        assert.equal(session.activePrivateKey, true, 'destroy preserves the private-key flag for a retained connection')
        assert.equal(bridge.calls.filter(call => call.name === 'ssh.close').length, 1, 'SSH is closed exactly once')
        assert.equal(bridge.listeners.size, 0, 'no bridge listeners remain after teardown')
    })

    return results
}

function injectorFor () {
    return { get: () => ({ create: () => logger, store: { ssh: { x11Display: null, agentType: null, agentPath: null } }, isEnabled: () => false }) }
}
const modals = { open: () => ({ componentInstance: {}, result: Promise.resolve(null), dismiss () {} }) }

console.log('reference (verbatim upstream 14e2d60 addPortForward + ForwardedPort class):')
const reference = await runReference()
assert.deepEqual(reference, [], `reference control must pass:\n${reference.join('\n')}`)
console.log('current (tabby-tauri/src/ssh/session.ts):')
const current = await runCurrent()
assert.deepEqual(current, [], `forwarding startup must match upstream:\n${current.join('\n')}`)
console.log('ssh forwarding failure isolation and diagnostics match upstream')
