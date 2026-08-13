import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'tabby-web/webpack.config.mjs'))
const source = fs.readFileSync(path.join(root, 'tabby-web/src/services/connectionGateway.service.ts'), 'utf8')
const javascriptSource = ts.transpileModule(source, {
    compilerOptions: {
        esModuleInterop: true,
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
    },
}).outputText

const module = { exports: {} }
const context = {
    ArrayBuffer,
    Blob,
    Error,
    Uint8Array,
    WebSocket: undefined,
    console,
    exports: module.exports,
    module,
    require,
}
vm.runInNewContext(javascriptSource, context, { filename: 'connectionGateway.service.ts' })

const { WebGatewayConnector } = module.exports
const sockets = []
const events = []
class FakeWebSocket {
    readyState = 0
    onclose = null
    onerror = null
    onmessage = null
    onopen = null
    sent = []

    constructor (url) {
        this.url = url
        sockets.push(this)
    }

    open () {
        this.readyState = 1
        this.onopen?.()
    }

    receive (data) {
        this.onmessage?.({ data })
    }

    fail (message) {
        this.onerror?.({ message })
    }

    close () {
        this.readyState = 3
        this.onclose?.()
    }

    send (data) {
        this.sent.push(data)
    }
}

const connector = new WebGatewayConnector({
    url: 'wss://gateway.example/socket',
    authToken: 'secret-token',
    webSocketFactory: url => new FakeWebSocket(url),
})
const socket = connector.createSocket()
socket.connect$.subscribe(() => events.push('connect'))
socket.data$.subscribe(data => events.push(`data:${Buffer.from(data).toString('hex')}`))
socket.error$.subscribe(error => events.push(`error:${error.message}`))
socket.close$.subscribe(() => events.push('close'))
socket.write(Uint8Array.from([1, 2]))
await socket.connect({ host: 'example.test', port: 22 })
const webSocket = sockets[0]
assert.equal(webSocket.url, 'wss://gateway.example/socket')
webSocket.open()
webSocket.receive(JSON.stringify({ _: 'hello' }))
webSocket.receive(JSON.stringify({ _: 'ready' }))
webSocket.receive(JSON.stringify({ _: 'connected' }))
webSocket.receive(Uint8Array.from([3, 4]))
await new Promise(resolve => setTimeout(resolve, 0))
socket.write(Uint8Array.from([5, 6]))

assert.deepEqual(webSocket.sent.slice(0, 2).map(x => JSON.parse(x)), [
    { _: 'hello', version: 1, auth_token: 'secret-token' },
    { _: 'connect', host: 'example.test', port: 22 },
])
assert.deepEqual([...webSocket.sent[2]], [1, 2])
assert.deepEqual([...webSocket.sent[3]], [5, 6])
assert.deepEqual(events, ['connect', 'data:0304'])
assert.equal(connector.sockets.size, 1)

webSocket.receive(JSON.stringify({ _: 'error', details: 'gateway rejected' }))
assert.deepEqual(events, ['connect', 'data:0304', 'error:gateway rejected', 'close'])
assert.equal(connector.sockets.size, 0)

console.log('Web gateway protocol contract passed')
