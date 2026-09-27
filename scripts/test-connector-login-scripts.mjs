import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'
import { setImmediate } from 'node:timers/promises'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const cache = new Map()
const logger = { info () {}, debug () {}, warn () {}, error () {} }

// Run the real session, middleware stack, and script processor. Only UI and
// native transport dependencies are replaced; assertions observe bridge writes.
function load (file) {
    file = path.resolve(root, file)
    if (cache.has(file)) return cache.get(file).exports
    const module = { exports: {} }
    cache.set(file, module)
    const require = name => {
        if (name === '@angular/core') return { Injector: class {} }
        if (name === 'tabby-core') return {
            ...load('tabby-core/src/components/base.component.ts'),
            ...load('tabby-core/src/utfSplitter.ts'),
            LogService: class {},
        }
        if (name === 'tabby-terminal') return Object.assign({}, ...[
            'session', 'api/middleware', 'middleware/inputProcessing',
            'middleware/streamProcessing', 'middleware/utf8Splitter',
        ].map(part => load(`tabby-terminal/src/${part}.ts`)))
        if (name === '../api/hostBridge') return {}
        if (name.startsWith('.')) return load(path.resolve(path.dirname(file), `${name}.ts`))
        return createRequire(file)(name)
    }
    const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, esModuleInterop: true },
    }).outputText
    vm.runInNewContext(code, {
        module, exports: module.exports, require, Buffer, TextDecoder, console,
        setTimeout, clearTimeout, window: { crypto: { randomUUID } },
    }, { filename: file })
    return module.exports
}

function fixture (protocol, mode = 'normal', scripts = [], options = {}) {
    const Session = load(`tabby-tauri/src/${protocol}/session.ts`)[protocol === 'telnet' ? 'TauriTelnetSession' : 'TauriSerialSession']
    const callbacks = new Map()
    const writes = []
    let finishConnect
    let started
    const connecting = new Promise(resolve => { started = resolve })
    const opened = new Promise(resolve => { finishConnect = resolve })
    const bridge = {
        listen: async (name, callback) => { callbacks.set(name, callback); return () => callbacks.delete(name) },
        invoke: async (name, request) => {
            if (name === `${protocol}.write`) writes.push(Buffer.from(request.data).toString())
            if (name === `${protocol}.${protocol === 'telnet' ? 'connect' : 'open'}`) {
                started(request)
                if (mode === 'failure') throw new Error('fixture refused')
                await opened
                return { id: 'fixture' }
            }
            return {}
        },
    }
    const profile = { id: 'test', options: {
        host: 'localhost', port: protocol === 'telnet' ? 23 : '/fixture', input: {},
        inputMode: null, outputMode: null, inputNewlines: null, outputNewlines: null,
        reconnect: { enabled: false }, scripts, ...options,
    } }
    if (mode === 'legacy') delete profile.options.scripts
    const session = new Session({ get: () => ({ create: () => logger }) }, bridge, profile)
    return { session, writes, connecting, finishConnect, profile, bridge,
        output: (request, text) => callbacks.get(`${protocol}:output`)?.({ connectionId: request.connectionId, data: [...Buffer.from(text)] }),
        state: (request, state) => callbacks.get('serial:connectionState')?.({ connectionId: request.connectionId, state }),
    }
}

for (const protocol of ['telnet', 'serial']) {
    for (const early of [false, true]) {
        const scripts = [{ expect: '', send: 'BEGIN' }, { expect: 'login:', send: 'fixture-user' }]
        const original = JSON.stringify(scripts)
        const f = fixture(protocol, 'normal', scripts)
        try {
            const start = f.session.start()
            const request = await f.connecting
            assert.deepEqual(f.writes, [], 'no scripts before connection')
            if (early) f.output(request, 'login:')
            f.finishConnect()
            await start
            if (!early) {
                assert.deepEqual(f.writes, ['BEGIN\n'], `${protocol}: send initial script`)
                f.output(request, 'login:')
            }
            await setImmediate()
            assert.deepEqual(f.writes, ['BEGIN\n', 'fixture-user\n'], `${protocol}: ordered scripts, early=${early}`)
            f.output(request, 'login:')
            await f.session.start()
            assert.equal(f.writes.length, 2, 'scripts run once per session')
            assert.equal(JSON.stringify(scripts), original, 'profile scripts survive for reconnect')
        } finally { await f.session.destroy() }
    }
    for (const mode of ['failure', 'cancelled', 'legacy']) {
        const f = fixture(protocol, mode, [{ expect: '', send: 'MUST_NOT_SEND' }])
        try {
            const start = f.session.start()
            await f.connecting
            if (mode === 'failure') await assert.rejects(start, /fixture refused/)
            else {
                if (mode === 'cancelled') await f.session.destroy()
                f.finishConnect()
                await start
            }
            assert.deepEqual(f.writes, [], `${protocol}: ${mode} does not send scripts`)
        } finally { await f.session.destroy() }
    }
    console.log(`${protocol}: initial, prompt, early output, once-only, failure, cancellation, legacy passed`)
}

for (const options of [{}, { slowSend: false }, { slowSend: true }]) {
    const f = fixture('serial', 'normal', [{ expect: '', send: 'BEGIN' }], options)
    try {
        const started = f.session.start()
        const request = await f.connecting
        assert.equal(request.slowSend, undefined, 'slow feed is processed before the native bridge')
        f.finishConnect()
        await started
        await setImmediate()
        assert.deepEqual(f.writes, options.slowSend ? ['B', 'EGIN\n'] : ['BEGIN\n'], 'login scripts use the upstream stream batching')
        f.writes.length = 0
        f.session.feedFromTerminal(Buffer.from('AB'))
        f.session.feedFromTerminal(Buffer.from('CD'))
        await setImmediate()
        assert.deepEqual(f.writes, options.slowSend ? ['A', 'BCD'] : ['AB', 'CD'], 'consecutive inputs share the same ordered queue')
    } finally { await f.session.destroy() }
}
console.log('serial: slow feed and legacy settings control upstream batching for scripts and consecutive input')

for (const protocol of ['serial', 'telnet']) {
    const bytes = Buffer.from('台🙂éa')
    for (let split = 1; split < bytes.length; split++) {
        const f = fixture(protocol)
        try {
            const start = f.session.start()
            const request = await f.connecting
            f.finishConnect()
            await start
            f.session.releaseInitialDataBuffer()
            const output = []
            f.session.output$.subscribe(text => output.push(text))
            f.output(request, bytes.subarray(0, split))
            f.output(request, bytes.subarray(split))
            assert.equal(output.join(''), '台🙂éa', `${protocol}: UTF-8 split at byte ${split}`)
        } finally { await f.session.destroy() }
    }
    console.log(`${protocol}: split UTF-8 output preserves complete characters`)
}

{
    const f = fixture('serial')
    const start = f.session.start()
    const request = await f.connecting
    f.finishConnect()
    await start
    f.session.releaseInitialDataBuffer()
    const output = []
    f.session.output$.subscribe(text => output.push(text))
    f.output(request, Buffer.from([0xe5, 0x8f]))
    assert.equal(output.join(''), '', 'retain an incomplete character while connected')
    await f.session.destroy()
    assert.equal(output.join(''), '\ufffd', 'flush an incomplete character when closing, like upstream')
}
console.log('serial: incomplete UTF-8 output is flushed on close')

for (const lateError of [false, true]) {
    const f = fixture('serial', 'normal', [], { reconnect: { enabled: true }, slowSend: true })
    try {
        const start = f.session.start()
        const request = await f.connecting
        f.finishConnect()
        await start
        const invoke = f.bridge.invoke
        let rejectWrite
        f.bridge.invoke = async (name, value) => {
            const result = await invoke(name, value)
            if (name === 'serial.write') {
                if (lateError) await new Promise((_, reject) => { rejectWrite = reject })
                else throw { code: 'io', details: 'Serial port is disconnected' }
            }
            return result
        }
        f.session.feedFromTerminal(Buffer.from('AB'))
        await setImmediate()
        assert.equal(f.session.open, true, 'write failure must preserve automatic reconnect')
        f.state(request, 'disconnected')
        f.state(request, 'waiting')
        const count = f.writes.length
        f.session.feedFromTerminal(Buffer.from('DROP'))
        assert.equal(f.writes.length, count, 'do not send input to a disconnected port')
        f.state(request, 'connected')
        f.bridge.invoke = invoke
        if (lateError) rejectWrite(new Error('old connection failed'))
        await setImmediate()
        f.session.feedFromTerminal(Buffer.from('CD'))
        await setImmediate()
        assert.deepEqual(f.writes, ['A', 'C', 'D'], 'reconnect uses a new queue and discards stale queued input')
        assert.equal(f.session.open, true)
    } finally { await f.session.destroy() }
}
console.log('serial: automatic reconnect survives failed and stale writes')

{
    const f = fixture('serial', 'normal', [], { reconnect: { enabled: true } })
    await assert.rejects(() => f.session.setBaudRate(9600), /not open/)
    const start = f.session.start()
    const request = await f.connecting
    f.finishConnect()
    await start
    const calls = []
    const invoke = f.bridge.invoke
    f.bridge.invoke = async (name, value) => {
        if (name === 'serial.setBaudRate') calls.push(value)
        return invoke(name, value)
    }
    try {
        await f.session.setBaudRate(9600)
        assert.equal(calls.length, 1)
        assert.equal(calls[0].id, 'fixture')
        assert.equal(calls[0].baudRate, 9600)
        f.bridge.invoke = async () => { throw new Error('driver rejected rate') }
        await assert.rejects(() => f.session.setBaudRate(19200), /driver rejected/)
        f.bridge.invoke = invoke
        f.state(request, 'disconnected')
        await f.session.setBaudRate(19200)
        assert.equal(f.session.open, true, 'changing the next reconnect rate must preserve the session')
        f.state(request, 'connected')
        await f.session.setBaudRate(38400)
    } finally {
        f.bridge.invoke = invoke
        await f.session.destroy()
    }
    await assert.rejects(() => f.session.setBaudRate(9600), /not open/)
}
console.log('serial: live baud updates, failed changes, reconnect-wait updates, and closed guards passed')
