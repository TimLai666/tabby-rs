import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'tabby-web/webpack.config.mjs'))
const transpile = source => ts.transpileModule(source, {
    compilerOptions: {
        esModuleInterop: true,
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
    },
}).outputText

const source = fs.readFileSync(path.join(root, 'tabby-web/src/services/connectionGateway.service.ts'), 'utf8')
const providerSource = fs.readFileSync(path.join(root, 'tabby-web/src/services/webProvider.service.ts'), 'utf8')
const providerModule = { exports: {} }
vm.runInNewContext(transpile(providerSource), {
    exports: providerModule.exports,
    module: providerModule,
    require,
}, { filename: 'webProvider.service.ts' })

const module = { exports: {} }
vm.runInNewContext(transpile(source), {
    ArrayBuffer,
    Blob,
    Error,
    Uint8Array,
    WebSocket: undefined,
    console,
    exports: module.exports,
    module,
    require: name => name === './webProvider.service' ? providerModule.exports : require(name),
}, { filename: 'connectionGateway.service.ts' })

const { WebGatewayConnector } = module.exports
const sockets = []
class FakeWebSocket {
    readyState = 1
    onclose = null
    onerror = null
    onmessage = null
    onopen = null
    sent = []

    close () {
        this.readyState = 3
        this.onclose?.()
    }

    send (data) {
        this.sent.push(data)
    }

    receive (data) {
        this.onmessage?.({ data })
    }
}

const connector = new WebGatewayConnector({
    url: 'wss://gateway.example/provider',
    authToken: 'provider-token',
    webSocketFactory: () => {
        const socket = new FakeWebSocket()
        sockets.push(socket)
        return socket
    },
})

async function connect (session, options) {
    const connecting = session.connect(options)
    const socket = sockets.at(-1)
    socket.receive(JSON.stringify({ _: 'hello' }))
    socket.receive(JSON.stringify({ _: 'ready' }))
    const connectMessage = JSON.parse(socket.sent[1])
    socket.receive(JSON.stringify({ _: 'connected', protocol: connectMessage.protocol }))
    await connecting
    return socket
}

const ssh = connector.createSSHSession()
const sshSocket = await connect(ssh, { host: 'ssh.example.test', port: 22, username: 'alice' })
assert.equal(ssh.protocol, 'ssh')
assert.deepEqual(JSON.parse(sshSocket.sent[1]), {
    _: 'connect',
    host: 'ssh.example.test',
    port: 22,
    protocol: 'ssh',
    username: 'alice',
})
const resize = ssh.resize(120, 40)
const resizeRequest = JSON.parse(sshSocket.sent[2])
assert.deepEqual(resizeRequest, {
    _: 'provider-request',
    id: resizeRequest.id,
    protocol: 'ssh',
    operation: 'resize',
    columns: 120,
    rows: 40,
})
sshSocket.receive(JSON.stringify({ _: 'response', id: resizeRequest.id, ok: true, result: null }))
await resize

const sftp = ssh.openSFTP()
const sftpSocket = await connect(sftp, { host: 'sftp.example.test', port: 22, username: 'alice' })
assert.equal(sftp.protocol, 'sftp')
const listing = sftp.list('/')
const listRequest = JSON.parse(sftpSocket.sent[2])
assert.equal(listRequest.operation, 'list')
assert.equal(listRequest.protocol, 'sftp')
sftpSocket.receive(JSON.stringify({ _: 'response', id: listRequest.id, ok: true, result: [{ name: 'fixture.txt' }] }))
assert.deepEqual(JSON.parse(JSON.stringify(await listing)), [{ name: 'fixture.txt' }])

const telnet = connector.createTelnetSession()
const telnetSocket = await connect(telnet, { host: 'telnet.example.test', port: 23 })
assert.equal(telnet.protocol, 'telnet')
assert.equal(JSON.parse(telnetSocket.sent[1]).protocol, 'telnet')

ssh.close()
sftp.close()
telnet.close()
assert.equal(connector.sockets.size, 0)
console.log('Web SSH/SFTP/Telnet provider contract passed')
