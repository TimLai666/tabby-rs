export type TerminalShellType = 'unix' | 'powershell' | 'cmd'

export function quoteTerminalPath (value: string, shellType: TerminalShellType): string {
    const path = value.replace(/[\x00-\x1F\x7F]/g, '')

    if (shellType === 'powershell') {
        return `'${path.replace(/['\u2018\u2019\u201A\u201B]/g, match => match + match)}'`
    }

    if (shellType === 'cmd') {
        if (!path) {
            return '""'
        }
        const escaped = path
            .replace(/\^/g, '^^')
            .replace(/!/g, '^!')
            .replace(/"/g, '""')
            .replace(/%/g, '%%')
        return `"${escaped}"`
    }

    return `'${path.replace(/'/g, `'\\''`)}'`
}

export function encodeTerminalPath (value: string, shellType: TerminalShellType, bracketedPaste: boolean): string {
    const data = `${quoteTerminalPath(value, shellType)} `
    return bracketedPaste ? `\x1b[200~${data}\x1b[201~` : data
}
