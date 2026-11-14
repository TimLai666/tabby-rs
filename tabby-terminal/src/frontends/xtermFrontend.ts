import deepEqual from 'deep-equal'
import { filter, fromEvent, takeUntil } from 'rxjs'
import { Injector } from '@angular/core'
import {
    ConfigService,
    getCSSFontFamily,
    getWindows10Build,
    HostAppService,
    HotkeysService,
    isWindowsBuild,
    Platform,
    PlatformService,
    TerminalColorScheme,
    ThemesService,
    WIN_BUILD_BUNDLED_CONPTY_SUPPORTED,
    WIN_BUILD_CONPTY_REFLOW_SUPPORTED,
} from 'tabby-core'

import {
    Frontend,
    SearchOptions,
    SearchState,
    TerminalLinkHandler,
    TerminalLinkProvider,
} from './frontend'
import { BaseTerminalProfile } from '../api/interfaces'
import { getXtermBackgroundColor } from '../helpers'
import { generatePalette } from '../generatePalette'
import { createTerminalRenderer } from '../renderer/rendererFactory'
import {
    TerminalRenderer,
    TerminalRendererFontOptions,
    TerminalRendererTheme,
} from '../renderer/terminalRenderer'

const RESIZE_MIN_INTERVAL = 32

// Fcitx5 applies Chinese punctuation during the browser's default text-input
// processing. If xterm handles these keys on keydown, it calls preventDefault()
// before Fcitx5 can emit the converted keypress/input event.
const LINUX_IME_TEXT_KEY_CODES = new Set([
    'Backquote',
    'Backslash',
    'BracketLeft',
    'BracketRight',
    'Comma',
    'Period',
    'Quote',
    'Semicolon',
    'Slash',
])

function isIMETextKey (event: KeyboardEvent): boolean {
    if (event.ctrlKey || event.altKey || event.metaKey) {
        return false
    }
    return LINUX_IME_TEXT_KEY_CODES.has(event.code) || event.code === 'Space' && event.shiftKey
}

/** @hidden */
export class XTermFrontend extends Frontend {
    enableResizing = true

    /**
     * @deprecated Legacy plugin compatibility only. New code must use the
     * renderer-neutral Frontend/TerminalRenderer APIs instead.
     */
    get xterm (): any {
        return this.renderer.getLegacyRendererHandle()
    }

    private renderer: TerminalRenderer
    private element?: HTMLElement
    private configuredFontSize = 0
    private configuredLinePadding = 0
    private zoom = 0
    private resizeHandler: () => void
    private configuredTheme?: TerminalRendererTheme
    private copyOnSelect = false
    private preventNextOnSelectionChangeEvent = false
    private pinnedToBottom = true
    private resizeObserver?: ResizeObserver
    private hostListenerCleanups: (() => void)[] = []
    private resizeTimeout?: ReturnType<typeof setTimeout>
    private resizeAnimationFrame?: number
    private resizePending = false
    private opened = false
    private disposed = false

    private configService: ConfigService
    private hotkeysService: HotkeysService
    private platformService: PlatformService
    private hostApp: HostAppService
    private themes: ThemesService

    private isAttachActive (): boolean {
        return !this.disposed && this.opened
    }

    constructor (injector: Injector) {
        super(injector)
        this.configService = injector.get(ConfigService)
        this.hotkeysService = injector.get(HotkeysService)
        this.platformService = injector.get(PlatformService)
        this.hostApp = injector.get(HostAppService)
        this.themes = injector.get(ThemesService)

        this.renderer = createTerminalRenderer({
            webgl: this.wantsWebGLRenderer(),
            sixel: this.configService.store.terminal.sixel,
            windowsPty: this.hostApp.platform === Platform.Windows ? {
                backend: this.configService.store.terminal.useConPTY ? 'conpty' : 'winpty',
                buildNumber: this.configService.store.terminal.useConPTY
                    && isWindowsBuild(WIN_BUILD_BUNDLED_CONPTY_SUPPORTED, this.hostApp.platform, this.hostApp.windowsBuild)
                    ? WIN_BUILD_CONPTY_REFLOW_SUPPORTED
                    : getWindows10Build(this.hostApp.platform, this.hostApp.windowsBuild),
            } : undefined,
        })

        this.renderer.events.binary$.pipe(takeUntil(this.destroyed$)).subscribe(data => {
            this.input.next(Buffer.from(data))
        })
        this.renderer.events.data$.pipe(takeUntil(this.destroyed$)).subscribe(data => {
            this.input.next(Buffer.from(data, 'utf-8'))
        })
        this.renderer.events.resize$.pipe(takeUntil(this.destroyed$)).subscribe(size => {
            this.resize.next(size)
        })
        this.renderer.events.titleChanged$.pipe(takeUntil(this.destroyed$)).subscribe(title => {
            this.title.next(title)
        })
        this.renderer.events.selectionChanged$.pipe(takeUntil(this.destroyed$)).subscribe(selection => {
            if (selection) {
                if (this.copyOnSelect && !this.preventNextOnSelectionChangeEvent) {
                    this.copySelection()
                }
                this.preventNextOnSelectionChangeEvent = false
            }
        })
        this.renderer.events.bell$.pipe(takeUntil(this.destroyed$)).subscribe(() => {
            this.bell.next()
        })
        this.renderer.events.alternateScreenChanged$.pipe(takeUntil(this.destroyed$)).subscribe(active => {
            this.alternateScreenActive.next(active)
        })

        const keyboardEventHandler = (name: string, event: KeyboardEvent) => {
            if (this.isAlternateScreenActive()) {
                let modifiers = 0
                modifiers += event.ctrlKey ? 1 : 0
                modifiers += event.altKey ? 1 : 0
                modifiers += event.shiftKey ? 1 : 0
                modifiers += event.metaKey ? 1 : 0
                if (event.key.startsWith('Arrow') && modifiers === 1) {
                    return true
                }
            }

            if (event.type === 'keydown' && event.key === '/' && event.ctrlKey) {
                this.input.next(Buffer.from('\u001f', 'binary'))
                return false
            }
            if (event.type === 'keydown' && event.key === '@' && event.ctrlKey) {
                this.input.next(Buffer.from('\u0000', 'binary'))
                return false
            }

            this.hotkeysService.pushKeyEvent(name, event)
            let result = true
            if (this.hotkeysService.matchActiveHotkey(true) !== null) {
                event.stopPropagation()
                event.preventDefault()
                result = false
            }
            return result
        }

        this.renderer.setKeyEventHandlers(
            event => {
                if (this.hostApp.platform !== Platform.Web) {
                    if (
                        event.getModifierState('Meta') && event.key.toLowerCase() === 'v' ||
                        event.key === 'Insert' && event.shiftKey
                    ) {
                        event.preventDefault()
                        return false
                    }
                }
                if (event.getModifierState('Meta') && event.key.startsWith('Arrow')) {
                    return false
                }

                // xterm calls this handler for keypress and keyup too, so pass the real
                // event type - labelling them all as keydown makes the hotkey service
                // match the same keystroke several times.
                const handled = keyboardEventHandler(event.type, event)
                if (!handled) {
                    // a hotkey claimed the event and already cancelled it
                    return false
                }

                // xterm 6 sends Alt+Left/Right as ESC[1;3D/C, which shells don't bind by default.
                // Keep xterm 5's word-jump sequences.
                if (
                    event.type === 'keydown' && event.altKey && !event.ctrlKey && !event.shiftKey && !event.metaKey &&
                    (event.key === 'ArrowLeft' || event.key === 'ArrowRight')
                ) {
                    const left = event.key === 'ArrowLeft'
                    const seq = this.hostApp.platform === Platform.macOS ? left ? '\x1bb' : '\x1bf' : left ? '\x1b[1;5D' : '\x1b[1;5C'
                    this.input.next(Buffer.from(seq, 'binary'))
                    return false
                }

                if (event.type === 'keydown' && this.hostApp.platform === Platform.Linux && isIMETextKey(event)) {
                    // Returning false keeps xterm from sending/cancelling keydown.
                    // The resulting keypress/input event contains either the IME
                    // commit string or the original character when IME is inactive.
                    return false
                }

                return handled
            },
            event => keyboardEventHandler('keyup', event),
        )

        const doResize = () => {
            try {
                const viewport = this.renderer.getViewportState()
                this.renderer.fit({
                    ...viewport,
                    pinnedToBottom: this.pinnedToBottom,
                })
            } catch (error) {
                console.warn('Could not resize terminal renderer', error)
            }
        }

        let lastResize = 0
        const runResize = () => {
            this.resizeAnimationFrame = undefined
            this.resizePending = false
            if (!this.isAttachActive()) {
                return
            }
            lastResize = Date.now()
            doResize()
        }
        this.resizeHandler = () => {
            if (this.resizePending) {
                return
            }
            this.resizePending = true
            const wait = Math.max(0, RESIZE_MIN_INTERVAL - (Date.now() - lastResize))
            if (wait > 0) {
                this.resizeTimeout = setTimeout(() => {
                    this.resizeTimeout = undefined
                    this.resizeAnimationFrame = requestAnimationFrame(runResize)
                }, wait)
            } else {
                this.resizeAnimationFrame = requestAnimationFrame(runResize)
            }
        }
    }

    protected wantsWebGLRenderer (): boolean {
        return false
    }

    async attach (host: HTMLElement, profile: BaseTerminalProfile): Promise<void> {
        if (this.disposed) {
            return
        }
        this.element = host
        this.renderer.open(host)
        this.opened = true

        await new Promise(resolve => setTimeout(resolve, this.hostApp.platform === Platform.Web ? 1000 : 0))
        if (!this.isAttachActive()) {
            return
        }
        this.configureColors(profile.terminalColorScheme)

        this.platformService.displayMetricsChanged$.pipe(
            takeUntil(this.destroyed$),
        ).subscribe(() => {
            this.renderer.clearTextureAtlas()
            this.resizeHandler()
        })

        await new Promise(resolve => setTimeout(resolve, 100))
        if (!this.isAttachActive()) {
            return
        }
        this.ready.next()
        this.ready.complete()

        window.addEventListener('resize', this.resizeHandler)
        this.hostListenerCleanups.push(() => window.removeEventListener('resize', this.resizeHandler))

        fromEvent(window, 'focus').pipe(
            takeUntil(this.destroyed$),
        ).subscribe(() => this.renderer.reactivate())

        this.resizeHandler()
        await new Promise(resolve => setTimeout(resolve, 0))
        if (!this.isAttachActive()) {
            return
        }

        this.addHostListener(host, 'wheel', (event: Event) => {
            const wheelEvent = event as WheelEvent
            if (wheelEvent.deltaY < 0) {
                this.pinnedToBottom = false
            }
            requestAnimationFrame(() => this.updatePinnedState())
        }, { capture: true, passive: true })

        this.hotkeysService.hotkey$.pipe(
            takeUntil(this.destroyed$),
            filter(hotkey => [
                'scroll-up',
                'scroll-down',
                'scroll-page-up',
                'scroll-page-down',
                'scroll-to-top',
                'scroll-to-bottom',
            ].includes(hotkey)),
        ).subscribe(hotkey => {
            if ([
                'scroll-up',
                'scroll-page-up',
                'scroll-to-top',
            ].includes(hotkey)) {
                this.pinnedToBottom = false
            }
            requestAnimationFrame(() => this.updatePinnedState())
        })

        this.addHostListener(host, 'dragover', event => this.dragOver.next(event as DragEvent))
        this.addHostListener(host, 'drop', event => this.drop.next(event as DragEvent))
        this.addHostListener(host, 'mousedown', event => this.mouseEvent.next(event as MouseEvent))
        this.addHostListener(host, 'mouseup', event => this.mouseEvent.next(event as MouseEvent))
        this.addHostListener(host, 'mousewheel', event => this.mouseEvent.next(event as MouseEvent))
        this.addHostListener(host, 'contextmenu', event => {
            event.preventDefault()
            event.stopPropagation()
        })

        this.resizeObserver = new ResizeObserver(() => this.resizeHandler())
        this.resizeObserver.observe(host)
    }

    detach (_host: HTMLElement): void {
        if (this.resizeTimeout !== undefined) {
            clearTimeout(this.resizeTimeout)
            this.resizeTimeout = undefined
        }
        if (this.resizeAnimationFrame !== undefined) {
            cancelAnimationFrame(this.resizeAnimationFrame)
            this.resizeAnimationFrame = undefined
        }
        this.resizePending = false
        this.resizeObserver?.disconnect()
        this.resizeObserver = undefined
        for (const cleanup of this.hostListenerCleanups.splice(0)) {
            cleanup()
        }
        this.opened = false
        this.element = undefined
    }

    destroy (): void {
        if (this.disposed) {
            return
        }
        this.disposed = true
        if (this.element) {
            this.detach(this.element)
        }
        super.destroy()
        this.renderer.dispose()
    }

    getSelection (): string {
        return this.renderer.getSelection()
    }

    getTextChunks (chunkSize?: number): AsyncIterable<Uint8Array> {
        return this.renderer.getTextChunks(chunkSize)
    }

    copySelection (): void {
        const text = this.getSelection()
        if (!text.trim().length) {
            return
        }
        if (text.length < 1024 * 32 && this.configService.store.terminal.copyAsHTML) {
            this.platformService.setClipboard({
                text,
                html: this.renderer.getSelectionAsHTML(),
            })
        } else {
            this.platformService.setClipboard({ text })
        }
    }

    selectAll (): void {
        this.renderer.selectAll()
    }

    clearSelection (): void {
        this.renderer.clearSelection()
    }

    focus (): void {
        setTimeout(() => {
            // Don't steal focus from modal dialogs (e.g. the SSH password prompt)
            // or the tab's own overlay inputs - the deferred focus from a window
            // focus event would otherwise land right after the user clicks into one
            const active = document.activeElement
            const tabHost = this.element?.parentElement
            if (
                active instanceof HTMLElement
                && !(this.renderer.element?.contains(active) ?? false)
                && active.matches('input, textarea, select')
                && ((tabHost?.contains(active) ?? false) || active.closest('ngb-modal-window') !== null)
            ) {
                return
            }
            this.renderer.focus()
        })
    }

    async write (data: string): Promise<void> {
        const wasPinned = this.pinnedToBottom
        const viewport = this.renderer.getViewportState()
        await this.renderer.write(data)
        if (wasPinned) {
            this.renderer.scrollToBottom()
        } else {
            const next = this.renderer.getViewportState()
            const targetY = Math.min(viewport.viewportY, next.baseY)
            if (next.viewportY !== targetY) {
                this.renderer.scrollToLine(targetY)
            }
        }
    }

    clear (): void {
        this.renderer.clear()
    }

    resetTerminalModes (): void {
        void this.renderer.resetTerminalModes()
    }

    visualBell (): void {
        if (this.element) {
            this.element.style.animation = 'none'
            void this.element.offsetWidth
            this.element.style.animation = 'terminalShakeFrames 0.3s ease'
        }
    }

    scrollToTop (): void {
        this.pinnedToBottom = false
        this.renderer.scrollToTop()
    }

    scrollPages (pages: number): void {
        this.renderer.scrollPages(pages)
        this.updatePinnedState()
    }

    scrollLines (amount: number): void {
        this.renderer.scrollLines(amount)
        this.updatePinnedState()
    }

    scrollToBottom (): void {
        this.pinnedToBottom = true
        this.renderer.scrollToBottom()
    }

    configure (profile: BaseTerminalProfile): void {
        const config = this.configService.store
        this.configuredFontSize = config.terminal.fontSize
        this.configuredLinePadding = config.terminal.linePadding
        this.copyOnSelect = config.terminal.copyOnSelect

        this.renderer.setOptions({
            columns: this.renderer.columns || 80,
            rows: this.renderer.rows || 30,
            scrollback: config.terminal.scrollbackLines,
            font: this.getFontOptions(),
            theme: this.getTheme(profile.terminalColorScheme),
            cursor: {
                style: config.terminal.cursor,
                blink: config.terminal.cursorBlink,
            },
            wordSeparator: config.terminal.wordSeparator,
            drawBoldTextInBrightColors: config.terminal.drawBoldTextInBrightColors,
            macOptionIsMeta: config.terminal.altIsMeta,
            minimumContrastRatio: config.terminal.minimumContrastRatio,
            platform: this.rendererPlatform(),
        }, profile)
        this.configuredTheme = this.getTheme(profile.terminalColorScheme)

        setImmediate(() => this.resizeHandler())
    }

    setZoom (zoom: number): void {
        this.zoom = zoom
        this.renderer.setOptions({ font: this.getFontOptions() })
        this.resizeHandler()
    }

    findNext (term: string, searchOptions?: SearchOptions): SearchState {
        if (this.copyOnSelect) {
            this.preventNextOnSelectionChangeEvent = true
        }
        return this.renderer.findNext(term, searchOptions)
    }

    findPrevious (term: string, searchOptions?: SearchOptions): SearchState {
        if (this.copyOnSelect) {
            this.preventNextOnSelectionChangeEvent = true
        }
        return this.renderer.findPrevious(term, searchOptions)
    }

    cancelSearch (): void {
        this.renderer.cancelSearch()
    }

    setLinkHandler (handler: TerminalLinkHandler | null): void {
        this.renderer.setLinkHandler(handler)
    }

    registerLinkProvider (provider: TerminalLinkProvider): () => void {
        return this.renderer.registerLinkProvider(provider)
    }

    saveState (): any {
        return this.renderer.saveState()
    }

    restoreState (state: string): void {
        void this.renderer.restoreState(state)
    }

    supportsBracketedPaste (): boolean {
        return this.renderer.supportsBracketedPaste()
    }

    isAlternateScreenActive (): boolean {
        return this.renderer.isAlternateScreenActive()
    }

    reactivate (): void {
        this.renderer.reactivate()
    }

    private updatePinnedState (): void {
        this.pinnedToBottom = this.renderer.getViewportState().pinnedToBottom
    }

    private configureColors (scheme: TerminalColorScheme | null): void {
        const theme = this.getTheme(scheme)
        if (!deepEqual(this.configuredTheme, theme)) {
            this.renderer.setOptions({ theme })
            this.configuredTheme = theme
        }
    }

    private getTheme (scheme: TerminalColorScheme | null): TerminalRendererTheme {
        const appColorScheme = this.themes._getActiveColorScheme()
        scheme = scheme ?? appColorScheme
        const extendedAnsi = this.configService.store.terminal.paletteGenerate ? generatePalette(
            scheme.colors,
            scheme.background,
            scheme.foreground,
            this.configService.store.terminal.paletteHarmonious,
        ) : undefined

        return {
            foreground: scheme.foreground,
            selectionBackground: scheme.selection ?? '#88888888',
            selectionForeground: scheme.selectionForeground ?? undefined,
            background: getXtermBackgroundColor(this.configService, this.themes, scheme),
            cursor: scheme.cursor,
            cursorAccent: scheme.cursorAccent,
            colors: [...scheme.colors],
            extendedAnsi,
        }
    }

    private getFontOptions (): TerminalRendererFontOptions {
        const config = this.configService.store
        const scale = Math.pow(1.1, this.zoom)
        return {
            family: getCSSFontFamily(config),
            size: this.configuredFontSize * scale,
            lineHeight: Math.max(
                1,
                (this.configuredFontSize + this.configuredLinePadding * 2) / this.configuredFontSize,
            ),
            weight: config.terminal.fontWeight,
            weightBold: config.terminal.fontWeightBold,
            ligatures: config.terminal.ligatures,
        }
    }

    private rendererPlatform (): 'windows' | 'macos' | 'linux' | 'web' {
        return {
            [Platform.Windows]: 'windows' as const,
            [Platform.macOS]: 'macos' as const,
            [Platform.Linux]: 'linux' as const,
            [Platform.Web]: 'web' as const,
        }[this.hostApp.platform]
    }

    private addHostListener (
        host: HTMLElement,
        type: string,
        listener: EventListener,
        options?: AddEventListenerOptions,
    ): void {
        host.addEventListener(type, listener, options)
        this.hostListenerCleanups.push(() => host.removeEventListener(type, listener, options))
    }
}

/** @hidden */
export class XTermWebGLFrontend extends XTermFrontend {
    protected override wantsWebGLRenderer (): boolean {
        return true
    }
}
