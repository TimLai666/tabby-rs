export interface ProcessCompletion {
    tabId: string
    title: string
    command?: string
    exitCode?: number|null
    durationMs: number
    wasFocused: boolean
}

/** Return only the executable name so argv values never reach notifications. */
export function redactProcessCommand (command?: string): string|undefined {
    if (!command) {
        return undefined
    }
    const firstToken = /^(?:"([^"]+)"|'([^']+)'|(\S+))/.exec(command.trim())?.slice(1).find(Boolean)
    if (!firstToken) {
        return undefined
    }
    return firstToken.split(/[\\/]/).pop() ?? undefined
}
