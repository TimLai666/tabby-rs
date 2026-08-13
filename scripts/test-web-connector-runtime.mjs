import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'web/webpack.config.mjs'))
const transpile = source => ts.transpileModule(source, {
    compilerOptions: {
        esModuleInterop: true,
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
    },
}).outputText

const source = fs.readFileSync(path.join(root, 'web/polyfills.ts'), 'utf8')
const bufferSource = fs.readFileSync(path.join(root, 'web/polyfills.buffer.ts'), 'utf8')
const sshSource = fs.readFileSync(path.join(root, 'tabby-ssh/src/session/ssh.ts'), 'utf8')
const sftpSource = fs.readFileSync(path.join(root, 'tabby-ssh/src/session/sftp.ts'), 'utf8')
const telnetSource = fs.readFileSync(path.join(root, 'tabby-telnet/src/session.ts'), 'utf8')

assert.match(source, /Socket: SocketProxy/, 'web net provider must use the gateway socket proxy')
assert.match(sshSource, /from 'net'/, 'web SSH provider must retain the shared net transport seam')
assert.match(sshSource, /openSFTP/, 'SFTP must remain attached to the SSH provider transport')
assert.match(sftpSource, /constructor \(private sftp:/, 'SFTP provider must consume the SSH SFTP session')
assert.match(telnetSource, /from 'net'/, 'web Telnet provider must retain the shared net transport seam')

const registeredModules = new Map()
const moduleCache = new Map()
const calls = {
    close: [],
    connect: [],
    createSocket: [],
    write: [],
}
const events = []
const socket = {
    connect$: {
        subscribe: listener => {
            socket.connectListener = listener
            return { unsubscribe () {} }
        },
    },
    data$: {
        subscribe: listener => {
            socket.dataListener = listener
            return { unsubscribe () {} }
        },
    },
    error$: {
        subscribe: listener => {
            socket.errorListener = listener
            return { unsubscribe () {} }
        },
    },
    close$: {
        subscribe: listener => {
            socket.closeListener = listener
            return { unsubscribe () {} }
        },
    },
    connect: (...args) => calls.connect.push(args),
    write: chunk => calls.write.push(Buffer.from(chunk).toString()),
    close: error => calls.close.push(error),
}
const window = {
    Buffer,
    Tabby: {
        registerMock: (name, value) => registeredModules.set(name, value),
        registerModule: (name, value) => registeredModules.set(name, value),
    },
    __connector__: {
        createSocket: (...args) => {
            calls.createSocket.push(args)
            return socket
        },
    },
    require,
}

const load = name => {
    if (name === './polyfills.buffer') {
        if (!moduleCache.has(name)) {
            const module = { exports: {} }
            const context = { Buffer, console, exports: module.exports, module, require: load, window }
            vm.runInNewContext(transpile(bufferSource), context, { filename: 'web/polyfills.buffer.ts' })
            moduleCache.set(name, module.exports)
        }
        return moduleCache.get(name)
    }
    if (name === 'buffer') return { Buffer }
    if (name === 'util/') return require('util')
    if (name === 'stream-browserify' || name === 'base64-js' || name === 'events') {
        return require(name)
    }
    return {}
}

const module = { exports: {} }
const context = {
    Buffer,
    clearTimeout,
    console,
    exports: module.exports,
    module,
    process: { addListener () {} },
    require: load,
    setTimeout,
    window,
}
vm.runInNewContext(transpile(source), context, { filename: 'web/polyfills.ts' })

const proxy = new context.module.exports.SocketProxy('example.test', 22)
proxy.on('connect', () => events.push('connect'))
proxy.on('data', data => events.push(`data:${data.toString('hex')}`))
proxy.on('error', error => events.push(`error:${error.message}`))
proxy.on('close', () => events.push('close'))

socket.connectListener()
socket.dataListener(Uint8Array.from([1, 2]))
socket.errorListener(new Error('gateway failed'))
socket.closeListener()
proxy.connect('retry')
proxy.write(Buffer.from('hello'))
proxy.destroy()

assert.deepEqual(calls.createSocket, [['example.test', 22]])
assert.deepEqual(calls.connect, [['retry']])
assert.deepEqual(calls.write, ['hello'])
assert.equal(calls.close.length, 1)
assert.deepEqual(events, ['connect', 'data:0102', 'error:gateway failed', 'close'])
assert.equal(registeredModules.get('net').Socket, context.module.exports.SocketProxy)

const providerRoutes = [
    ['ssh', ['ssh.example', 22]],
    ['sftp-over-ssh', ['sftp.example', 22]],
    ['telnet', [23, 'telnet.example']],
]
for (const [, args] of providerRoutes) {
    const providerProxy = new context.module.exports.SocketProxy()
    providerProxy.connect(...args)
}
assert.deepEqual(calls.connect.slice(1), providerRoutes.map(([, args]) => args))

console.log('Web connector provider routing contract passed')
