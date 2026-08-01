import { ShellType } from '../../../tabby-local/src/api'

import './hostBridge'

export interface DetectedShell {
    id: string
    providerId: string
    name: string
    command: string
    args: string[]
    env: Record<string, string>
    fsBase?: string
    cwd?: string
    icon?: string
    shellType?: ShellType
    hidden: boolean
    metadata: unknown
}

export interface ShellDetectionResult {
    shells: DetectedShell[]
    warnings: string[]
}

export interface PrepareSpawnRequest {
    command: string
    args: string[]
    cwd?: string | null
    profileEnvironment: Record<string, string>
    runtimeEnvironment: Record<string, string>
    shellType?: ShellType | null
    loginShell: boolean
}

export interface PreparedSpawnRequest {
    executable: string
    arguments: string[]
    cwd: string | null
    environment: Record<string, string>
    shellType: ShellType | null
    loginShell: boolean
    cwdFallback: boolean
}

declare module './hostBridge' {
    interface HostRequestMap {
        'shell.detect': {
            request: { identification?: string | null }
            response: ShellDetectionResult
        }
        'shell.prepareSpawn': {
            request: PrepareSpawnRequest
            response: PreparedSpawnRequest
        }
    }
}
