export type PasteRisk = 'multiline'|'control-characters'|'bracketed-paste-terminator'

export interface PasteInspection {
    text: string
    lineCount: number
    containsControlCharacters: boolean
    containsBracketedPasteEnd: boolean
    shouldConfirm: boolean
    reasons: PasteRisk[]
}

export interface TerminalInputState {
    alternateScreenActive: boolean
    warnOnMultilinePaste: boolean
    bracketedPaste: boolean
}

export abstract class PastePolicy {
    abstract inspect (text: string, state: TerminalInputState): PasteInspection
    abstract encode (text: string, state: TerminalInputState): Uint8Array
}

export interface TerminalPastePolicyOptions {
    windows: boolean
    replaceNewlinesWithSpaces: boolean
    trimWhitespace: boolean
}

const BRACKETED_PASTE_END = '\x1b[201~'

export class DefaultPastePolicy extends PastePolicy {
    constructor (private options: TerminalPastePolicyOptions) {
        super()
    }

    inspect (text: string, state: TerminalInputState): PasteInspection {
        const normalized = this.normalize(text, state)
        const lineCount = Math.max(1, (normalized.match(/\r|\n/g) ?? []).length + 1)
        const containsControlCharacters = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(normalized)
        const containsBracketedPasteEnd = normalized.includes(BRACKETED_PASTE_END)
        const reasons: PasteRisk[] = []

        if (lineCount > 1) {
            reasons.push('multiline')
        }
        if (containsControlCharacters) {
            reasons.push('control-characters')
        }
        if (containsBracketedPasteEnd) {
            reasons.push('bracketed-paste-terminator')
        }

        return {
            text: normalized,
            lineCount,
            containsControlCharacters,
            containsBracketedPasteEnd,
            shouldConfirm: !state.alternateScreenActive && state.warnOnMultilinePaste && lineCount > 1,
            reasons,
        }
    }

    encode (text: string, state: TerminalInputState): Uint8Array {
        const inspection = this.inspect(text, state)
        text = inspection.text.replaceAll(BRACKETED_PASTE_END, '')
        if (this.options.trimWhitespace && !state.alternateScreenActive && !inspection.shouldConfirm) {
            text = text.trimEnd()
            if (!text.includes('\r')) {
                text = text.trimStart()
            }
        }

        if (state.bracketedPaste) {
            text = `\x1b[200~${text}${BRACKETED_PASTE_END}`
        }
        return new TextEncoder().encode(text)
    }

    private normalize (text: string, state: TerminalInputState): string {
        let normalized = this.options.windows
            ? text.replaceAll('\r\n', '\r')
            : text.replaceAll('\n', '\r')

        if (this.options.replaceNewlinesWithSpaces) {
            normalized = normalized.replace(/[\r\n]+/g, ' ')
        }

        if (this.options.trimWhitespace && !state.alternateScreenActive) {
            const lineBreaks = normalized.match(/\r|\n/g) ?? []
            if (lineBreaks.length === 1 && /(\r|\n)$/.test(normalized)) {
                normalized = normalized.slice(0, -1)
            }
        }
        return normalized
    }
}
