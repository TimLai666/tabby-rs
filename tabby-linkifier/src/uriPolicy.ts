export type UriDecision =
    | { action: 'open', normalized: string }
    | { action: 'confirm', normalized: string, reason: string }
    | { action: 'reject', reason: string }

export interface UriPolicyContext {
    source: 'terminal-output'|'user-input'|'plugin'
    cwd?: string|null
    allowedSchemes: string[]
}

const MAX_URI_LENGTH = 8192
const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/
const ENCODED_CONTROL_CHARACTERS = /%(?:0[0-9a-f]|7f|9[0-9a-f])/i
const SCHEME = /^[a-z][a-z\d+.-]*:/i
const WINDOWS_PATH = /^[a-z]:[\\/]/i
function encodePath (value: string): string {
    return value.split('/').map(part => encodeURIComponent(part).replace(/%3A/gi, ':')).join('/')
}

function resolveFilePath (raw: string, cwd?: string|null): string|null {
    let path = raw.replace(/\\/g, '/')
    if (path.startsWith('~')) {
        if (!cwd) {
            return null
        }
        path = `${cwd.replace(/[\\/]$/, '')}/${path.substring(2)}`
    } else if (!path.startsWith('/') && !WINDOWS_PATH.test(path) && !raw.startsWith('\\\\')) {
        if (!cwd) {
            return null
        }
        path = `${cwd.replace(/[\\/]$/, '')}/${path}`
    }

    if (WINDOWS_PATH.test(path)) {
        return new URL(`file:///${encodePath(path)}`).toString()
    }
    if (path.startsWith('//')) {
        return new URL(`file://${encodePath(path.substring(2))}`).toString()
    }
    return new URL(`file://${encodePath(path.startsWith('/') ? path : `/${path}`)}`).toString()
}

export function decideUri (raw: string, context: UriPolicyContext): UriDecision {
    if (typeof raw !== 'string' || !raw || raw.length > MAX_URI_LENGTH || CONTROL_CHARACTERS.test(raw)) {
        return { action: 'reject', reason: 'invalid-uri' }
    }

    if (raw.trim() !== raw) {
        return { action: 'reject', reason: 'invalid-uri' }
    }

    const filePath = raw.startsWith('/') || raw.startsWith('~') || raw.startsWith('.') || WINDOWS_PATH.test(raw) || raw.startsWith('\\\\')
        ? resolveFilePath(raw, context.cwd)
        : null
    if (filePath) {
        return { action: 'confirm', normalized: filePath, reason: 'local-file' }
    }
    if (!SCHEME.test(raw) && !raw.startsWith('www.')) {
        return { action: 'reject', reason: context.cwd ? 'invalid-uri' : 'path-without-cwd' }
    }
    if (SCHEME.test(raw) && /^(?:[a-z][a-z\d+.-]*:){2,}/i.test(raw) || raw.includes('\\')) {
        return { action: 'reject', reason: 'mixed-scheme' }
    }

    const scheme = raw.slice(0, raw.indexOf(':')).toLowerCase()
    if (['javascript', 'data', 'vbscript'].includes(scheme)) {
        return { action: 'reject', reason: 'unsafe-scheme' }
    }

    const url = (() => {
        try {
            return new URL(raw.startsWith('www.') ? `https://${raw}` : raw)
        } catch {
            return null
        }
    })()
    if (!url) {
        return { action: 'reject', reason: 'invalid-uri' }
    }
    if (!url.protocol || CONTROL_CHARACTERS.test(url.toString()) || ENCODED_CONTROL_CHARACTERS.test(url.toString())) {
        return { action: 'reject', reason: 'invalid-uri' }
    }
    const normalized = url.toString()
    if (['http:', 'https:', 'mailto:'].includes(url.protocol)) {
        return { action: 'open', normalized }
    }
    if (url.protocol === 'file:') {
        return { action: 'confirm', normalized, reason: 'local-file' }
    }
    return { action: 'confirm', normalized, reason: 'external-application' }
}
