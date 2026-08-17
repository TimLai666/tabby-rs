import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ws from 'ws'

const { Server: WebSocketServer } = ws

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const dist = path.join(root, 'web/dist')
const token = 'tabby-rs-web-fixture-token'
const requestedPort = Number(process.env.TABBY_WEB_FIXTURE_PORT ?? 0)

const contentTypes = {
    '.html': 'text/html; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.map': 'application/json; charset=utf-8',
}

const pluginAssets = new Map([
    ['/plugins/tabby-core/package.json', path.join(root, 'tabby-core/package.json')],
    ['/plugins/tabby-core/dist/index.js', path.join(root, 'tabby-core/dist/index.js')],
    ['/plugins/tabby-terminal/package.json', path.join(root, 'tabby-terminal/package.json')],
    ['/plugins/tabby-terminal/dist/index.js', path.join(root, 'tabby-terminal/dist/index.js')],
    ['/plugins/tabby-settings/package.json', path.join(root, 'tabby-settings/package.json')],
    ['/plugins/tabby-settings/dist/index.js', path.join(root, 'tabby-settings/dist/index.js')],
    ['/plugins/tabby-web/package.json', path.join(root, 'tabby-web/package.json')],
    ['/plugins/tabby-web/dist/index.js', path.join(root, 'tabby-web/dist/index.js')],
])

function serve (request, response) {
    const requestPath = request.url === '/' ? '/fixture.html' : request.url
    const filePath = requestPath === '/fixture.html'
        ? path.join(root, 'web', requestPath.slice(1))
        : pluginAssets.get(requestPath) ?? path.join(dist, requestPath.slice(1))
    if (!filePath.startsWith(root) || !fs.existsSync(filePath)) {
        response.writeHead(404)
        response.end('not found')
        return
    }
    response.writeHead(200, { 'content-type': contentTypes[path.extname(filePath)] ?? 'application/octet-stream' })
    fs.createReadStream(filePath).pipe(response)
}

const server = http.createServer(serve)
const gateway = new WebSocketServer({ noServer: true })
const events = []

server.on('upgrade', (request, socket, head) => {
    if (request.url !== '/gateway') {
        socket.destroy()
        return
    }
    gateway.handleUpgrade(request, socket, head, client => gateway.emit('connection', client, request))
})

gateway.on('connection', client => {
    let authenticated = false
    let connected = false
    let protocol = 'tcp'
    client.send(JSON.stringify({ _: 'hello' }))
    events.push('hello')
    client.on('message', (message, isBinary) => {
        const text = Buffer.from(message).toString()
        if (isBinary || !text.trimStart().startsWith('{')) {
            events.push(`data:${text}`)
            client.send(Buffer.from(`fixture echo: ${text}`))
            return
        }
        const serviceMessage = JSON.parse(message.toString())
        if (serviceMessage._ === 'hello') {
            authenticated = serviceMessage.auth_token === token
            events.push(authenticated ? 'authenticated' : 'authentication-failed')
            client.send(JSON.stringify(authenticated ? { _: 'ready' } : { _: 'error', details: 'invalid fixture token' }))
        } else if (serviceMessage._ === 'connect' && authenticated) {
            connected = true
            protocol = serviceMessage.protocol ?? 'tcp'
            events.push(`connect:${protocol}:${serviceMessage.host}:${serviceMessage.port}`)
            client.send(JSON.stringify({ _: 'connected', protocol }))
            if (protocol !== 'sftp') client.send(Buffer.from('fixture gateway ready\r\n'))
        } else if (serviceMessage._ === 'provider-request' && authenticated && connected) {
            events.push(`provider:${serviceMessage.protocol}:${serviceMessage.operation}`)
            if (serviceMessage.operation === 'list' && serviceMessage.protocol === 'sftp') {
                client.send(JSON.stringify({
                    _: 'response',
                    id: serviceMessage.id,
                    ok: true,
                    result: [{ name: 'fixture.txt', path: '/', directory: false, size: 16 }],
                }))
            } else if (serviceMessage.operation === 'resize') {
                client.send(JSON.stringify({ _: 'response', id: serviceMessage.id, ok: true, result: null }))
            } else {
                client.send(JSON.stringify({ _: 'response', id: serviceMessage.id, ok: true, result: null }))
            }
        }
    })
    client.on('close', () => events.push(connected ? 'closed-connected' : 'closed'))
})

server.listen(requestedPort, '127.0.0.1', () => {
    const address = server.address()
    const port = typeof address === 'object' && address ? address.port : requestedPort
    console.log(`Web browser fixture: http://127.0.0.1:${port}/`)
    console.log(`Expected token: ${token}`)
    console.log('Use the page to sign in, connect, send terminal input, record resize, request SFTP list, save/load settings, and boot shared UI.')
})

function shutdown () {
    for (const client of gateway.clients) client.close()
    gateway.close()
    server.close(() => process.exit(0))
}

process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
