import { ChildProcess } from '../../../tabby-local/src/api'
import { PreparedSpawnRequest } from './shell'

export interface SudoConfig {
    enabled: boolean
    secretRef: string|null
}

export interface SudoPromptEvent {
    sessionId: string
    promptId: string
    account?: string
}

export interface PtySpawnRequest {
    prepared: PreparedSpawnRequest
    columns: number
    rows: number
    sudo: SudoConfig|null
}

export interface PtySpawnResponse {
    id: string
    pid: number
}

export interface PtyOutputEvent {
    id: string
    sequence: number
    data: number[]
}

export interface PtyExitEvent {
    id: string
    exitCode: number|null
    signal: string|null
}

export interface PtyErrorEvent {
    id: string
    code: string
    details: string
}

export type { ChildProcess }
