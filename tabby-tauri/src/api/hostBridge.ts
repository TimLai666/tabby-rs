import { InjectionToken } from '@angular/core'
import { BootstrapData } from 'tabby-core'

export type UpdateChannel = 'stable' | 'nightly'

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

export interface CliAliasStatus {
    supported: boolean
    enabled: boolean
    aliasPath: string | null
    conflict: string | null
    message: string | null
}

export interface ConfigReadResult {
    yaml: string
    revision: string | null
    path: string
}

export interface ConfigWriteResult {
    revision: string
    path: string
}

export interface BackupFile {
    path: string
    sha256: string
    size: number
}

export interface BackupManifest {
    schemaVersion: number
    backupId: string
    createdAt: string
    reason: string
    sourceVersion: string
    channel: UpdateChannel
    files: BackupFile[]
    absent: string[]
}

export interface RestoreReport {
    backupId: string
    restored: string[]
    removed: string[]
}

export interface SecretReference {
    path: string
    kind: string
}

export interface ImportPlan {
    sourceDataDir: string
    config: boolean
    profiles: number
    plugins: string[]
    secretReferences: SecretReference[]
    sourceRevision: string
}

export interface ImportReportItem {
    kind: string
    name: string
    detail: string
}

export interface ImportReport {
    imported: ImportReportItem[]
    skipped: ImportReportItem[]
    failed: ImportReportItem[]
    requiresSecretReentry: string[]
    reportPath: string
    backupId: string
}

export interface LegacyCliArguments {
    _: string[]
    directory?: string
    command?: string[]
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
    'backup.create': {
        request: {
            reason: string
            sourceVersion?: string | null
            channel?: UpdateChannel | null
        }
        response: BackupManifest
    }
    'backup.list': {
        request: Record<string, never>
        response: BackupManifest[]
    }
    'backup.restore': {
        request: { backupId: string }
        response: RestoreReport
    }
    'config.read': {
        request: Record<string, never>
        response: ConfigReadResult
    }
    'config.write': {
        request: {
            yaml: string
            expectedRevision?: string | null
            requireMissing?: boolean
        }
        response: ConfigWriteResult
    }
    'identity.get': {
        request: Record<string, never>
        response: AppIdentity
    }
    'identity.aliasStatus': {
        request: Record<string, never>
        response: CliAliasStatus
    }
    'identity.setAlias': {
        request: { enabled: boolean }
        response: CliAliasStatus
    }
    'migration.detect': {
        request: Record<string, never>
        response: ImportPlan[]
    }
    'migration.execute': {
        request: {
            sourceDataDir: string
            config: boolean
            plugins: string[]
        }
        response: ImportReport
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
