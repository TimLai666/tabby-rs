interface FileStatLike {
    isDirectory: () => boolean
}

function unavailable (): Error {
    const error = new Error('Direct renderer filesystem access is unavailable in Tabby RS')
    error.name = 'TabbyRsFilesystemUnavailableError'
    return error
}

export async function exists (_path: unknown): Promise<boolean> {
    return false
}

export async function stat (_path: unknown): Promise<FileStatLike> {
    return { isDirectory: () => false }
}

export async function lstat (_path: unknown): Promise<FileStatLike> {
    return { isDirectory: () => false }
}

export async function realpath (path: unknown): Promise<string> {
    if (typeof path !== 'string' || !path) {
        throw unavailable()
    }
    return path
}

export async function access (_path: unknown): Promise<void> {
    throw unavailable()
}

export async function readFile (_path: unknown, _options?: unknown): Promise<never> {
    throw unavailable()
}
