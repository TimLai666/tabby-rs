import { Subject } from 'rxjs'

export interface WebGatewaySocketOptions {
    host: string
    port: number
}

export interface WebGatewaySocketLike {
    readonly readyState: number
    onclose: (() => void)|null
    onerror: ((event: { message?: string }) => void)|null
    onmessage: ((event: { data: unknown }) => void) | null
    onopen: (() => void)|null
    close: () => void
    send: (data: string|Uint8Array) => void
}

export type WebGatewaySocketFactory = (url: string) => WebGatewaySocketLike

export interface WebGatewayConnectorOptions {
    url: string
    authToken: string
    webSocketFactory?: WebGatewaySocketFactory
}

interface GatewayMessage {
    _: string
    [key: string]: unknown
}

const OPEN_STATE = 1

function defaultWebSocketFactory (url: string): WebGatewaySocketLike {
    if (typeof WebSocket === 'undefined') {
        throw new Error('WebSocket is not available in this browser')
    }
    return new WebSocket(url) as unknown as WebGatewaySocketLike
}

function asError (error: unknown, fallback: string): Error {
    if (error instanceof Error) {
        return error
    }
    if (typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string') {
        return new Error(error.message)
    }
    return new Error(fallback)
}

function normalizeConnectOptions (options: WebGatewaySocketOptions|number, host?: string): WebGatewaySocketOptions {
    if (typeof options === 'number') {
        if (!Number.isInteger(options) || options < 1 || options > 65535) {
            throw new Error(`Invalid web gateway port: ${options}`)
        }
        return {
            host: host ?? 'localhost',
            port: options,
        }
    }
    if (
        typeof options.host !== 'string'
        || !options.host
        || !Number.isInteger(options.port)
        || options.port < 1
        || options.port > 65535
    ) {
        throw new Error('Web gateway socket requires a valid host and port')
    }
    return {
        host: options.host,
        port: options.port,
    }
}

export class WebGatewaySocket {
    readonly connect$ = new Subject<void>()
    readonly data$ = new Subject<Uint8Array>()
    readonly error$ = new Subject<Error>()
    readonly close$ = new Subject<void>()

    private webSocket: WebGatewaySocketLike|null = null
    private options: WebGatewaySocketOptions|null = null
    private readonly pendingWrites: Uint8Array[] = []
    private connected = false
    private closed = false

    constructor (
        private readonly gatewayUrl: string,
        private readonly authToken: string,
        private readonly createWebSocket: WebGatewaySocketFactory = defaultWebSocketFactory,
    ) { }

    async connect (options: WebGatewaySocketOptions|number, host?: string): Promise<void> {
        if (this.closed) {
            throw new Error('Web gateway socket is already closed')
        }
        if (this.webSocket) {
            throw new Error('Web gateway socket is already connecting')
        }
        this.options = normalizeConnectOptions(options, host)
        try {
            const webSocket = this.createWebSocket(this.gatewayUrl)
            this.webSocket = webSocket
            webSocket.onerror = event => {
                this.close(asError(event, `Failed to connect to the web gateway at ${this.gatewayUrl}`))
            }
            webSocket.onmessage = event => {
                void this.handleMessage(event.data)
            }
            webSocket.onclose = () => this.close()
        } catch (error) {
            this.close(asError(error, `Failed to connect to the web gateway at ${this.gatewayUrl}`))
        }
    }

    private async handleMessage (data: unknown): Promise<void> {
        if (typeof data === 'string') {
            try {
                const message = JSON.parse(data) as GatewayMessage
                this.handleServiceMessage(message)
            } catch (error) {
                this.close(asError(error, 'Web gateway sent invalid JSON'))
            }
            return
        }
        try {
            this.data$.next(await this.toBytes(data))
        } catch (error) {
            this.close(asError(error, 'Web gateway sent an unsupported data frame'))
        }
    }

    private async toBytes (data: unknown): Promise<Uint8Array> {
        if (data instanceof Uint8Array) {
            return data
        }
        if (data instanceof ArrayBuffer) {
            return new Uint8Array(data)
        }
        if (typeof Blob !== 'undefined' && data instanceof Blob) {
            return new Uint8Array(await data.arrayBuffer())
        }
        throw new Error('Web gateway sent an unsupported data frame')
    }

    handleServiceMessage (message: GatewayMessage): void {
        if (message._ === 'hello') {
            this.sendServiceMessage({
                _: 'hello',
                version: 1,
                auth_token: this.authToken,
            })
        } else if (message._ === 'ready') {
            if (!this.options) {
                this.close(new Error('Web gateway became ready before connect options were set'))
                return
            }
            this.sendServiceMessage({
                _: 'connect',
                host: this.options.host,
                port: this.options.port,
            })
        } else if (message._ === 'connected') {
            this.connected = true
            this.connect$.next()
            this.connect$.complete()
            this.flushWrites()
        } else if (message._ === 'error') {
            this.close(new Error(typeof message.details === 'string' ? message.details : 'Web gateway rejected the connection'))
        }
    }

    private sendServiceMessage (message: GatewayMessage): void {
        this.webSocket?.send(JSON.stringify(message))
    }

    private flushWrites (): void {
        const writes = this.pendingWrites.splice(0)
        for (const chunk of writes) {
            this.sendBytes(chunk)
        }
    }

    private sendBytes (chunk: Uint8Array): void {
        if (this.webSocket?.readyState !== OPEN_STATE) {
            throw new Error('Web gateway socket is not open')
        }
        this.webSocket.send(chunk)
    }

    write (chunk: Uint8Array): void {
        if (this.closed) {
            throw new Error('Web gateway socket is closed')
        }
        if (!this.connected || this.webSocket?.readyState !== OPEN_STATE) {
            this.pendingWrites.push(Uint8Array.from(chunk))
            return
        }
        this.sendBytes(chunk)
    }

    close (error?: Error): void {
        if (this.closed) {
            return
        }
        this.closed = true
        this.connected = false
        const webSocket = this.webSocket
        this.webSocket = null
        try {
            webSocket?.close()
        } catch { }
        if (error) {
            this.error$.next(error)
        }
        this.connect$.complete()
        this.data$.complete()
        this.error$.complete()
        this.close$.next()
        this.close$.complete()
    }
}

export class WebGatewayConnector {
    readonly sockets = new Set<WebGatewaySocket>()

    constructor (private readonly options: WebGatewayConnectorOptions) { }

    createSocket (): WebGatewaySocket {
        const socket = new WebGatewaySocket(this.options.url, this.options.authToken, this.options.webSocketFactory)
        this.sockets.add(socket)
        socket.close$.subscribe(() => this.sockets.delete(socket))
        return socket
    }
}
