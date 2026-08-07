export type RendererWriter = (data: string, done: () => void) => void

/**
 * Serializes renderer writes and resolves only from the renderer completion
 * callback. This is the backpressure boundary used by Tauri PTY ACKs.
 */
export class RendererWriteQueue {
    private chain = Promise.resolve()
    private disposed = false

    constructor (private writer: RendererWriter) { }

    write (data: string): Promise<void> {
        if (this.disposed) {
            return Promise.reject(new Error('Terminal renderer is disposed'))
        }

        const next = this.chain.then(() => new Promise<void>((resolve, reject) => {
            if (this.disposed) {
                reject(new Error('Terminal renderer is disposed'))
                return
            }
            try {
                this.writer(data, resolve)
            } catch (error) {
                reject(error)
            }
        }))

        // Keep the internal chain usable after a failed write while still
        // returning the original rejection to the caller.
        this.chain = next.catch(() => undefined)
        return next
    }

    dispose (): void {
        this.disposed = true
    }
}
