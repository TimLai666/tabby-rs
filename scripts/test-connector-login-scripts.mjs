import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'
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
            LogService: class {},
        }
        if (name === 'tabby-terminal') return Object.assign({}, ...[
            'session', 'api/middleware', 'middleware/inputProcessing',
            'middleware/streamProcessing',
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

function fixture (protocol, mode = 'normal', scripts = []) {
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
        reconnect: { enabled: false }, scripts,
    } }
    if (mode === 'legacy') delete profile.options.scripts
    const session = new Session({ get: () => ({ create: () => logger }) }, bridge, profile)
    return { session, writes, connecting, finishConnect, profile,
        output: (request, text) => callbacks.get(`${protocol}:output`)?.({ connectionId: request.connectionId, data: [...Buffer.from(text)] }),
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
