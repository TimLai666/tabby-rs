import { Injectable } from '@angular/core'

import { HostBridge, HostEventMap, HostRequestMap } from '../api/hostBridge'

interface TauriEvent<T> {
    payload: T
}

interface TauriGlobal {
    core: {
        invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>
    }
    event: {
        listen: <T>(event: string, handler: (event: TauriEvent<T>) => void) => Promise<() => void>
    }
}

declare global {
    interface Window {
        __TAURI__?: TauriGlobal
    }
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
        const rustCommand = command.replace(/\./g, '_')
        return this.api.core.invoke<HostRequestMap[K]['response']>(rustCommand, { request })
    }

    async listen<K extends keyof HostEventMap> (
        event: K,
        handler: (payload: HostEventMap[K]) => void,
    ): Promise<() => void> {
        return this.api.event.listen<HostEventMap[K]>(event, message => handler(message.payload))
    }
}
