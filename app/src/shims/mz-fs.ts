interface FileStatLike {
    isDirectory: () => boolean
}

function unavailable (): Error {
    const error = new Error('Direct renderer filesystem access is unavailable in Tabby RS')
    error.name = 'TabbyRsFilesystemUnavailableError'
    return error
}

interface TauriInvoke {
    core: { invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> }
}

/** Asks the native host whether a path is a directory, a file, or missing. */
async function pathKind (path: unknown): Promise<'directory' | 'file' | null> {
    const api = (window as unknown as { __TAURI__?: TauriInvoke }).__TAURI__
    if (typeof path !== 'string' || !path || !api) {
        return null
    }
    try {
        const result = await api.core.invoke<{ kind: 'directory' | 'file' | null }>('desktop_path_kind', { request: { path } })
        return result.kind
    } catch {
        return null
    }
}

export async function exists (path: unknown): Promise<boolean> {
    return await pathKind(path) !== null
}

export async function stat (path: unknown): Promise<FileStatLike> {
    const kind = await pathKind(path)
    if (!kind) {
        throw unavailable()
    }
    return { isDirectory: () => kind === 'directory' }
}

export async function lstat (path: unknown): Promise<FileStatLike> {
    return stat(path)
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
