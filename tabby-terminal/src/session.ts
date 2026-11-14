import { Observable, Subject } from 'rxjs'
import { Logger } from 'tabby-core'
import { LoginScriptProcessor, LoginScriptsOptions } from './middleware/loginScriptProcessing'
import { OSCProcessor } from './middleware/oscProcessing'
import { SessionMiddlewareStack } from './api/middleware'

/** Output kept for a session whose terminal hasn't been shown yet (tab never focused) */
const MAX_INITIAL_DATA_BUFFER = 8 * 1024 * 1024

export interface SessionRenderOutput {
    data: string
    consumed: () => void
}

class OutputConsumptionBarrier {
    private pending = 0
    private sealed = false
    private completed = false

    constructor (private onConsumed?: () => void) { }

    addOutput (): () => void {
        if (!this.onConsumed) {
            return () => undefined
        }
        this.pending++
        let consumed = false
        return () => {
            if (consumed) {
                return
            }
            consumed = true
            this.pending--
            this.flush()
        }
    }

    seal (): void {
        this.sealed = true
        this.flush()
    }

    private flush (): void {
        if (this.completed || !this.sealed || this.pending > 0) {
            return
        }
        this.completed = true
        this.onConsumed?.()
    }
}

/**
 * A session object for a [[BaseTerminalTabComponent]]
 * Extend this to implement custom I/O and process management for your terminal tab
 */
export abstract class BaseSession {
    open: boolean
    readonly oscProcessor = new OSCProcessor()
    readonly middleware = new SessionMiddlewareStack()
    protected output = new Subject<string>()
    protected binaryOutput = new Subject<Buffer>()
    protected renderOutput = new Subject<SessionRenderOutput>()
    protected closed = new Subject<void>()
    protected destroyed = new Subject<void>()
    protected loginScriptProcessor: LoginScriptProcessor | null = null
    protected reportedCWD?: string
    private initialDataBuffer: Buffer[] = []
    private initialDataBufferLength = 0
    private initialDataBufferTruncated = false
    private initialDataBufferReleased = false
    private initialDataConsumptions: (() => void)[] = []
    private activeOutputBarrier?: OutputConsumptionBarrier

    get output$ (): Observable<string> { return this.output }
    get binaryOutput$ (): Observable<Buffer> { return this.binaryOutput }
    /** @hidden Renderer-consumption stream used for PTY backpressure. */
    get renderOutput$ (): Observable<SessionRenderOutput> { return this.renderOutput }
    get closed$ (): Observable<void> { return this.closed }
    get destroyed$ (): Observable<void> { return this.destroyed }

    constructor (protected logger: Logger) {
        this.middleware.push(this.oscProcessor)
        this.oscProcessor.cwdReported$.subscribe(cwd => {
            this.reportedCWD = cwd
        })

        this.middleware.outputToTerminal$.subscribe(data => {
            const consumed = this.activeOutputBarrier?.addOutput() ?? (() => undefined)
            if (!this.initialDataBufferReleased) {
                // Collect chunks instead of Buffer.concat per chunk (O(n^2), and unbounded for a
                // tab that is never focused). Keep only the newest output - the terminal keeps
                // just its scrollback anyway. PTY chunks start on UTF-8 boundaries, so dropping
                // whole chunks from the front can't split a character.
                this.initialDataBuffer.push(data)
                this.initialDataConsumptions.push(consumed)
                this.initialDataBufferLength += data.length
                while (this.initialDataBufferLength > MAX_INITIAL_DATA_BUFFER && this.initialDataBuffer.length > 1) {
                    this.initialDataBufferLength -= this.initialDataBuffer.shift()!.length
                    // Dropped output never reaches the renderer; release its backpressure now.
                    this.initialDataConsumptions.shift()?.()
                    this.initialDataBufferTruncated = true
                }
            } else {
                this.deliverOutput(data, consumed)
            }
        })

        this.middleware.outputToSession$.subscribe(data => this.write(data))
    }

    feedFromTerminal (data: Buffer): void {
        this.middleware.feedFromTerminal(data)
    }

    /**
     * Emits process output through the middleware stack.
     *
     * `onConsumed` runs only after every renderer-visible output produced by
     * this synchronous middleware pass has completed rendering. If the chunk
     * produces no visible output (for example a buffered OSC control sequence),
     * it is considered consumed once middleware has accepted it.
     */
    protected emitOutput (data: Buffer, onConsumed?: () => void): void {
        const barrier = new OutputConsumptionBarrier(onConsumed)
        const previousBarrier = this.activeOutputBarrier
        this.activeOutputBarrier = barrier
        try {
            this.middleware.feedFromSession(data)
        } finally {
            this.activeOutputBarrier = previousBarrier
            barrier.seal()
        }
    }

    releaseInitialDataBuffer (): void {
        if (this.initialDataBufferReleased) {
            return
        }
        this.initialDataBufferReleased = true
        if (this.initialDataBufferTruncated) {
            this.logger.warn(`Output buffered before the terminal was shown exceeded ${MAX_INITIAL_DATA_BUFFER} bytes; older output was dropped`)
        }
        const data = Buffer.concat([
            ...this.initialDataBufferTruncated ? [Buffer.from('\x1b[0m[... earlier output dropped ...]\r\n')] : [],
            ...this.initialDataBuffer,
        ])
        const consumptions = this.initialDataConsumptions
        this.initialDataBuffer = []
        this.initialDataBufferLength = 0
        this.initialDataConsumptions = []

        if (!data.length) {
            for (const consumed of consumptions) {
                consumed()
            }
            return
        }

        this.deliverOutput(data, () => {
            for (const consumed of consumptions) {
                consumed()
            }
        })
    }

    setLoginScriptsOptions (options: LoginScriptsOptions): void {
        const newProcessor = new LoginScriptProcessor(this.logger, options)
        if (this.loginScriptProcessor) {
            this.middleware.replace(this.loginScriptProcessor, newProcessor)
        } else {
            this.middleware.push(newProcessor)
        }
        this.loginScriptProcessor = newProcessor
    }

    private destroying = false

    async destroy (): Promise<void> {
        // closed$ subscribers can call destroy() again synchronously - closing a local tab
        // runs LocalTerminalTabComponent.ngOnDestroy() -> session.destroy(). That inner call
        // used to complete destroyed$ before this call emitted it, so destroyed$ subscribers
        // (e.g. the local session's pty.unsubscribeAll()) never ran and every closed session
        // stayed reachable through its PTY IPC listeners.
        if (this.destroying) {
            return
        }
        this.destroying = true
        if (this.open) {
            this.logger.info('Destroying')
            this.open = false
            this.closed.next()
            this.destroyed.next()
            await this.gracefullyKillProcess()
        }
        this.middleware.close()
        this.closed.complete()
        this.destroyed.complete()
        this.output.complete()
        this.binaryOutput.complete()
        this.renderOutput.complete()
    }

    private deliverOutput (data: Buffer, consumed: () => void): void {
        const text = data.toString()
        this.output.next(text)
        this.binaryOutput.next(data)
        this.renderOutput.next({ data: text, consumed })
    }

    abstract start (options: unknown): Promise<void>
    abstract resize (columns: number, rows: number): void
    abstract write (data: Buffer): void
    abstract kill (signal?: string): void
    abstract gracefullyKillProcess (): Promise<void>
    abstract supportsWorkingDirectory (): boolean
    abstract getWorkingDirectory (): Promise<string|null>
}
