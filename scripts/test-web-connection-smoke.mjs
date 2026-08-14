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

const gatewaySource = fs.readFileSync(path.join(root, 'tabby-web/src/services/connectionGateway.service.ts'), 'utf8')
const bufferSource = fs.readFileSync(path.join(root, 'web/polyfills.buffer.ts'), 'utf8')
const polyfillsSource = fs.readFileSync(path.join(root, 'web/polyfills.ts'), 'utf8')
const sshSource = fs.readFileSync(path.join(root, 'tabby-ssh/src/session/ssh.ts'), 'utf8')
const sftpSource = fs.readFileSync(path.join(root, 'tabby-ssh/src/session/sftp.ts'), 'utf8')
const telnetSource = fs.readFileSync(path.join(root, 'tabby-telnet/src/session.ts'), 'utf8')

assert.match(sshSource, /from 'net'/, 'web SSH must retain the shared socket transport seam')
assert.match(sshSource, /openSFTP/, 'SFTP must remain attached to the SSH session transport')
assert.match(sftpSource, /constructor \(private sftp:/, 'SFTP must consume the SSH SFTP session')
assert.match(telnetSource, /from 'net'/, 'web Telnet must retain the shared socket transport seam')

const gatewayModule = { exports: {} }
const gatewayContext = {
    ArrayBuffer,
    Error,
    exports: gatewayModule.exports,
    JSON,
    module: gatewayModule,
    Promise,
    require: name => name === 'rxjs' ? require('rxjs') : require(name),
    Set,
    Uint8Array,
}
vm.runInNewContext(transpile(gatewaySource), gatewayContext, {
    filename: 'connectionGateway.service.cjs',
})

const { WebGatewayConnector } = gatewayModule.exports

class FakeWebSocket {
    readyState = 1
    sent = []
    onclose = null
    onerror = null
    onmessage = null
    onopen = null

    send (data) {
        this.sent.push(data)
    }

    close () {
        if (this.readyState === 3) {
            return
        }
        this.readyState = 3
        this.onclose?.()
    }

    receive (data) {
        this.onmessage?.({ data })
    }
}

const fakeSockets = []
const connector = new WebGatewayConnector({
    url: 'wss://gateway.example.test/socket',
    authToken: 'web-fixture-token',
    webSocketFactory: () => {
        const socket = new FakeWebSocket()
        fakeSockets.push(socket)
        return socket
    },
})

const registeredModules = new Map()
const moduleCache = new Map()
const window = {
    Buffer,
    Tabby: {
        registerMock: (name, value) => registeredModules.set(name, value),
        registerModule: (name, value) => registeredModules.set(name, value),
    },
    __connector__: connector,
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
    if (name === 'stream-browserify' || name === 'base64-js' || name === 'events') return require(name)
    return {}
}

const polyfillsModule = { exports: {} }
const polyfillsContext = {
    Buffer,
    clearTimeout,
    console,
    exports: polyfillsModule.exports,
    module: polyfillsModule,
    process: { addListener () {} },
    require: load,
    setTimeout,
    window,
}
vm.runInNewContext(transpile(polyfillsSource), polyfillsContext, {
    filename: 'web/polyfills.ts',
})

const { SocketProxy } = polyfillsModule.exports
const tick = () => new Promise(resolve => setImmediate(resolve))

async function runConnectionSmoke (name, connectArgs) {
    const proxy = new SocketProxy()
    const events = []
    const received = []
    proxy.on('connect', () => events.push('connect'))
    proxy.on('data', data => received.push(Buffer.from(data).toString('hex')))
    proxy.on('error', error => events.push(`error:${error.message}`))
    proxy.on('close', () => events.push('close'))

    proxy.connect(...connectArgs)
    const fakeSocket = fakeSockets[fakeSockets.length - 1]
    assert.ok(fakeSocket, `${name} must create a WebSocket through the web connector`)
    fakeSocket.receive(JSON.stringify({ _: 'hello' }))
    fakeSocket.receive(JSON.stringify({ _: 'ready' }))
    fakeSocket.receive(JSON.stringify({ _: 'connected' }))
    await tick()

    assert.deepEqual(fakeSocket.sent.slice(0, 2).map(JSON.parse), [
        { _: 'hello', version: 1, auth_token: 'web-fixture-token' },
        { _: 'connect', host: connectArgs[0]?.host ?? connectArgs[1], port: connectArgs[0]?.port ?? connectArgs[0] },
    ], `${name} gateway handshake must be routed through the web connector`)
    assert.deepEqual(events, ['connect'], `${name} must emit connect after the gateway accepts the route`)

    proxy.write(Buffer.from(`${name}-request`))
    assert.equal(Buffer.from(fakeSocket.sent[2]).toString(), `${name}-request`, `${name} writes must use the gateway socket`)

    fakeSocket.receive(Uint8Array.from([0x01, 0x02, 0x03]))
    await tick()
    assert.deepEqual(received, ['010203'], `${name} data must reach the web session`)

    fakeSocket.receive(JSON.stringify({ _: 'error', details: `${name} gateway failure` }))
    await tick()
    assert.deepEqual(events, ['connect', `error:${name} gateway failure`, 'close'], `${name} errors must close the web session`)
}

await runConnectionSmoke('ssh', [{ host: 'ssh.example.test', port: 22 }])
await runConnectionSmoke('sftp', [{ host: 'sftp.example.test', port: 22 }])
await runConnectionSmoke('telnet', [23, 'telnet.example.test'])

assert.equal(connector.sockets.size, 0, 'closed web sockets must be removed from the connector')
assert.equal(registeredModules.get('net').Socket, SocketProxy, 'web net provider must use the gateway socket proxy')

console.log('Web SSH/SFTP/Telnet connection smoke passed')
