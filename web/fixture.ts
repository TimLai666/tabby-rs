import { WebGatewayConnector, type WebHostConnector } from '../tabby-web/src/services/connectionGateway.service'
import type { WebSSHSession, WebSFTPSession, WebTelnetSession } from '../tabby-web/src/services/webProvider.service'
import type { BootstrapData } from '../tabby-core/src/api/mainProcess'

interface FixtureBootstrapOptions {
    packageModules: any[]
    bootstrapData: BootstrapData
    debugMode: boolean
    connector: WebHostConnector
}

interface FixtureTabby {
    loadPlugin: (url: string) => Promise<any>
}

const tabbyWindow = window as Window & {
    bootstrapTabby?: (options: FixtureBootstrapOptions) => Promise<unknown>
    Tabby?: FixtureTabby
}

const fixtureToken = 'tabby-rs-web-fixture-token'
const output = document.querySelector<HTMLPreElement>('#fixture-output')!
const status = document.querySelector<HTMLElement>('#fixture-status')!
const tokenInput = document.querySelector<HTMLInputElement>('#fixture-token')!
const hostInput = document.querySelector<HTMLInputElement>('#fixture-host')!
const portInput = document.querySelector<HTMLInputElement>('#fixture-port')!
const terminalInput = document.querySelector<HTMLInputElement>('#fixture-terminal-input')!
const resizeButton = document.querySelector<HTMLButtonElement>('#fixture-resize')!
const sftpButton = document.querySelector<HTMLButtonElement>('#fixture-sftp-list')!
const telnetButton = document.querySelector<HTMLButtonElement>('#fixture-telnet-connect')!
const settingsInput = document.querySelector<HTMLTextAreaElement>('#fixture-settings')!

let connector: WebGatewayConnector|null = null
let sshSession: WebSSHSession|null = null
let sftpSession: WebSFTPSession|null = null
let telnetSession: WebTelnetSession|null = null

function setStatus (message: string): void {
    status.textContent = message
}

function log (message: string): void {
    output.textContent += `${message}\n`
    output.scrollTop = output.scrollHeight
}

function connectSocket (): void {
    if (!connector) {
        throw new Error('Sign in to the web host before connecting')
    }
    sshSession?.close()
    sshSession = connector.createSSHSession()
    sshSession.connect$.subscribe(() => {
        setStatus('connected')
        log('web SSH provider connected')
    })
    sshSession.data$.subscribe(data => {
        const text = new TextDecoder().decode(data)
        log(`received: ${JSON.stringify(text)}`)
    })
    sshSession.error$.subscribe(error => {
        setStatus(`error: ${error.message}`)
        log(`gateway error: ${error.message}`)
    })
    sshSession.close$.subscribe(() => log('web SSH provider closed'))
    void sshSession.connect({
        host: hostInput.value || 'fixture.example.test',
        port: Number(portInput.value) || 22,
        username: 'fixture-user',
    })
}

document.querySelector<HTMLFormElement>('#fixture-login')!.addEventListener('submit', event => {
    event.preventDefault()
    const token = tokenInput.value
    if (!token) {
        setStatus('token required')
        return
    }
    connector = new WebGatewayConnector({
        url: `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.host}/gateway`,
        authToken: token,
    })
    setStatus(token === fixtureToken ? 'signed in' : 'invalid token')
    log(token === fixtureToken ? 'host login accepted' : 'host login rejected')
})

document.querySelector<HTMLFormElement>('#fixture-connect')!.addEventListener('submit', event => {
    event.preventDefault()
    try {
        connectSocket()
    } catch (error) {
        setStatus(error instanceof Error ? error.message : 'connect failed')
    }
})

terminalInput.addEventListener('keydown', event => {
    if (event.key !== 'Enter') {
        return
    }
    event.preventDefault()
    if (!sshSession) {
        setStatus('connect before sending terminal input')
        return
    }
    const value = `${terminalInput.value}\r`
    sshSession.write(new TextEncoder().encode(value))
    log(`sent: ${JSON.stringify(value)}`)
    terminalInput.value = ''
})

resizeButton.addEventListener('click', () => {
    const columns = Math.max(1, Math.floor(window.innerWidth / 8))
    const rows = Math.max(1, Math.floor(window.innerHeight / 16))
    document.body.dataset.viewport = `${columns}x${rows}`
    if (!sshSession) {
        setStatus('connect before resizing')
        return
    }
    void sshSession.resize(columns, rows).then(() => {
        log(`viewport resize observed: ${columns}x${rows}`)
    }).catch(error => log(`viewport resize failed: ${error instanceof Error ? error.message : String(error)}`))
})

sftpButton.addEventListener('click', () => {
    if (!connector) {
        setStatus('connect before requesting SFTP list')
        return
    }
    sftpSession?.close()
    sftpSession = connector.createSFTPSession()
    void sftpSession.connect({
        host: hostInput.value || 'fixture.example.test',
        port: Number(portInput.value) || 22,
        username: 'fixture-user',
    }).then(async () => {
        const entries = await sftpSession!.list('/')
        log(`sftp list response: ${JSON.stringify(entries)}`)
    }).catch(error => log(`sftp list failed: ${error instanceof Error ? error.message : String(error)}`))
})

telnetButton.addEventListener('click', () => {
    if (!connector) {
        setStatus('sign in before connecting Telnet')
        return
    }
    telnetSession?.close()
    telnetSession = connector.createTelnetSession()
    telnetSession.connect$.subscribe(() => log('web Telnet provider connected'))
    void telnetSession.connect({
        host: hostInput.value || 'fixture.example.test',
        port: 23,
    }).catch(error => log(`Telnet provider failed: ${error instanceof Error ? error.message : String(error)}`))
})

document.querySelector<HTMLButtonElement>('#fixture-save-settings')!.addEventListener('click', () => {
    const content = settingsInput.value
    window.localStorage.setItem('tabby-rs-web-fixture-settings', content)
    log(`settings saved: ${JSON.stringify(content)}`)
})

document.querySelector<HTMLButtonElement>('#fixture-load-settings')!.addEventListener('click', () => {
    settingsInput.value = window.localStorage.getItem('tabby-rs-web-fixture-settings') ?? '{}'
    log(`settings loaded: ${JSON.stringify(settingsInput.value)}`)
})

document.querySelector<HTMLButtonElement>('#fixture-boot-shared-ui')!.addEventListener('click', async () => {
    if (!connector) {
        setStatus('sign in before booting shared UI')
        return
    }
    if (!tabbyWindow.bootstrapTabby) {
        setStatus('shared UI bundle is unavailable')
        return
    }
    if (!tabbyWindow.Tabby) {
        setStatus('shared plugin loader is unavailable')
        return
    }
    try {
        const hostConnector: WebHostConnector = {
            createSocket: (..._args: unknown[]) => connector!.createSocket(),
            loadConfig: async () => window.localStorage.getItem('tabby-rs-web-fixture-settings') ?? '{}',
            saveConfig: async content => window.localStorage.setItem('tabby-rs-web-fixture-settings', content),
            getAppVersion: () => 'web-fixture',
        }
        const packageNames = ['tabby-core', 'tabby-settings', 'tabby-terminal', 'tabby-web']
        const packageModules = []
        for (const packageName of packageNames) {
            packageModules.push(await tabbyWindow.Tabby.loadPlugin(`/plugins/${packageName}`))
        }
        log(`shared plugins loaded: ${packageNames.join(', ')}`)
        await tabbyWindow.bootstrapTabby({
            packageModules,
            bootstrapData: {
                config: {},
                executable: 'tabby-rs-web-fixture',
                installedPlugins: [],
                isMainWindow: true,
                userPluginsPath: '',
                windowID: 1,
            },
            debugMode: true,
            connector: hostConnector,
        })
        log('shared Tabby UI bootstrapped')
    } catch (error) {
        setStatus(error instanceof Error ? error.message : 'shared UI bootstrap failed')
        log(`shared UI bootstrap failed: ${error instanceof Error ? error.message : String(error)}`)
    }
})

tokenInput.value = fixtureToken
settingsInput.value = '{}'
setStatus('signed out')
