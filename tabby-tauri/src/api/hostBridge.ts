import { InjectionToken } from '@angular/core'
import { BootstrapData } from 'tabby-core'

export interface RuntimeInfo {
    host: 'tauri'
    platform: string
    arch: string
    version: string
}

export interface HostRequestMap {
    'app.bootstrap': {
        request: Record<string, never>
        response: BootstrapData
    }
    'app.runtimeInfo': {
        request: Record<string, never>
        response: RuntimeInfo
    }
    'app.quit': {
        request: Record<string, never>
        response: null
    }
}

export interface HostEventMap {
    'app.start': BootstrapData
}

export abstract class HostBridge {
    abstract invoke<K extends keyof HostRequestMap> (
        command: K,
        request: HostRequestMap[K]['request'],
    ): Promise<HostRequestMap[K]['response']>

    abstract listen<K extends keyof HostEventMap> (
        event: K,
        handler: (payload: HostEventMap[K]) => void,
    ): Promise<() => void>
}

export const TAURI_RUNTIME_INFO = new InjectionToken<RuntimeInfo>('TAURI_RUNTIME_INFO')
