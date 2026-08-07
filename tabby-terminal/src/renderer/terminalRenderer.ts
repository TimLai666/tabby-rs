import type { Observable } from 'rxjs'
import type { BaseTerminalProfile, ResizeEvent } from '../api/interfaces'
import type { SearchOptions, SearchState } from '../frontends/frontend'

export interface TerminalRendererFontOptions {
    family: string
    size: number
    lineHeight: number
    weight: string | number
    weightBold: string | number
    ligatures: boolean
}

export interface TerminalRendererTheme {
    foreground: string
    background: string
    selectionBackground: string
    selectionForeground?: string
    cursor?: string
    cursorAccent?: string
    colors: string[]
    extendedAnsi?: string[]
}

export interface TerminalRendererCursorOptions {
    style: string
    blink: boolean
}

export interface TerminalRendererOptions {
    columns: number
    rows: number
    scrollback: number
    font: TerminalRendererFontOptions
    theme: TerminalRendererTheme
    cursor: TerminalRendererCursorOptions
    wordSeparator: string
    drawBoldTextInBrightColors: boolean
    macOptionIsMeta: boolean
    minimumContrastRatio: number
    platform: 'windows' | 'macos' | 'linux' | 'web'
}

export interface TerminalRendererEvents {
    data: string
    binary: string
    resize: ResizeEvent
    selectionChanged: string
    titleChanged: string
    bell: void
    alternateScreenChanged: boolean
}

export interface TerminalRendererConstructionOptions {
    webgl: boolean
    sixel: boolean
    windowsPty?: {
        backend: 'conpty' | 'winpty'
        buildNumber: number
    }
}

export interface TerminalRendererViewportState {
    viewportY: number
    baseY: number
    pinnedToBottom: boolean
}

export interface TerminalRendererEventStreams {
    data$: Observable<string>
    binary$: Observable<string>
    resize$: Observable<ResizeEvent>
    selectionChanged$: Observable<string>
    titleChanged$: Observable<string>
    bell$: Observable<void>
    alternateScreenChanged$: Observable<boolean>
}

/**
 * Renderer boundary underneath Tabby's higher-level Frontend API.
 *
 * Implementations own VT renderer-specific objects and addons. Code outside
 * this directory must not add new xterm.js dependencies.
 */
export abstract class TerminalRenderer {
    abstract readonly events: TerminalRendererEventStreams
    abstract readonly element: HTMLElement | null
    abstract readonly columns: number
    abstract readonly rows: number

    abstract open (container: HTMLElement): void
    abstract write (data: string): Promise<void>
    abstract fit (viewport: TerminalRendererViewportState): void
    abstract focus (): void
    abstract clear (): void
    abstract reset (): void
    abstract selectAll (): void
    abstract clearSelection (): void
    abstract getSelection (): string
    abstract getSelectionAsHTML (): string
    abstract findNext (query: string, options?: SearchOptions): SearchState
    abstract findPrevious (query: string, options?: SearchOptions): SearchState
    abstract cancelSearch (): void
    abstract setOptions (patch: Partial<TerminalRendererOptions>, profile?: BaseTerminalProfile): void
    abstract scrollToTop (): void
    abstract scrollLines (amount: number): void
    abstract scrollPages (pages: number): void
    abstract scrollToBottom (): void
    abstract scrollToLine (line: number): void
    abstract getViewportState (): TerminalRendererViewportState
    abstract saveState (): string
    abstract restoreState (state: string): Promise<void>
    abstract supportsBracketedPaste (): boolean
    abstract isAlternateScreenActive (): boolean
    abstract resetTerminalModes (): Promise<void>
    abstract clearTextureAtlas (): void
    abstract reactivate (): void
    abstract setKeyEventHandlers (
        keyDown: (event: KeyboardEvent) => boolean,
        keyUp: (event: KeyboardEvent) => boolean,
    ): void
    abstract dispose (): void

    /**
     * Legacy plugin compatibility only. New code must use TerminalRenderer.
     * This accessor can disappear only in a future public API breaking release.
     */
    abstract getLegacyRendererHandle (): unknown
}
