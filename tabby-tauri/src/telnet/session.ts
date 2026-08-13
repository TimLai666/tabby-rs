import { Injector } from '@angular/core'
import { Observable, Subject } from 'rxjs'
import { LogService } from 'tabby-core'
import { BaseSession, InputProcessor, SessionMiddleware, TerminalStreamProcessor } from 'tabby-terminal'

import { HostBridge, TelnetConnectRequest, TelnetEchoEvent } from '../api/hostBridge'
import { TauriTelnetProfile } from './profile'

class TelnetEncodingMiddleware extends SessionMiddleware {
    private decoder: TextDecoder

    constructor (encoding: string) {
        super()
        try {
            this.decoder = new TextDecoder(encoding || 'utf-8')
        } catch {
            this.decoder = new TextDecoder('utf-8')
        }
    }

    feedFromSession (data: Buffer): void {
        const text = this.decoder.decode(data, { stream: true })
        if (text) {
            this.outputToTerminal.next(Buffer.from(text))
        }
    }

    close (): void {
        const remainder = this.decoder.decode()
        if (remainder) {
            this.outputToTerminal.next(Buffer.from(remainder))
        }
        super.close()
    }
}

export class TauriTelnetSession extends BaseSession {
    private id: string|null = null
    private destroying = false
    private readonly connectionId = window.crypto.randomUUID()
    private pendingOutput: number[][] = []
    private pendingExit: string|null = null
    private unlisteners: (() => void)[] = []
    private readonly serviceMessage = new Subject<string>()
    private readonly streamProcessor: TerminalStreamProcessor

    get serviceMessage$ (): Observable<string> { return this.serviceMessage }

    constructor (
        injector: Injector,
        private bridge: HostBridge,
        public profile: TauriTelnetProfile,
    ) {
        super(injector.get(LogService).create(`telnet-${profile.options.host}-${profile.options.port ?? 23}`))
        this.streamProcessor = new TerminalStreamProcessor(profile.options)
        this.middleware.push(new TelnetEncodingMiddleware(profile.options.encoding || 'utf-8'))
        this.middleware.push(this.streamProcessor)
        this.middleware.push(new InputProcessor(profile.options.input))
    }

    async start (): Promise<void> {
        if (this.open || this.destroying) {
            return
        }
        this.unlisteners.push(...await Promise.all([
            this.bridge.listen('telnet:output', event => {
                if (event.connectionId !== this.connectionId) {
                    return
                }
                if (!this.id) {
                    this.pendingOutput.push(event.data)
                } else {
                    this.emitOutput(Buffer.from(event.data))
                }
            }),
            this.bridge.listen('telnet:message', event => {
                if (event.connectionId === this.connectionId) {
                    this.serviceMessage.next(event.message)
                }
            }),
            this.bridge.listen('telnet:echo', event => {
                if (event.connectionId === this.connectionId) {
                    this.handleEcho(event)
                }
            }),
            this.bridge.listen('telnet:exit', event => {
                if (event.connectionId !== this.connectionId) {
                    return
                }
                if (!this.id) {
                    this.pendingExit = event.reason
                } else if (this.open) {
                    this.serviceMessage.next(`Connection closed: ${event.reason}`)
                    void this.destroy()
                }
            }),
        ]))

        const options = this.profile.options
        const info = await this.bridge.invoke('telnet.connect', {
            profileId: this.profile.id,
            connectionId: this.connectionId,
            host: options.host,
            port: options.port ?? 23,
            terminalType: options.terminalType || 'xterm-256color',
            connectTimeoutMs: options.connectTimeoutMs || 10_000,
            localEcho: options.inputMode === 'local-echo',
            keepalive: options.keepaliveInterval > 0 ? {
                intervalMs: options.keepaliveInterval,
                maxCount: options.keepaliveCountMax,
            } : null,
        } satisfies TelnetConnectRequest)
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) {
            await this.bridge.invoke('telnet.close', { id: info.id }).catch(error => this.logger.debug('Telnet close failed after cancelled start', error))
            return
        }
        this.id = info.id
        this.open = true
        for (const data of this.pendingOutput.splice(0)) {
            this.emitOutput(Buffer.from(data))
        }
        if (this.pendingExit) {
            void this.destroy()
            return
        }
        this.streamProcessor.start()
    }

    resize (columns: number, rows: number): void {
        if (!this.id) {
            return
        }
        void this.bridge.invoke('telnet.resize', { id: this.id, columns, rows }).catch(error => this.logger.warn('Telnet resize failed', error))
    }

    write (data: Buffer): void {
        if (!this.id || data.length === 0) {
            return
        }
        void this.bridge.invoke('telnet.write', { id: this.id, data: Array.from(data) }).catch(error => this.logger.warn('Telnet write failed', error))
    }

    kill (_signal?: string): void { void this.destroy() }

    async destroy (): Promise<void> {
        if (this.destroying) {
            return
        }
        this.destroying = true
        const id = this.id
        this.id = null
        if (id) {
            await this.bridge.invoke('telnet.close', { id }).catch(error => this.logger.debug('Telnet close failed after session end', error))
        }
        for (const unlisten of this.unlisteners.splice(0)) {
            unlisten()
        }
        this.serviceMessage.complete()
        await super.destroy()
    }

    async gracefullyKillProcess (): Promise<void> { await this.destroy() }
    supportsWorkingDirectory (): boolean { return false }
    async getWorkingDirectory (): Promise<string|null> { return null }

    private handleEcho (event: TelnetEchoEvent): void {
        this.streamProcessor.forceEcho = event.forceEcho
    }
}
