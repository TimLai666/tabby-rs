import { ChildProcess } from '../../../tabby-local/src/api'
import {
    PtyErrorEvent,
    PtyExitEvent,
    PtyOutputEvent,
    PtySpawnRequest,
    PtySpawnResponse,
    SudoPromptEvent,
} from './pty'
import './hostBridge'

declare module './hostBridge' {
    interface HostRequestMap {
        'pty.spawn': {
            request: PtySpawnRequest
            response: PtySpawnResponse
        }
        'pty.exists': {
            request: { id: string }
            response: boolean
        }
        'pty.isAlive': {
            request: { id: string }
            response: boolean
        }
        'pty.attach': {
            request: { id: string }
            response: null
        }
        'pty.detach': {
            request: { id: string }
            response: null
        }
        'pty.write': {
            request: { id: string; data: number[] }
            response: null
        }
        'pty.resize': {
            request: { id: string; columns: number; rows: number }
            response: null
        }
        'pty.kill': {
            request: { id: string; signal: string|null }
            response: null
        }
        'pty.ack': {
            request: { id: string; bytes: number }
            response: null
        }
        'pty.getPid': {
            request: { id: string }
            response: number
        }
        'pty.getTruePid': {
            request: { id: string }
            response: number
        }
        'pty.getChildren': {
            request: { id: string }
            response: ChildProcess[]
        }
        'pty.getCwd': {
            request: { id: string }
            response: string|null
        }
        'sudo.respond': {
            request: { promptId: string }
            response: null
        }
    }

    interface HostEventMap {
        'pty.output': PtyOutputEvent
        'pty.exit': PtyExitEvent
        'pty.error': PtyErrorEvent
        'sudo.prompt': SudoPromptEvent
    }
}
