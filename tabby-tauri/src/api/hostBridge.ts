import { InjectionToken } from '@angular/core'
import { BootstrapData } from 'tabby-core'

export interface RuntimeInfo {
    host: 'tauri'
    platform: string
    arch: string
    version: string
}

export interface AppIdentity {
    productName: string
    appIdentifier: string
    cliName: string
    urlScheme: string
    dataDirName: string
    credentialService: string
    executable: string
    dataDir: string
    pluginsDir: string
    logsDir: string
    portable: boolean
    portableRoot: string | null
}

export interface LegacyCliArguments {
    _: string[]
    directory?: string
    command: string[]
    profileName?: string
    text?: string
    escape: boolean
    providerId?: string
    query?: string
    debug: boolean
    hidden: boolean
    profileNumber?: number
    newWindow: boolean
    safeMode: boolean
    config?: string
}

export interface LaunchRequest {
    profile: string | null
    cwd: string | null
    newWindow: boolean
    safeMode: boolean
    config: string | null
    command: string[]
    urls: string[]
    argv: LegacyCliArguments
}

export interface LaunchContext {
    request: LaunchRequest
    cwd: string
    secondInstance: boolean
    parseError: string | null
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
    'app.initialLaunch': {
        request: Record<string, never>
        response: LaunchContext | null
    }
    'app.quit': {
        request: Record<string, never>
        response: null
    }
    'identity.get': {
        request: Record<string, never>
        response: AppIdentity
    }
}

export interface HostEventMap {
    'app.start': BootstrapData
    'app.launch': LaunchContext
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
