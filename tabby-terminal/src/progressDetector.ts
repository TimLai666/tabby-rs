export interface TerminalProgressState {
    value: number|null
    state: 'none'|'normal'|'indeterminate'|'paused'|'error'
    source: 'osc'|'heuristic'|'process'
}

export abstract class TerminalProgressDetector {
    abstract consume (data: Uint8Array): TerminalProgressState|null
    abstract reset (): void
}

const OSC_PROGRESS_PREFIX = [0x1b, 0x5d]
const OSC_PROGRESS_MAX_BYTES = 4096

export class OSCProgressDetector extends TerminalProgressDetector {
    private buffer = Buffer.alloc(0)

    consume (data: Uint8Array): TerminalProgressState|null {
        if (data.length) {
            this.buffer = Buffer.concat([this.buffer, Buffer.from(data)])
        }

        let result: TerminalProgressState|null = null
        while (this.buffer.length) {
            const start = this.buffer.indexOf(Buffer.from(OSC_PROGRESS_PREFIX))
            if (start === -1) {
                this.buffer = this.buffer.subarray(this.buffer[this.buffer.length - 1] === OSC_PROGRESS_PREFIX[0] ? this.buffer.length - 1 : this.buffer.length)
                break
            }
            if (start > 0) {
                this.buffer = this.buffer.subarray(start)
            }

            let end = this.buffer.indexOf(0x07, 2)
            let endLength = 1
            const st = this.buffer.indexOf(Buffer.from([0x1b, 0x5c]), 2)
            if (st !== -1 && (end === -1 || st < end)) {
                end = st
                endLength = 2
            }
            if (end === -1) {
                if (this.buffer.length > OSC_PROGRESS_MAX_BYTES) {
                    this.reset()
                }
                break
            }

            const body = this.buffer.subarray(2, end).toString('utf8')
            this.buffer = this.buffer.subarray(end + endLength)
            const match = /^9;4;([0-4])(?:;([0-9]{1,3}))?$/.exec(body)
            if (!match) {
                continue
            }

            const state = Number(match[1])
            const value = match[2] ? Number(match[2]) : null
            if (value !== null && value > 100) {
                continue
            }
            const states: TerminalProgressState['state'][] = ['none', 'normal', 'error', 'indeterminate', 'paused']
            result = {
                value: state === 1 ? value : null,
                state: states[state],
                source: 'osc',
            }
        }
        return result
    }

    reset (): void {
        this.buffer = Buffer.alloc(0)
    }
}

export function detectHeuristicProgress (data: string): TerminalProgressState|null {
    const match = /(^|[^\d])(\d+(\.\d+)?)%([^\d]|$)/.exec(data)
    if (!match) {
        return null
    }
    const value = Number(match[2])
    if (!Number.isFinite(value) || value <= 0 || value > 100) {
        return null
    }
    return { value, state: 'normal', source: 'heuristic' }
}
