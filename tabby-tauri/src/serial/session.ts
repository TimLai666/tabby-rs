import { Injector } from '@angular/core'
import { Observable, Subject } from 'rxjs'
import { LogService } from 'tabby-core'
import { BaseSession, InputProcessor, TerminalStreamProcessor } from 'tabby-terminal'

import { HostBridge, SerialConnectionStateEvent, SerialOpenRequest, SerialSignalState } from '../api/hostBridge'
import { TauriSerialProfile } from './profile'

export class TauriSerialSession extends BaseSession {
    private id: string|null = null
    private destroying = false
    private readonly connectionId = window.crypto.randomUUID()
    private readonly serviceMessage = new Subject<string>()
    private readonly streamProcessor: TerminalStreamProcessor
    private pendingOutput: number[][] = []
    private pendingState: SerialConnectionStateEvent|null = null
    private unlisteners: (() => void)[] = []

    get serviceMessage$ (): Observable<string> { return this.serviceMessage }

    constructor (
        injector: Injector,
        private bridge: HostBridge,
        public profile: TauriSerialProfile,
    ) {
        super(injector.get(LogService).create(`serial-${profile.options.port ?? 'unselected'}`))
        this.streamProcessor = new TerminalStreamProcessor(profile.options)
        this.middleware.push(this.streamProcessor)
        this.middleware.push(new InputProcessor(profile.options.input))
    }

    async start (): Promise<void> {
        if (this.open || this.destroying) {
            return
        }
        this.unlisteners.push(...await Promise.all([
            this.bridge.listen('serial.output', event => {
                if (event.connectionId === this.connectionId) {
                    if (!this.id) {
                        this.pendingOutput.push(event.data)
                    } else {
                        this.emitOutput(Buffer.from(event.data))
                    }
                }
            }),
            this.bridge.listen('serial.connectionState', event => {
                if (event.connectionId === this.connectionId) {
                    if (!this.id) {
                        this.pendingState = event
                    } else {
                        this.handleState(event)
                    }
                }
            }),
        ]))

        const options = this.profile.options
        const request: SerialOpenRequest = {
            profileId: this.profile.id,
            connectionId: this.connectionId,
            port: options.port ?? '',
            baudRate: options.baudRate ?? 115200,
            dataBits: options.dataBits,
            stopBits: options.stopBits,
            parity: options.parity,
            flowControl: options.flowControl,
            readTimeoutMs: options.readTimeoutMs,
            reconnect: options.reconnect,
        }
        const info = await this.bridge.invoke('serial.open', request)
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) {
            await this.bridge.invoke('serial.close', { id: info.id }).catch(error => this.logger.debug('Serial close failed after cancelled start', error))
            return
        }
        this.id = info.id
        this.open = true
        this.streamProcessor.start()
        for (const data of this.pendingOutput.splice(0)) {
            this.emitOutput(Buffer.from(data))
        }
        if (this.pendingState) {
            const state = this.pendingState
            this.pendingState = null
            this.handleState(state)
        }
    }

    resize (): void {
        this.streamProcessor.resize()
    }

    write (data: Buffer): void {
        if (!this.id || data.length === 0) {
            return
        }
        void this.bridge.invoke('serial.write', { id: this.id, data: Array.from(data) }).catch(error => this.logger.warn('Serial write failed', error))
    }

    async getSignals (): Promise<SerialSignalState> {
        if (!this.id) {
            throw new Error('Serial session is not open')
        }
        return this.bridge.invoke('serial.getSignals', { id: this.id })
    }

    async setSignal (signal: 'requestToSend'|'dataTerminalReady', value: boolean): Promise<void> {
        if (!this.id) {
            throw new Error('Serial session is not open')
        }
        await this.bridge.invoke('serial.setSignals', { id: this.id, signal, value })
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
            await this.bridge.invoke('serial.close', { id }).catch(error => this.logger.debug('Serial close failed after session end', error))
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

    private handleState (event: SerialConnectionStateEvent): void {
        if (event.state === 'connected') {
            this.serviceMessage.next(`Port connected: ${event.path ?? this.profile.options.port ?? ''}`)
        } else if (event.state === 'disconnected') {
            this.serviceMessage.next(`Port disconnected${event.error ? `: ${event.error}` : ''}`)
            if (!this.profile.options.reconnect.enabled) {
                void this.destroy()
            }
        } else if (event.state === 'reconnecting') {
            this.serviceMessage.next(`Reconnecting serial port${event.error ? `: ${event.error}` : ''}`)
        } else if (event.state === 'waiting') {
            this.serviceMessage.next('Waiting for the same serial device to return')
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        } else if (event.state === 'closed' && this.open) {
            void this.destroy()
        }
    }
}
