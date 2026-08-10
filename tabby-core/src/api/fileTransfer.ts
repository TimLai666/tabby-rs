export type TransferDirection = 'upload' | 'download'

export type TransferState = 'pending' | 'running' | 'completed' | 'cancelled' | 'failed'

export interface TransferError {
    code: string
    message: string
    retryable?: boolean
}

export interface TransferDescriptor {
    id: string
    direction: TransferDirection
    name: string
    size?: number
    transferred: number
    state: TransferState
    error?: TransferError
}

/** Host-neutral streaming handle. Implementations must not buffer a whole file. */
export abstract class FileTransferHandle {
    abstract readonly id: string
    abstract read (maxBytes: number): Promise<Uint8Array>
    abstract write (chunk: Uint8Array): Promise<void>
    abstract close (): Promise<void>
    abstract cancel (): Promise<void>
}

const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f]/g
const WINDOWS_RESERVED_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?$/i

/** Remote names are suggestions, never local paths. */
export function sanitizeTransferName (name: string, fallback = 'download'): string {
    let result = name
        .replace(/[\\/]+/g, '_')
        .replace(CONTROL_CHARACTERS, '')
        .trim()

    if (!result || result === '.' || result === '..') {
        result = fallback
    }
    if (WINDOWS_RESERVED_NAME.test(result)) {
        result = `_${result}`
    }
    return result.slice(0, 255) || fallback
}

/** Resolve an untrusted relative path without allowing traversal or absolutes. */
export function sanitizeTransferRelativePath (value: string): string {
    const normalized = value.replace(/\\/g, '/')
    if (!normalized || normalized.startsWith('/') || /^[A-Za-z]:\//.test(normalized) || normalized.startsWith('//')) {
        throw new Error('Transfer path must be relative')
    }

    const parts = normalized.split('/').filter(part => part && part !== '.')
    if (!parts.length || parts.some(part => part === '..')) {
        throw new Error('Transfer path escapes its destination')
    }
    return parts.map(part => sanitizeTransferName(part)).join('/')
}
