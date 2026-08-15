export type WebProcessPlatform = 'darwin'|'win32'|'linux'

export function detectWebProcessPlatform (userAgent: string): WebProcessPlatform {
    const normalized = userAgent.toLowerCase()
    if (/(?:macintosh|mac os x|iphone|ipad|ipod)/.test(normalized)) {
        return 'darwin'
    }
    if (normalized.includes('windows')) {
        return 'win32'
    }
    return 'linux'
}
