import { Writable } from 'stream'

/** Preserve SerialPortStream's ordered writes and batching in the WebView. */
export class SerialWriteQueue {
    private readonly stream: Writable
    private closed = false

    constructor (send: (data: Buffer) => Promise<void>, onError: (error: Error) => void) {
        const write = (data: Buffer, done: (error?: Error|null) => void) => {
            const fail = (error: any) => done(error instanceof Error ? error : new Error(String(error?.details ?? error?.message ?? error)))
            const transmit = async () => {
                // The native bridge accepts at most 1 MiB per call. Keep a
                // large stream batch ordered across those calls.
                for (let offset = 0; offset < data.length; offset += 1024 * 1024) {
                    if (this.closed) {
                        return
                    }
                    await send(data.subarray(offset, offset + 1024 * 1024))
                }
            }
            void transmit().then(() => done(), fail)
        }
        this.stream = new Writable({
            highWaterMark: 64 * 1024,
            write: (data, _encoding, done) => write(data, done),
            writev: (chunks, done) => write(Buffer.concat(chunks.map(chunk => chunk.chunk)), done),
        })
        this.stream.on('error', error => {
            if (!this.closed) {
                this.closed = true
                onError(error)
            }
        })
    }

    write (data: Buffer): void {
        if (!this.closed && data.length) {
            this.stream.write(data)
        }
    }

    close (): void {
        this.closed = true
        this.stream.destroy()
    }
}
