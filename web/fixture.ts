import { WebGatewayConnector, WebGatewaySocket } from '../tabby-web/src/services/connectionGateway.service'
import type { WebHostConnector } from '../tabby-web/src/services/connectionGateway.service'
import type { BootstrapData } from '../tabby-core/src/api/mainProcess'

interface FixtureBootstrapOptions {
    packageModules: any[]
    bootstrapData: BootstrapData
    debugMode: boolean
    connector: WebHostConnector
}

const tabbyWindow = window as Window & { bootstrapTabby?: (options: FixtureBootstrapOptions) => Promise<unknown> }

const fixtureToken = 'tabby-rs-web-fixture-token'
const output = document.querySelector<HTMLPreElement>('#fixture-output')!
const status = document.querySelector<HTMLElement>('#fixture-status')!
const tokenInput = document.querySelector<HTMLInputElement>('#fixture-token')!
const hostInput = document.querySelector<HTMLInputElement>('#fixture-host')!
const portInput = document.querySelector<HTMLInputElement>('#fixture-port')!
const terminalInput = document.querySelector<HTMLInputElement>('#fixture-terminal-input')!
const resizeButton = document.querySelector<HTMLButtonElement>('#fixture-resize')!
const sftpButton = document.querySelector<HTMLButtonElement>('#fixture-sftp-list')!
const settingsInput = document.querySelector<HTMLTextAreaElement>('#fixture-settings')!

let connector: WebGatewayConnector|null = null
let socket: WebGatewaySocket|null = null

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
    socket?.close()
    socket = connector.createSocket()
    socket.connect$.subscribe(() => {
        setStatus('connected')
        log('gateway connected')
    })
    socket.data$.subscribe(data => {
        const text = new TextDecoder().decode(data)
        log(`received: ${JSON.stringify(text)}`)
    })
    socket.error$.subscribe(error => {
        setStatus(`error: ${error.message}`)
        log(`gateway error: ${error.message}`)
    })
    socket.close$.subscribe(() => log('gateway closed'))
    void socket.connect({
        host: hostInput.value || 'fixture.example.test',
        port: Number(portInput.value) || 22,
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
    if (!socket) {
        setStatus('connect before sending terminal input')
        return
    }
    const value = `${terminalInput.value}\r`
    socket.write(new TextEncoder().encode(value))
    log(`sent: ${JSON.stringify(value)}`)
    terminalInput.value = ''
})

resizeButton.addEventListener('click', () => {
    const columns = Math.max(1, Math.floor(window.innerWidth / 8))
    const rows = Math.max(1, Math.floor(window.innerHeight / 16))
    document.body.dataset.viewport = `${columns}x${rows}`
    log(`viewport resize observed: ${columns}x${rows}`)
})

sftpButton.addEventListener('click', () => {
    if (!socket) {
        setStatus('connect before requesting SFTP list')
        return
    }
    const request = `SFTP-LIST /\r`
    socket.write(new TextEncoder().encode(request))
    log(`sent: ${JSON.stringify(request)}`)
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
    try {
        const hostConnector: WebHostConnector = {
            createSocket: (..._args: unknown[]) => connector!.createSocket(),
            loadConfig: async () => window.localStorage.getItem('tabby-rs-web-fixture-settings') ?? '{}',
            saveConfig: async content => window.localStorage.setItem('tabby-rs-web-fixture-settings', content),
            getAppVersion: () => 'web-fixture',
        }
        await tabbyWindow.bootstrapTabby({
            packageModules: [],
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
