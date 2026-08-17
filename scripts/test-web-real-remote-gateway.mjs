import { spawn, spawnSync } from 'node:child_process'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'

function commandOutput (program, args) {
    const result = spawnSync(program, args, { encoding: 'utf8' })
    if (result.error) throw result.error
    if (result.status !== 0) {
        throw new Error(`${program} failed: ${result.stderr?.trim() || result.status}`)
    }
    return result.stdout.trim()
}

function ensureExecutable (program) {
    const result = spawnSync(program, [], { encoding: 'utf8' })
    if (result.error) throw result.error
}

function generateKey (pathName) {
    commandOutput('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', pathName])
}

async function listen (server) {
    await new Promise((resolve, reject) => {
        server.once('error', reject)
        server.listen(0, '127.0.0.1', resolve)
    })
    return server.address().port
}

function connectSocket (port) {
    const socket = net.createConnection({ host: '127.0.0.1', port })
    socket.setNoDelay(true)
    return socket
}

function sshArguments (fixture, extra = [], portFlag = '-p') {
    return [
        '-o', 'BatchMode=yes',
        '-o', 'StrictHostKeyChecking=no',
        '-o', 'UserKnownHostsFile=/dev/null',
        '-o', 'LogLevel=ERROR',
        '-i', fixture.privateKey,
        portFlag, String(fixture.sshPort),
        ...extra,
        `${fixture.username}@127.0.0.1`,
    ]
}

async function startOpenSsh () {
    if (process.platform === 'win32') {
        throw new Error('real Web remote fixture requires Unix OpenSSH')
    }

    commandOutput(process.env.TABBY_RS_SSHD || '/usr/sbin/sshd', ['-V'])
    ensureExecutable('ssh')
    ensureExecutable('sftp')

    const directory = await mkdtemp(path.join(os.tmpdir(), 'tabby-web-open-ssh-'))
    const hostKey = path.join(directory, 'host_key')
    const privateKey = path.join(directory, 'client_key')
    const username = commandOutput('id', ['-un'])
    const remoteDirectory = path.join(directory, 'remote')
    const { mkdir, copyFile } = await import('node:fs/promises')
    await mkdir(remoteDirectory, { recursive: true })
    await writeFile(path.join(remoteDirectory, 'fixture.txt'), 'real sftp fixture\n')
    generateKey(hostKey)
    generateKey(privateKey)
    const authorizedKeys = path.join(directory, 'authorized_keys')
    await copyFile(`${privateKey}.pub`, authorizedKeys)
    const portReservation = net.createServer()
    const sshPort = await listen(portReservation)
    await new Promise(resolve => portReservation.close(resolve))
    const config = path.join(directory, 'sshd_config')
    await writeFile(config, [
        `Port ${sshPort}`,
        'ListenAddress 127.0.0.1',
        `HostKey ${hostKey}`,
        `AuthorizedKeysFile ${authorizedKeys}`,
        'PubkeyAuthentication yes',
        'PasswordAuthentication no',
        'KbdInteractiveAuthentication no',
        'UsePAM no',
        'StrictModes no',
        'PermitRootLogin no',
        'Subsystem sftp internal-sftp',
        `AllowUsers ${username}`,
        `PidFile ${path.join(directory, 'sshd.pid')}`,
        'PrintMotd no',
        'LogLevel ERROR',
        '',
    ].join('\n'))

    const sshd = spawn(process.env.TABBY_RS_SSHD || '/usr/sbin/sshd', ['-D', '-e', '-f', config], {
        stdio: ['ignore', 'ignore', 'pipe'],
    })
    let sshdError = ''
    sshd.stderr.on('data', chunk => { sshdError += chunk.toString() })
    for (let attempt = 0; attempt < 100; attempt++) {
        const probe = net.createConnection({ host: '127.0.0.1', port: sshPort })
        const ready = await new Promise(resolve => {
            probe.once('connect', () => { probe.destroy(); resolve(true) })
            probe.once('error', () => resolve(false))
        })
        if (ready) break
        if (sshd.exitCode !== null) throw new Error(`sshd exited before ready: ${sshdError.trim()}`)
        await new Promise(resolve => setTimeout(resolve, 50))
        if (attempt === 99) throw new Error(`sshd did not become ready: ${sshdError.trim()}`)
    }

    const telnetServer = net.createServer(socket => {
        socket.write('real telnet fixture\r\n')
        socket.on('data', data => socket.write(Buffer.from(`telnet echo: ${data.toString()}`)))
    })
    const telnetPort = await listen(telnetServer)

    return {
        directory,
        privateKey,
        remoteDirectory,
        sshPort,
        telnetPort,
        username,
        sshd,
        telnetServer,
    }
}

async function runSftpList (fixture) {
    const child = spawn('sftp', [
        '-q', '-b', '-',
        ...sshArguments(fixture, [], '-P'),
    ], { stdio: ['pipe', 'pipe', 'pipe'] })
    child.stdin.end(`ls -1 ${fixture.remoteDirectory}\nbye\n`)
    const [stdout, stderr, status] = await Promise.all([
        collect(child.stdout),
        collect(child.stderr),
        new Promise(resolve => child.once('close', resolve)),
    ])
    if (status !== 0) throw new Error(`real SFTP list failed: ${stderr.trim()}`)
    const names = stdout.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
    return names.map(name => ({ name, path: '/', directory: false, size: name === 'fixture.txt' ? 18 : undefined }))
}

function collect (stream) {
    return new Promise(resolve => {
        let output = ''
        stream.on('data', chunk => { output += chunk.toString() })
        stream.once('end', () => resolve(output))
    })
}

export async function startRealWebGateway ({ token }) {
    const fixture = await startOpenSsh()
    const clients = new Set()

    function closeClient (client, state) {
        state.ssh?.kill()
        state.telnet?.destroy()
        clients.delete(client)
    }

    function handleConnection (client) {
        const state = { authenticated: false, connected: false, protocol: 'tcp', ssh: null, telnet: null }
        clients.add(client)
        client.send(JSON.stringify({ _: 'hello' }))
        client.on('message', async (message, isBinary) => {
            if (isBinary || !Buffer.from(message).toString().trimStart().startsWith('{')) {
                if (state.protocol === 'telnet') state.telnet?.write(message)
                else state.ssh?.stdin.write(message)
                return
            }
            const serviceMessage = JSON.parse(message.toString())
            if (serviceMessage._ === 'hello') {
                state.authenticated = serviceMessage.auth_token === token
                client.send(JSON.stringify(state.authenticated ? { _: 'ready' } : { _: 'error', details: 'invalid fixture token' }))
                return
            }
            if (serviceMessage._ === 'connect' && state.authenticated) {
                state.connected = true
                state.protocol = serviceMessage.protocol ?? 'tcp'
                client.send(JSON.stringify({ _: 'connected', protocol: state.protocol }))
                if (state.protocol === 'ssh') {
                    state.ssh = spawn('ssh', [...sshArguments(fixture, ['-tt']), "printf 'real OpenSSH gateway ready\\r\\n'; exec $SHELL -l"], {
                        stdio: ['pipe', 'pipe', 'pipe'],
                    })
                    state.ssh.stdout.on('data', data => client.send(data))
                    state.ssh.stderr.on('data', data => client.send(data))
                    state.ssh.on('close', () => client.close())
                } else if (state.protocol === 'telnet') {
                    state.telnet = connectSocket(fixture.telnetPort)
                    state.telnet.on('data', data => client.send(data))
                    state.telnet.on('close', () => client.close())
                }
                return
            }
            if (serviceMessage._ === 'provider-request' && state.authenticated && state.connected) {
                if (serviceMessage.operation === 'list' && serviceMessage.protocol === 'sftp') {
                    try {
                        const result = await runSftpList(fixture)
                        client.send(JSON.stringify({ _: 'response', id: serviceMessage.id, ok: true, result }))
                    } catch (error) {
                        client.send(JSON.stringify({ _: 'response', id: serviceMessage.id, ok: false, error: error instanceof Error ? error.message : String(error) }))
                    }
                } else if (serviceMessage.operation === 'resize') {
                    client.send(JSON.stringify({ _: 'response', id: serviceMessage.id, ok: true, result: null }))
                } else {
                    client.send(JSON.stringify({ _: 'response', id: serviceMessage.id, ok: true, result: null }))
                }
            }
        })
        client.on('close', () => closeClient(client, state))
    }

    return {
        handleConnection,
        async close () {
            for (const client of clients) client.close()
            fixture.telnetServer.close()
            fixture.sshd.kill()
            await rm(fixture.directory, { recursive: true, force: true })
        },
    }
}
