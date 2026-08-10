import { Subject } from 'rxjs'
import { Terminal, ITheme } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { LigaturesAddon } from '@xterm/addon-ligatures'
import { ISearchOptions, SearchAddon } from '@xterm/addon-search'
import { WebglAddon } from '@xterm/addon-webgl'
import { Unicode11Addon } from '@xterm/addon-unicode11'
import { SerializeAddon } from '@xterm/addon-serialize'
import { ImageAddon } from '@xterm/addon-image'
import { CanvasAddon } from '@xterm/addon-canvas'
import { WebLinksAddon } from '@xterm/addon-web-links'

import type { BaseTerminalProfile, ResizeEvent } from '../api/interfaces'
import type {
    SearchOptions,
    SearchState,
    TerminalLinkHandler,
    TerminalLinkProvider,
} from '../frontends/frontend'
import {
    TerminalRenderer,
    TerminalRendererConstructionOptions,
    TerminalRendererEventStreams,
    TerminalRendererOptions,
    TerminalRendererTheme,
    TerminalRendererViewportState,
} from './terminalRenderer'
import { RendererWriteData, RendererWriteQueue } from './writeQueue'
import '../frontends/xterm.css'

const COLOR_NAMES = [
    'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
    'brightBlack', 'brightRed', 'brightGreen', 'brightYellow', 'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite',
]

const MAX_WEBGL_RECOVERY_ATTEMPTS = 3

export class XtermRenderer extends TerminalRenderer {
    private terminal: Terminal
    private core: any
    private fitAddon = new FitAddon()
    private searchAddon = new SearchAddon()
    private serializeAddon = new SerializeAddon()
    private ligaturesAddon?: LigaturesAddon
    private webGLAddon?: WebglAddon
    private canvasAddon?: CanvasAddon
    private linkAddons = new Set<WebLinksAddon>()
    private searchState: SearchState = { resultCount: 0 }
    private opened = false
    private pendingRendererRecovery = false
    private rendererRecoveryAttempts = 0
    private searchLoaded = false
    private writeQueue: RendererWriteQueue
    private originalKeyUp?: (event: KeyboardEvent) => void
    private platform: TerminalRendererOptions['platform'] = 'web'

    private data = new Subject<string>()
    private binary = new Subject<Uint8Array>()
    private resizeEvents = new Subject<ResizeEvent>()
    private selectionChanged = new Subject<string>()
    private titleChanged = new Subject<string>()
    private bell = new Subject<void>()
    private scroll = new Subject<number>()
    private alternateScreenChanged = new Subject<boolean>()

    readonly events: TerminalRendererEventStreams = {
        data$: this.data,
        binary$: this.binary,
        resize$: this.resizeEvents,
        selectionChanged$: this.selectionChanged,
        titleChanged$: this.titleChanged,
        bell$: this.bell,
        scroll$: this.scroll,
        alternateScreenChanged$: this.alternateScreenChanged,
    }

    constructor (private construction: TerminalRendererConstructionOptions) {
        super()
        this.terminal = new Terminal({
            allowTransparency: true,
            allowProposedApi: true,
            overviewRulerWidth: 8,
            windowsPty: construction.windowsPty as any,
        })
        this.core = this.terminal['_core']
        this.writeQueue = new RendererWriteQueue((data, done) => this.terminal.write(data, done))

        this.terminal.onBinary(data => this.binary.next(this.binaryStringToBytes(data)))
        this.terminal.onData(data => this.data.next(data))
        this.terminal.onResize(({ cols, rows }) => this.resizeEvents.next({ columns: cols, rows }))
        this.terminal.onScroll(position => this.scroll.next(position))
        this.terminal.onTitleChange(title => this.titleChanged.next(title))
        this.terminal.onSelectionChange(() => this.selectionChanged.next(this.getSelection()))
        this.terminal.onBell(() => this.bell.next())
        this.terminal.buffer.onBufferChange(() => {
            this.alternateScreenChanged.next(this.isAlternateScreenActive())
        })

        this.terminal.loadAddon(this.fitAddon)
        this.terminal.loadAddon(this.serializeAddon)
        this.terminal.loadAddon(new Unicode11Addon())
        this.terminal.unicode.activeVersion = '11'
        if (construction.sixel) {
            this.terminal.loadAddon(new ImageAddon())
        }

        this.core._scrollToBottom = this.core.scrollToBottom.bind(this.core)
        this.core.scrollToBottom = () => null
    }

    get element (): HTMLElement | null | undefined {
        return this.terminal.element
    }

    get columns (): number {
        return this.terminal.cols
    }

    get rows (): number {
        return this.terminal.rows
    }

    open (container: HTMLElement): void {
        if (this.opened) {
            return
        }
        this.terminal.open(container)
        this.opened = true

        if (this.construction.webgl) {
            this.attachWebGLAddon()
        } else {
            this.ensureCanvasFallback()
        }

        if (!this.searchLoaded) {
            this.terminal.loadAddon(this.searchAddon)
            this.searchAddon.onDidChangeResults(state => {
                this.searchState = state
            })
            this.searchLoaded = true
        }
    }

    write (data: RendererWriteData): Promise<void> {
        return this.writeQueue.write(data)
    }

    resize (columns: number, rows: number): void {
        if (columns > 0 && rows > 0 && (columns !== this.terminal.cols || rows !== this.terminal.rows)) {
            this.terminal.resize(columns, rows)
        }
    }

    fit (viewport: TerminalRendererViewportState): void {
        if (!this.terminal.element || getComputedStyle(this.terminal.element).getPropertyValue('height') === 'auto') {
            return
        }

        this.fitAddon.fit()
        this.core.viewport?._refresh()

        if (viewport.pinnedToBottom) {
            this.core._scrollToBottom()
        } else {
            const targetY = Math.min(viewport.viewportY, this.terminal.buffer.active.baseY)
            this.terminal.scrollToLine(targetY)
        }

        this.core._renderService?._renderRows(0, this.terminal.rows - 1)
    }

    focus (): void {
        this.terminal.focus()
    }

    clear (): void {
        this.terminal.clear()
    }

    reset (): void {
        this.terminal.reset()
    }

    selectAll (): void {
        this.terminal.selectAll()
    }

    clearSelection (): void {
        this.terminal.clearSelection()
    }

    getSelection (): string {
        return this.terminal.getSelection()
    }

    async *getTextChunks (chunkSize = 64 * 1024): AsyncIterable<Uint8Array> {
        let pending = ''
        const buffer = this.terminal.buffer.active
        for (let index = 0; index < buffer.length; index++) {
            const line = buffer.getLine(index)
            pending += `${line?.translateToString(false) ?? ''}${index + 1 < buffer.length ? '\n' : ''}`
            while (pending.length >= chunkSize) {
                const chunk = pending.slice(0, chunkSize)
                pending = pending.slice(chunk.length)
                yield Buffer.from(chunk)
            }
        }
        if (pending) {
            yield Buffer.from(pending)
        }
    }

    getSelectionAsHTML (): string {
        return this.serializeAddon.serializeAsHTML({
            includeGlobalBackground: true,
            onlySelection: true,
        })
    }

    findNext (query: string, options?: SearchOptions): SearchState {
        const result = this.searchAddon.findNext(query, this.mapSearchOptions(options))
        return result ? this.searchState : { resultCount: 0 }
    }

    findPrevious (query: string, options?: SearchOptions): SearchState {
        const result = this.searchAddon.findPrevious(query, this.mapSearchOptions(options))
        return result ? this.searchState : { resultCount: 0 }
    }

    cancelSearch (): void {
        this.searchAddon.clearDecorations()
        this.focus()
    }

    setOptions (patch: Partial<TerminalRendererOptions>, _profile?: BaseTerminalProfile): void {
        if (patch.platform) {
            this.platform = patch.platform
            this.core.browser.isWindows = patch.platform === 'windows'
            this.core.browser.isLinux = patch.platform === 'linux'
            this.core.browser.isMac = patch.platform === 'macos'
        }

        const font = patch.font
        if (font) {
            this.terminal.options.fontFamily = font.family
            this.terminal.options.fontSize = font.size
            this.terminal.options.lineHeight = font.lineHeight
            this.terminal.options.fontWeight = font.weight as any
            this.terminal.options.fontWeightBold = font.weightBold as any
            if (this.opened && font.ligatures && !this.ligaturesAddon && this.platform !== 'web') {
                this.ligaturesAddon = new LigaturesAddon()
                this.terminal.loadAddon(this.ligaturesAddon)
            }
        }

        if (patch.cursor) {
            this.terminal.options.cursorStyle = this.mapCursorStyle(patch.cursor.style)
            this.terminal.options.cursorBlink = patch.cursor.blink
        }
        if (patch.theme) {
            this.terminal.options.theme = this.mapTheme(patch.theme)
        }
        if (patch.scrollback !== undefined) {
            this.terminal.options.scrollback = patch.scrollback
        }
        if (patch.wordSeparator !== undefined) {
            this.terminal.options.wordSeparator = patch.wordSeparator
        }
        if (patch.drawBoldTextInBrightColors !== undefined) {
            this.terminal.options.drawBoldTextInBrightColors = patch.drawBoldTextInBrightColors
        }
        if (patch.macOptionIsMeta !== undefined) {
            this.terminal.options.macOptionIsMeta = patch.macOptionIsMeta
        }
        if (patch.minimumContrastRatio !== undefined) {
            this.terminal.options.minimumContrastRatio = patch.minimumContrastRatio
        }

        if (this.terminal.cols && this.terminal.rows && this.core.charMeasure) {
            this.core.charMeasure.measure(this.core.options)
            this.core.renderer?._updateDimensions()
        }
    }

    setLinkHandler (handler: TerminalLinkHandler | null): void {
        this.terminal.options.linkHandler = handler ? {
            activate: (event, uri) => {
                void handler.activate(event, uri)
            },
        } : undefined
    }

    registerLinkProvider (provider: TerminalLinkProvider): () => void {
        const addon = new WebLinksAddon(
            (event, uri) => {
                void provider.activate(event, uri)
            },
            { urlRegex: provider.regex },
        )
        this.terminal.loadAddon(addon)
        this.linkAddons.add(addon)
        return () => {
            if (this.linkAddons.delete(addon)) {
                addon.dispose()
            }
        }
    }

    scrollToTop (): void {
        this.terminal.scrollToTop()
    }

    scrollLines (amount: number): void {
        this.terminal.scrollLines(amount)
    }

    scrollPages (pages: number): void {
        this.terminal.scrollPages(pages)
    }

    scrollToBottom (): void {
        this.core._scrollToBottom()
    }

    scrollToLine (line: number): void {
        this.terminal.scrollToLine(line)
    }

    getViewportState (): TerminalRendererViewportState {
        const buffer = this.terminal.buffer.active
        return {
            viewportY: buffer.viewportY,
            baseY: buffer.baseY,
            pinnedToBottom: buffer.viewportY >= buffer.baseY - 1,
        }
    }

    saveState (): string {
        return this.serializeAddon.serialize({
            excludeAltBuffer: true,
            excludeModes: true,
            scrollback: 1000,
        })
    }

    restoreState (state: string): Promise<void> {
        return this.write(state)
    }

    supportsBracketedPaste (): boolean {
        return this.terminal.modes.bracketedPasteMode
    }

    isAlternateScreenActive (): boolean {
        return this.terminal.buffer.active.type === 'alternate'
    }

    async resetTerminalModes (): Promise<void> {
        await this.write('\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l')
        await this.write('\x1b[?2004l')
    }

    clearTextureAtlas (): void {
        this.webGLAddon?.clearTextureAtlas()
        this.canvasAddon?.clearTextureAtlas()
    }

    reactivate (): void {
        if (this.pendingRendererRecovery || this.construction.webgl && !this.webGLAddon && !this.canvasAddon) {
            this.pendingRendererRecovery = true
            this.recoverRenderer()
        } else {
            this.rendererRecoveryAttempts = 0
            this.redraw()
        }
    }

    setKeyEventHandlers (
        keyDown: (event: KeyboardEvent) => boolean,
        keyUp: (event: KeyboardEvent) => boolean,
    ): void {
        this.terminal.attachCustomKeyEventHandler(keyDown)

        if (!this.originalKeyUp) {
            this.originalKeyUp = this.core._keyUp.bind(this.core)
            this.core._keyUp = (event: KeyboardEvent) => {
                this.core.updateCursorStyle(event)
                if (keyUp(event)) {
                    this.originalKeyUp?.(event)
                }
            }
        }
    }

    dispose (): void {
        this.writeQueue.dispose()
        for (const addon of this.linkAddons) {
            addon.dispose()
        }
        this.linkAddons.clear()
        this.webGLAddon?.dispose()
        this.canvasAddon?.dispose()
        this.ligaturesAddon?.dispose()
        this.searchAddon.dispose()
        this.serializeAddon.dispose()
        this.fitAddon.dispose()
        this.terminal.dispose()
        this.data.complete()
        this.binary.complete()
        this.resizeEvents.complete()
        this.selectionChanged.complete()
        this.titleChanged.complete()
        this.bell.complete()
        this.scroll.complete()
        this.alternateScreenChanged.complete()
    }

    getLegacyRendererHandle (): unknown {
        return this.terminal
    }

    private mapTheme (theme: TerminalRendererTheme): ITheme {
        const mapped: ITheme = {
            foreground: theme.foreground,
            background: theme.background,
            selectionBackground: theme.selectionBackground,
            selectionForeground: theme.selectionForeground,
            cursor: theme.cursor,
            cursorAccent: theme.cursorAccent,
            extendedAnsi: theme.extendedAnsi,
        }
        for (let i = 0; i < COLOR_NAMES.length && i < theme.colors.length; i++) {
            mapped[COLOR_NAMES[i]] = theme.colors[i]
        }
        return mapped
    }

    private mapCursorStyle (style: string): 'block' | 'underline' | 'bar' {
        if (style === 'beam') {
            return 'bar'
        }
        if (style === 'underline' || style === 'bar') {
            return style
        }
        return 'block'
    }

    private mapSearchOptions (options?: SearchOptions): ISearchOptions {
        return {
            ...options,
            decorations: {
                matchOverviewRuler: '#888888',
                activeMatchColorOverviewRuler: '#ffff00',
                matchBackground: '#888888',
                activeMatchBackground: '#ffff00',
            },
        }
    }

    private attachWebGLAddon (): void {
        try {
            const addon = new WebglAddon()
            addon.onContextLoss(() => this.onWebGLContextLoss())
            this.terminal.loadAddon(addon)
            this.webGLAddon = addon
        } catch {
            this.webGLAddon = undefined
            this.ensureCanvasFallback()
        }
    }

    private onWebGLContextLoss (): void {
        this.webGLAddon?.dispose()
        this.webGLAddon = undefined
        this.pendingRendererRecovery = true
        this.recoverRenderer()
    }

    private recoverRenderer (): void {
        if (!this.pendingRendererRecovery || !this.canRecoverRenderer()) {
            return
        }
        this.pendingRendererRecovery = false
        if (this.rendererRecoveryAttempts < MAX_WEBGL_RECOVERY_ATTEMPTS) {
            this.rendererRecoveryAttempts++
            this.attachWebGLAddon()
        } else {
            this.ensureCanvasFallback()
        }
        this.redraw()
    }

    private ensureCanvasFallback (): void {
        if (this.canvasAddon) {
            return
        }
        try {
            const addon = new CanvasAddon()
            this.terminal.loadAddon(addon)
            this.canvasAddon = addon
        } catch {
            this.canvasAddon = undefined
        }
    }

    private canRecoverRenderer (): boolean {
        return !!this.terminal.element && this.terminal.element.offsetParent !== null && document.hasFocus()
    }

    private redraw (): void {
        const renderService = this.core._renderService
        renderService?.clear()
        const viewport = this.getViewportState()
        this.fit(viewport)
        renderService?.handleResize(this.terminal.cols, this.terminal.rows)
    }

    private binaryStringToBytes (data: string): Uint8Array {
        const bytes = new Uint8Array(data.length)
        for (let index = 0; index < data.length; index++) {
            bytes[index] = data.charCodeAt(index) & 0xff
        }
        return bytes
    }
}
