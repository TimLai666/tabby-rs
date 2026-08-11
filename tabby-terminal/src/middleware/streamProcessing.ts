import bufferReplace from 'buffer-replace'
import colors from 'ansi-colors'
import binstring from 'binstring'
import { interval, debounce } from 'rxjs'
import { PassThrough, Readable, Writable } from 'stream'
import { ReadLine, createInterface as createReadline, clearLine } from 'readline'
import { SessionMiddleware } from '../api/middleware'

export type InputMode = null | 'local-echo' | 'readline' | 'readline-hex'
export type OutputMode = null | 'hex'
export type NewlineMode = null | 'cr' | 'lf' | 'crlf' | 'implicit_cr' | 'implicit_lf'

const MAX_INPUT_LINE_LENGTH = 64 * 1024

export interface StreamProcessingOptions {
    inputMode: InputMode
    inputNewlines: NewlineMode
    outputMode: OutputMode
    outputNewlines: NewlineMode
    maxInputLineLength?: number
    preserveOutputHexdumpOffset?: boolean
}

export function renderHexdump (data: Buffer, offset = 0): string {
    const columns = 16
    const lines: string[] = []
    for (let row = 0; row < data.length; row += columns) {
        const chunk = data.subarray(row, row + columns)
        const bytes = Array.from(chunk, byte => byte.toString(16).padStart(2, '0')).join(' ')
        const paddedBytes = bytes.padEnd(columns * 3 - 1, ' ')
        const human = Array.from(chunk, byte => byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : '.')
            .join('')
            .padEnd(columns, '╳')
        lines.push(`${(offset + row).toString(16).padStart(8, '0')} : ${paddedBytes}${colors.gray(' ｜ ')}${human}`)
    }
    return lines.join('\n')
}

export class TerminalStreamProcessor extends SessionMiddleware {
    forceEcho = false
    private inputReadline: ReadLine|null = null
    private inputPromptVisible = false
    private inputReadlineInStream: Readable & Writable
    private inputReadlineOutStream: Readable & Writable
    private started = false
    private inputLineLength = 0
    private droppingOverlongInput = false
    private outputHexdumpOffset = 0

    constructor (private options: StreamProcessingOptions) {
        super()
        this.inputReadlineInStream = new PassThrough()
        this.inputReadlineOutStream = new PassThrough()
        this.inputReadlineOutStream.on('data', data => {
            this.outputToTerminal.next(Buffer.from(data))
        })
        this.outputToTerminal$.pipe(debounce(() => interval(500))).subscribe(() => {
            if (this.started) {
                this.onOutputSettled()
            }
        })
    }

    start (): void {
        this.inputReadline = createReadline({
            input: this.inputReadlineInStream,
            output: this.inputReadlineOutStream,
            terminal: true,
            prompt: this.options.inputMode === 'readline-hex' ? 'hex> ' : '> ',
        })
        this.inputReadline.on('line', line => {
            this.onTerminalInput(Buffer.from(line + '\n'))
            this.resetInputPrompt()
        })
        this.started = true
    }

    feedFromSession (data: Buffer): void {
        if (this.options.inputMode?.startsWith('readline')) {
            if (this.inputPromptVisible) {
                clearLine(this.inputReadlineOutStream, 0)
                this.outputToTerminal.next(Buffer.from('\r'))
                this.inputPromptVisible = false
            }
        }

        data = this.replaceNewlines(data, this.options.outputNewlines)

        if (this.options.outputMode === 'hex') {
            this.outputToTerminal.next(Buffer.concat([
                Buffer.from('\r\n'),
                Buffer.from(renderHexdump(
                    data,
                    this.options.preserveOutputHexdumpOffset ? this.outputHexdumpOffset : 0,
                ).replaceAll('\n', '\r\n')),
                Buffer.from('\r\n\n'),
            ]))
            if (this.options.preserveOutputHexdumpOffset) {
                this.outputHexdumpOffset += data.length
            }
        } else {
            this.outputToTerminal.next(data)
        }
    }

    feedFromTerminal (data: Buffer): void {
        if (this.options.inputMode?.startsWith('readline')) {
            if (this.droppingOverlongInput) {
                if (data.includes(0x0a) || data.includes(0x0d)) {
                    this.droppingOverlongInput = false
                    this.inputLineLength = 0
                }
                return
            }
            this.inputLineLength += data.length
            const maxInputLineLength = this.options.maxInputLineLength
                ? Math.min(this.options.maxInputLineLength, MAX_INPUT_LINE_LENGTH)
                : null
            if (maxInputLineLength && this.inputLineLength > maxInputLineLength) {
                this.droppingOverlongInput = true
                this.outputToTerminal.next(Buffer.from('\r\n[Input rejected: line is too long]\r\n'))
                return
            }
            if (data.includes(0x0a) || data.includes(0x0d)) {
                this.inputLineLength = 0
            }
        }
        if (this.options.inputMode === 'local-echo' || this.forceEcho) {
            this.outputToTerminal.next(this.replaceNewlines(data, 'crlf'))
        }
        if (this.options.inputMode?.startsWith('readline')) {
            this.inputReadlineInStream.write(data)
        } else {
            this.onTerminalInput(data)
        }
    }

    resize (): void {
        if (this.options.inputMode?.startsWith('readline')) {
            this.inputReadlineOutStream.emit('resize')
        }
    }

    close (): void {
        this.inputReadline?.close()
        super.close()
    }

    private onTerminalInput (data: Buffer) {
        if (this.options.inputMode === 'readline-hex') {
            const tokens = data.toString().trim().split(/\s+/g).filter(t => !!t)
            const bytes: Buffer[] = []
            for (const [index, originalToken] of tokens.entries()) {
                const token = originalToken.startsWith('0x') ? originalToken.substring(2) : originalToken
                if (token.length !== 2 || !/^[0-9a-f]+$/i.test(token)) {
                    this.outputToTerminal.next(Buffer.from(`\r\n[Invalid hex token at index ${index}: ${originalToken}]\r\n`))
                    this.resetInputPrompt()
                    return
                }
                bytes.push(binstring(token, { 'in': 'hex' }))
            }
            data = Buffer.concat(bytes)
        }

        data = this.replaceNewlines(data, this.options.inputNewlines)
        this.outputToSession.next(data)
    }

    private onOutputSettled () {
        if (this.options.inputMode?.startsWith('readline') && !this.inputPromptVisible) {
            this.resetInputPrompt()
        }
    }

    private resetInputPrompt () {
        this.outputToTerminal.next(Buffer.from('\r\n'))
        this.inputReadline?.prompt(true)
        this.inputPromptVisible = true
    }

    private replaceNewlines (data: Buffer, mode?: NewlineMode): Buffer {
        if (!mode) {
            return data
        } else if (mode === 'implicit_cr') {
            return bufferReplace(data, '\n', '\r\n')
        } else if (mode === 'implicit_lf') {
            return bufferReplace(data, '\r', '\r\n')
        }

        data = bufferReplace(data, '\r\n', '\n')
        data = bufferReplace(data, '\r', '\n')
        const replacement = {
            strip: '',
            cr: '\r',
            lf: '\n',
            crlf: '\r\n',
        }[mode]
        return bufferReplace(data, '\n', replacement)
    }
}
