/* eslint-disable @typescript-eslint/no-type-alias */
import { Platform } from '../api/hostApp'

function detectBrowserPlatform (): Platform {
    if (typeof navigator === 'undefined') {
        return Platform.Web
    }

    const platform = `${navigator.platform} ${navigator.userAgent}`
    if (/Macintosh|Mac OS X/i.test(platform)) {
        return Platform.macOS
    }
    if (/Windows/i.test(platform)) {
        return Platform.Windows
    }
    if (/Linux/i.test(platform)) {
        return Platform.Linux
    }
    return Platform.Web
}

export function getMetaKeyName (platform: Platform = detectBrowserPlatform()): string {
    if (platform === Platform.macOS) {
        return '⌘'
    }
    if (platform === Platform.Windows) {
        return 'Win'
    }
    return 'Super'
}

export function getAltKeyName (platform: Platform = detectBrowserPlatform()): string {
    return platform === Platform.macOS ? '⌥' : 'Alt'
}

// Keep the legacy exports for plugin compatibility. Platform-aware code should
// use getMetaKeyName/getAltKeyName with HostAppService.configPlatform.
export const metaKeyName = getMetaKeyName()
export const altKeyName = getAltKeyName()

export interface KeyEventData {
    ctrlKey?: boolean
    metaKey?: boolean
    altKey?: boolean
    shiftKey?: boolean
    key?: string
    code?: string
    deltaX?: number
    deltaY?: number
    button?: number
    eventName: string
    time: number
    registrationTime: number
}

const REGEX_LATIN_KEYNAME = /^[A-Za-z]$/

export type KeyName = string
export type Keystroke = string

export function getKeyName (event: KeyEventData, platform?: Platform): KeyName {
    if (event.eventName === 'mouseup' || event.eventName === 'auxclick') {
        if (event.button === 1) {
            return 'MiddleClick'
        }
        return 'Mouse'
    }

    if (event.eventName === 'wheel') {
        const deltaX = event.deltaX ?? 0
        const deltaY = event.deltaY ?? 0
        if (Math.abs(deltaY) >= Math.abs(deltaX) && deltaY !== 0) {
            return deltaY < 0 ? 'WheelUp' : 'WheelDown'
        }
        if (deltaX !== 0) {
            return deltaX < 0 ? 'WheelLeft' : 'WheelRight'
        }
        return 'Wheel'
    }

    // eslint-disable-next-line @typescript-eslint/init-declarations
    let key: string
    if (event.key === 'Control') {
        key = 'Ctrl'
    } else if (event.key === 'Meta') {
        key = getMetaKeyName(platform)
    } else if (event.key === 'Alt') {
        key = getAltKeyName(platform)
    } else if (event.key === 'Shift') {
        key = 'Shift'
    } else if (event.key === '`') {
        key = '`'
    } else if (event.key === '~') {
        key = '~'
    } else {
        key = event.code ?? ''
        if (event.key && REGEX_LATIN_KEYNAME.test(event.key)) {
            // Handle Dvorak etc via the reported "character" instead of the scancode
            key = event.key.toUpperCase()
        } else {
            key = key.replace('Key', '')
            key = key.replace('Arrow', '')
            key = key.replace('Digit', '')
            key = {
                Comma: ',',
                Period: '.',
                Slash: '/',
                Backslash: '\\',
                IntlBackslash: '`',
                Minus: '-',
                Equal: '=',
                Semicolon: ';',
                Quote: '\'',
                BracketLeft: '[',
                BracketRight: ']',
            }[key] ?? key
        }
    }
    return key
}

export function getKeystrokeName (keys: KeyName[], platform?: Platform): Keystroke {
    const strictOrdering: KeyName[] = ['Ctrl', getMetaKeyName(platform), getAltKeyName(platform), 'Shift']
    keys = [
        ...strictOrdering.map(x => keys.find(k => k === x)).filter(x => !!x) as KeyName[],
        ...keys.filter(k => !strictOrdering.includes(k)),
    ]
    return keys.join('-')
}
