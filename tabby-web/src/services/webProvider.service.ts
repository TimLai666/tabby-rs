import type { WebGatewayProtocol, WebGatewaySocket, WebGatewaySocketOptions } from './connectionGateway.service'

export interface WebProviderConnectOptions extends WebGatewaySocketOptions {
    protocol?: Exclude<WebGatewayProtocol, 'tcp'>
}

export interface WebSFTPEntry {
    name: string
    path: string
    directory: boolean
    size?: number
}

export class WebProviderSession {
    readonly connect$ = this.socket.connect$
    readonly data$ = this.socket.data$
    readonly service$ = this.socket.service$
    readonly error$ = this.socket.error$
    readonly close$ = this.socket.close$

    constructor (
        protected readonly connector: { createSFTPSession: () => WebSFTPSession },
        protected readonly socket: WebGatewaySocket,
        public readonly protocol: Exclude<WebGatewayProtocol, 'tcp'>,
    ) { }

    async connect (options: Omit<WebProviderConnectOptions, 'protocol'>): Promise<void> {
        await this.socket.connect({ ...options, protocol: this.protocol })
        await this.socket.waitForConnection()
    }

    write (data: Uint8Array): void {
        this.socket.write(data)
    }

    resize (columns: number, rows: number): Promise<null> {
        return this.request<null>('resize', { columns, rows })
    }

    protected request<T> (operation: string, payload: Record<string, unknown> = {}): Promise<T> {
        return this.socket.request<T>({
            _: 'provider-request',
            protocol: this.protocol,
            operation,
            ...payload,
        })
    }

    close (): void {
        this.socket.close()
    }
}

export class WebSSHSession extends WebProviderSession {
    constructor (
        connector: { createSFTPSession: () => WebSFTPSession },
        socket: WebGatewaySocket,
    ) {
        super(connector, socket, 'ssh')
    }

    openSFTP (): WebSFTPSession {
        return this.connector.createSFTPSession()
    }
}

export class WebSFTPSession extends WebProviderSession {
    constructor (
        connector: { createSFTPSession: () => WebSFTPSession },
        socket: WebGatewaySocket,
    ) {
        super(connector, socket, 'sftp')
    }

    async list (path: string): Promise<WebSFTPEntry[]> {
        return this.request<WebSFTPEntry[]>('list', { path })
    }

    stat (path: string): Promise<WebSFTPEntry> {
        return this.request<WebSFTPEntry>('stat', { path })
    }

    mkdir (path: string): Promise<null> {
        return this.request<null>('mkdir', { path })
    }

    remove (path: string): Promise<null> {
        return this.request<null>('remove', { path })
    }

    rename (from: string, to: string): Promise<null> {
        return this.request<null>('rename', { from, to })
    }
}

export class WebTelnetSession extends WebProviderSession {
    constructor (
        connector: { createSFTPSession: () => WebSFTPSession },
        socket: WebGatewaySocket,
    ) {
        super(connector, socket, 'telnet')
    }
}
