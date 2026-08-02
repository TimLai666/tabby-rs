import { Injectable } from '@angular/core'

import {
    ChildProcess,
    PTYInterface,
    PTYProxy,
    SessionOptions,
} from '../../../tabby-local/src/api'
import { HostBridge } from '../api/hostBridge'
import {
    PtyErrorEvent,
    PtyExitEvent,
    PtyOutputEvent,
    PtySpawnResponse,
} from '../api/pty'
import { TauriSpawnRequestService } from './shellProvider.service'

interface LegacySpawnOptions {
    cwd?: string|null
    env?: Record<string, string>
    cols?: number
    rows?: number
}

type EventHandler = (...args: any[]) => void

interface ResizeWaiter {
    resolve: () => void
    reject: (error: unknown) => void
}

@Injectable()
export class TauriPTYInterface extends PTYInterface {
    constructor (
        private spawnRequests: TauriSpawnRequestService,
        private bridge: HostBridge,
    ) {
        super()
    }

    override async spawn (
        command: string,
        args: string[],
        options: LegacySpawnOptions = {},
    ): Promise<PTYProxy> {
        const prepared = await this.spawnRequests.prepare(this.toSessionOptions(command, args, options))
        const response = await this.bridge.invoke('pty.spawn', {
            prepared,
            columns: options.cols ?? 80,
            rows: options.rows ?? 30,
        })
        return TauriPTYProxy.create(response, this.bridge)
    }

    override async restore (id: string): Promise<PTYProxy|null> {
        try {
            if (!await this.bridge.invoke('pty.exists', { id })) {
                return null
            }
            const pid = await this.bridge.invoke('pty.getPid', { id })
            return await TauriPTYProxy.create({ id, pid }, this.bridge)
        } catch (error) {
            console.info('PTY session ended during restore:', error)
            return null
        }
    }

    private toSessionOptions (
        command: string,
        args: string[],
        options: LegacySpawnOptions,
    ): SessionOptions {
        return {
            restoreFromPTYID: null,
            command,
            args,
            cwd: options.cwd ?? null,
            env: options.env ?? {},
            width: options.cols ?? null,
            height: options.rows ?? null,
            shellType: null,
            pauseAfterExit: false,
            runAsAdministrator: false,
        }
    }
}

export class TauriPTYProxy extends PTYProxy {
    private handlers = new Map<string, Set<EventHandler>>()
    private unlisteners: (() => void)[] = []
    private queuedOutput: Uint8Array[] = []
    private pendingExit: PtyExitEvent|null = null
    private expectedSequence: number|null = null
    private truePID: Promise<number>
    private resizeTimer: ReturnType<typeof setTimeout>|null = null
    private pendingResize: {
        columns: number
        rows: number
        waiters: ResizeWaiter[]
    }|null = null

    private detached = false

    private constructor (
        private response: PtySpawnResponse,
        private bridge: HostBridge,
    ) {
        super()
        this.truePID = new Promise(resolve => setTimeout(resolve, 2000))
            .then(() => this.bridge.invoke('pty.getTruePid', { id: this.response.id }))
            .catch(() => this.getPID())
    }

    static async create (
        response: PtySpawnResponse,
        bridge: HostBridge,
    ): Promise<TauriPTYProxy> {
        const proxy = new TauriPTYProxy(response, bridge)
        await proxy.startListening()
        await bridge.invoke('pty.attach', { id: response.id })
        return proxy
    }

    override getID (): string {
        return this.response.id
    }

    override getPID (): Promise<number> {
        return Promise.resolve(this.response.pid)
    }

    override getTruePID (): Promise<number> {
        return this.truePID
    }

    override subscribe (event: string, handler: EventHandler): void {
        const handlers = this.handlers.get(event) ?? new Set<EventHandler>()
        handlers.add(handler)
        this.handlers.set(event, handlers)

        if (event === 'data' && this.queuedOutput.length) {
            const queued = this.queuedOutput
            this.queuedOutput = []
            for (const chunk of queued) {
                handler(chunk)
            }
        }
        if ((event === 'exit' || event === 'close') && this.pendingExit) {
            this.emitExitTo(event, handler, this.pendingExit)
        }
    }

    override ackData (bytes: number): void {
        void this.bridge.invoke('pty.ack', {
            id: this.response.id,
            bytes,
        }).catch(error => console.warn('PTY acknowledgement failed:', error))
    }

    override unsubscribeAll (): void {
        if (this.detached) {
            return
        }
        this.detached = true
        for (const unlisten of this.unlisteners.splice(0)) {
            unlisten()
        }
        this.handlers.clear()
        this.queuedOutput = []
        void this.bridge.invoke('pty.detach', { id: this.response.id })
            .catch(error => console.warn('PTY detach failed:', error))
    }

    override resize (columns: number, rows: number): Promise<void> {
        return new Promise<void>((resolve, reject) => {
            if (this.pendingResize) {
                this.pendingResize.columns = columns
                this.pendingResize.rows = rows
                this.pendingResize.waiters.push({ resolve, reject })
            } else {
                this.pendingResize = {
                    columns,
                    rows,
                    waiters: [{ resolve, reject }],
                }
            }
            if (!this.resizeTimer) {
                this.resizeTimer = setTimeout(() => void this.flushResize(), 16)
            }
        })
    }

    override write (data: Buffer): Promise<void> {
        return this.bridge.invoke('pty.write', {
            id: this.response.id,
            data: Array.from(data),
        })
    }

    override kill (signal?: string): Promise<void> {
        return this.bridge.invoke('pty.kill', {
            id: this.response.id,
            signal: signal ?? null,
        })
    }

    override getChildProcesses (): Promise<ChildProcess[]> {
        return this.bridge.invoke('pty.getChildren', { id: this.response.id })
    }

    override getWorkingDirectory (): Promise<string|null> {
        return this.bridge.invoke('pty.getCwd', { id: this.response.id })
    }

    private async startListening (): Promise<void> {
        this.unlisteners.push(
            await this.bridge.listen('pty.output', payload => this.onOutput(payload)),
            await this.bridge.listen('pty.exit', payload => this.onExit(payload)),
            await this.bridge.listen('pty.error', payload => this.onError(payload)),
        )
    }

    private onOutput (payload: PtyOutputEvent): void {
        if (payload.id !== this.response.id) {
            return
        }
        if (this.expectedSequence !== null && payload.sequence !== this.expectedSequence) {
            this.onError({
                id: payload.id,
                code: 'sequenceGap',
                details: `Expected PTY output sequence ${this.expectedSequence}, received ${payload.sequence}`,
            })
        }
        this.expectedSequence = payload.sequence + 1
        const chunk = Uint8Array.from(payload.data)
        const handlers = this.handlers.get('data')
        if (!handlers?.size) {
            this.queuedOutput.push(chunk)
            return
        }
        for (const handler of handlers) {
            handler(chunk)
        }
    }

    private onExit (payload: PtyExitEvent): void {
        if (payload.id !== this.response.id) {
            return
        }
        this.pendingExit = payload
        for (const event of ['exit', 'close']) {
            for (const handler of this.handlers.get(event) ?? []) {
                this.emitExitTo(event, handler, payload)
            }
        }
    }

    private onError (payload: PtyErrorEvent): void {
        if (payload.id !== this.response.id) {
            return
        }
        console.warn(`PTY ${payload.code}: ${payload.details}`)
        for (const handler of this.handlers.get('error') ?? []) {
            handler(payload)
        }
    }

    private emitExitTo (
        event: string,
        handler: EventHandler,
        payload: PtyExitEvent,
    ): void {
        if (event === 'exit') {
            handler(payload.exitCode, payload.signal)
        } else {
            handler()
        }
    }

    private async flushResize (): Promise<void> {
        this.resizeTimer = null
        const pending = this.pendingResize
        this.pendingResize = null
        if (!pending) {
            return
        }
        try {
            await this.bridge.invoke('pty.resize', {
                id: this.response.id,
                columns: pending.columns,
                rows: pending.rows,
            })
            for (const waiter of pending.waiters) {
                waiter.resolve()
            }
        } catch (error) {
            for (const waiter of pending.waiters) {
                waiter.reject(error)
            }
        }
    }
}
