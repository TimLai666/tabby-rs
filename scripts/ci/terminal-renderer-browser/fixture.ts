import { Terminal } from '@xterm/xterm'
import { CanvasAddon } from '@xterm/addon-canvas'
import { Unicode11Addon } from '@xterm/addon-unicode11'
import { XtermRenderer } from '../../../tabby-terminal/src/renderer/xtermRenderer'

interface ParityResult {
    ok: boolean
    checks: string[]
    error?: string
}

declare global {
    interface Window {
        __TABBY_RENDERER_TEST__: Promise<ParityResult>
    }
}

const COLORS = [
    '#000000', '#cc0000', '#4e9a06', '#c4a000', '#3465a4', '#75507b', '#06989a', '#d3d7cf',
    '#555753', '#ef2929', '#8ae234', '#fce94f', '#729fcf', '#ad7fa8', '#34e2e2', '#eeeeec',
]

const FIXTURE_LINES = [
    'ASCII abc XYZ 0123456789',
    'CJK 漢字 中文 日本語 한글',
    'Emoji 🙂 🫨 👨‍👩‍👧‍👦',
    'Combining e\u0301 A\u0308 n\u0303',
    'Powerline \ue0b0 \ue0b1 \ue0b2',
]

function assert (condition: unknown, message: string): asserts condition {
    if (!condition) {
        throw new Error(message)
    }
}

function terminalOptions () {
    return {
        allowTransparency: true,
        allowProposedApi: true,
        overviewRulerWidth: 8,
        cols: 80,
        rows: 12,
        scrollback: 1000,
        fontFamily: 'monospace, "Noto Sans Mono", "Apple Color Emoji", "Segoe UI Emoji"',
        fontSize: 14,
        lineHeight: 1.15,
        fontWeight: '400' as const,
        fontWeightBold: '700' as const,
        cursorStyle: 'bar' as const,
        cursorBlink: false,
        wordSeparator: ' ()[]{}\'"',
        drawBoldTextInBrightColors: true,
        minimumContrastRatio: 1,
        theme: {
            foreground: '#e6e6e6',
            background: '#101014cc',
            selectionBackground: '#5378aa88',
            cursor: '#f0f0f0',
            cursorAccent: '#101014',
            black: COLORS[0],
            red: COLORS[1],
            green: COLORS[2],
            yellow: COLORS[3],
            blue: COLORS[4],
            magenta: COLORS[5],
            cyan: COLORS[6],
            white: COLORS[7],
            brightBlack: COLORS[8],
            brightRed: COLORS[9],
            brightGreen: COLORS[10],
            brightYellow: COLORS[11],
            brightBlue: COLORS[12],
            brightMagenta: COLORS[13],
            brightCyan: COLORS[14],
            brightWhite: COLORS[15],
        },
    }
}

function writeRaw (terminal: Terminal, data: string | Uint8Array): Promise<void> {
    return new Promise(resolve => terminal.write(data, resolve))
}

function bufferSnapshot (terminal: Terminal): string[] {
    const buffer = terminal.buffer.active
    const lines: string[] = []
    for (let row = 0; row < buffer.length; row++) {
        const line = buffer.getLine(row)?.translateToString(true)
        if (line !== undefined) {
            lines.push(line)
        }
    }
    return lines
}

function findCellWidth (terminal: Terminal, character: string): number | undefined {
    const buffer = terminal.buffer.active
    for (let row = 0; row < buffer.length; row++) {
        const line = buffer.getLine(row)
        if (!line) {
            continue
        }
        for (let column = 0; column < line.length; column++) {
            const cell = line.getCell(column)
            if (cell?.getChars() === character) {
                return cell.getWidth()
            }
        }
    }
    return undefined
}

function hashBytes (bytes: Uint8ClampedArray): number {
    let hash = 2166136261
    for (const byte of bytes) {
        hash ^= byte
        hash = Math.imul(hash, 16777619)
    }
    return hash >>> 0
}

function canvasSnapshot (container: HTMLElement): Array<{ width: number; height: number; hash: number }> {
    return [...container.querySelectorAll('canvas')].map(canvas => {
        const context = canvas.getContext('2d')
        assert(context, 'Canvas renderer did not expose a 2D context')
        const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data
        return {
            width: canvas.width,
            height: canvas.height,
            hash: hashBytes(pixels),
        }
    })
}

async function run (): Promise<ParityResult> {
    const checks: string[] = []
    const baselineHost = document.querySelector<HTMLElement>('#baseline')
    const adapterHost = document.querySelector<HTMLElement>('#adapter')
    assert(baselineHost && adapterHost, 'Fixture hosts are missing')

    const options = terminalOptions()
    const baseline = new Terminal(options)
    baseline.loadAddon(new Unicode11Addon())
    baseline.unicode.activeVersion = '11'
    baseline.loadAddon(new CanvasAddon())
    baseline.open(baselineHost)

    const adapter = new XtermRenderer({ webgl: false, sixel: false })
    adapter.open(adapterHost)
    adapter.setOptions({
        columns: 80,
        rows: 12,
        scrollback: options.scrollback,
        font: {
            family: options.fontFamily,
            size: options.fontSize,
            lineHeight: options.lineHeight,
            weight: options.fontWeight,
            weightBold: options.fontWeightBold,
            ligatures: false,
        },
        theme: {
            foreground: options.theme.foreground,
            background: options.theme.background,
            selectionBackground: options.theme.selectionBackground,
            cursor: options.theme.cursor,
            cursorAccent: options.theme.cursorAccent,
            colors: COLORS,
        },
        cursor: { style: 'beam', blink: false },
        wordSeparator: options.wordSeparator,
        drawBoldTextInBrightColors: options.drawBoldTextInBrightColors,
        macOptionIsMeta: false,
        minimumContrastRatio: options.minimumContrastRatio,
        platform: 'linux',
    })

    baseline.resize(80, 12)
    adapter.resize(80, 12)

    let resizeEvent: { columns: number; rows: number } | undefined
    const resizeSubscription = adapter.events.resize$.subscribe(event => {
        resizeEvent = event
    })
    adapter.resize(72, 10)
    baseline.resize(72, 10)
    assert(resizeEvent?.columns === 72 && resizeEvent.rows === 10, 'Renderer resize event did not match requested size')
    checks.push('resize-event')

    const payload = `${FIXTURE_LINES.join('\r\n')}\r\n`
    const binaryPayload = new TextEncoder().encode('Binary UTF-8: 台灣 🙂\r\n')
    await Promise.all([
        writeRaw(baseline, payload),
        adapter.write(payload),
    ])
    await Promise.all([
        writeRaw(baseline, binaryPayload),
        adapter.write(binaryPayload),
    ])

    const baselineSnapshot = bufferSnapshot(baseline)
    const adapterTerminal = adapter.getLegacyRendererHandle() as Terminal
    const adapterSnapshot = bufferSnapshot(adapterTerminal)
    assert(JSON.stringify(adapterSnapshot) === JSON.stringify(baselineSnapshot), 'Adapter buffer differs from raw xterm baseline')
    checks.push('buffer-parity')

    const renderedText = adapterSnapshot.join('\n')
    for (const fixture of ['漢字', '🙂', '🫨', '👨‍👩‍👧‍👦', 'é', '\ue0b0', '台灣']) {
        assert(renderedText.includes(fixture), `Rendered buffer lost fixture text: ${fixture}`)
    }
    checks.push('unicode-roundtrip')

    assert(findCellWidth(adapterTerminal, '漢') === findCellWidth(baseline, '漢'), 'CJK cell width differs from xterm baseline')
    assert(findCellWidth(adapterTerminal, '🙂') === findCellWidth(baseline, '🙂'), 'Emoji cell width differs from xterm baseline')
    assert(findCellWidth(adapterTerminal, 'é') === findCellWidth(baseline, 'é'), 'Combining character width differs from xterm baseline')
    checks.push('cell-width-parity')

    const legacy = adapter.getLegacyRendererHandle() as Terminal
    assert(legacy.buffer && legacy.options && typeof legacy.write === 'function', 'Legacy renderer accessor is not xterm-compatible')
    assert(legacy.options.cursorStyle === 'bar', 'Cursor option mapping failed')
    assert(legacy.options.fontFamily === options.fontFamily, 'Font fallback mapping failed')
    assert(legacy.options.theme?.background === options.theme.background, 'Theme/opacity mapping failed')
    checks.push('legacy-and-options')

    for (let index = 0; index < 20; index++) {
        const line = `scroll-${index}\r\n`
        await Promise.all([writeRaw(baseline, line), adapter.write(line)])
    }
    const scrollEvents: number[] = []
    const scrollSubscription = adapter.events.scroll$.subscribe(position => scrollEvents.push(position))
    adapter.scrollToTop()
    adapter.scrollToBottom()
    assert(scrollEvents.length > 0, 'Renderer did not publish scroll events')
    checks.push('scroll-event')

    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))
    const baselineCanvas = canvasSnapshot(baselineHost)
    const adapterCanvas = canvasSnapshot(adapterHost)
    assert(baselineCanvas.length > 0, 'Raw xterm did not render Canvas layers')
    assert(JSON.stringify(adapterCanvas) === JSON.stringify(baselineCanvas), 'Canvas pixel snapshot differs from raw xterm baseline')
    checks.push('canvas-visual-parity')

    const fallbackHost = document.querySelector<HTMLElement>('#fallback')
    assert(fallbackHost, 'Fallback host is missing')
    const fallback = new XtermRenderer({ webgl: true, sixel: false })
    fallback.open(fallbackHost)
    fallback.setOptions({
        font: {
            family: options.fontFamily,
            size: options.fontSize,
            lineHeight: options.lineHeight,
            weight: options.fontWeight,
            weightBold: options.fontWeightBold,
            ligatures: false,
        },
        theme: {
            foreground: options.theme.foreground,
            background: options.theme.background,
            selectionBackground: options.theme.selectionBackground,
            colors: COLORS,
        },
        cursor: { style: 'block', blink: false },
        platform: 'linux',
    })
    const fallbackInternals = fallback as any
    fallbackInternals.webGLAddon?.dispose()
    fallbackInternals.webGLAddon = undefined
    fallbackInternals.rendererRecoveryAttempts = 3
    fallbackInternals.pendingRendererRecovery = true
    Object.defineProperty(document, 'hasFocus', { configurable: true, value: () => true })
    fallbackInternals.recoverRenderer()
    assert(fallbackInternals.canvasAddon, 'WebGL recovery did not fall back to Canvas after bounded retries')
    checks.push('webgl-canvas-fallback')

    const unregister = adapter.registerLinkProvider({
        regex: /https?:\/\/[^\s]+/,
        activate: () => undefined,
    })
    assert((adapter as any).linkAddons.size === 1, 'Link addon was not registered inside renderer adapter')
    unregister()
    assert((adapter as any).linkAddons.size === 0, 'Link addon disposable did not clean up')
    checks.push('addon-cleanup')

    let completed = false
    const completionSubscription = adapter.events.scroll$.subscribe({ complete: () => { completed = true } })
    resizeSubscription.unsubscribe()
    scrollSubscription.unsubscribe()
    baseline.dispose()
    fallback.dispose()
    adapter.dispose()
    assert(completed, 'Renderer event streams did not complete on dispose')
    completionSubscription.unsubscribe()
    checks.push('renderer-dispose')

    return { ok: true, checks }
}

window.__TABBY_RENDERER_TEST__ = run().catch(error => ({
    ok: false,
    checks: [],
    error: error instanceof Error ? `${error.message}\n${error.stack ?? ''}` : String(error),
}))
