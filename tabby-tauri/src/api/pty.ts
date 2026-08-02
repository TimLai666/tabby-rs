import { ChildProcess } from '../../../tabby-local/src/api'
import { PreparedSpawnRequest } from './shell'

export interface PtySpawnRequest {
    prepared: PreparedSpawnRequest
    columns: number
    rows: number
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
