import { Injectable } from '@angular/core'

import { HostBridge, HostEventMap, HostRequestMap } from '../api/hostBridge'

interface TauriEvent<T> {
    payload: T
}

interface TauriEventSource {
    listen: <T>(event: string, handler: (event: TauriEvent<T>) => void) => Promise<() => void>
}

const windowEvents = new Set<keyof HostEventMap>([
    'desktop:windowFocused', 'desktop:windowMoved', 'desktop:windowResized',
    'desktop:windowCloseRequested', 'desktop:fileDrop', 'desktop:themeChanged', 'desktop:displayMetricsChanged',
    'app:launch',
])

interface TauriGlobal {
    core: {
        invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>
    }
    event: TauriEventSource
    webviewWindow: {
        getCurrentWebviewWindow: () => TauriEventSource
    }
}

declare global {
    interface Window {
        __TAURI__?: TauriGlobal
    }
}

function toRustCommand (command: string): string {
    return command
        .replace(/\./g, '_')
        .replace(/[A-Z]/g, letter => `_${letter.toLowerCase()}`)
}

@Injectable({ providedIn: 'root' })
export class TauriHostBridge extends HostBridge {
    private get api (): TauriGlobal {
        const api = window.__TAURI__
        if (!api) {
            throw new Error('Tauri global API is unavailable')
        }
        return api
    }

    invoke<K extends keyof HostRequestMap> (
        command: K,
        request: HostRequestMap[K]['request'],
    ): Promise<HostRequestMap[K]['response']> {
        return this.api.core.invoke<HostRequestMap[K]['response']>(toRustCommand(command), { request })
    }

    async listen<K extends keyof HostEventMap> (
        event: K,
        handler: (payload: HostEventMap[K]) => void,
    ): Promise<() => void> {
        const source = windowEvents.has(event) ? this.api.webviewWindow.getCurrentWebviewWindow() : this.api.event
        return source.listen<HostEventMap[K]>(event, message => handler(message.payload))
    }
}
