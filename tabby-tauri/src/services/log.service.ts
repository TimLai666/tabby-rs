import { Injectable } from '@angular/core'
import { ConsoleLogger, Logger, LogService } from 'tabby-core'

import { DiagnosticsAppendRequest } from '../api/hostBridge'
import { TauriHostBridge } from './tauriHostBridge.service'

function formatMessage (value: unknown): string {
    if (value instanceof Error) {
        return `${value.name}: ${value.message}${value.stack ? `\n${value.stack}` : ''}`
    }
    if (typeof value === 'string') {
        return value.slice(0, 120 * 1024)
    }
    if (value === undefined) {
        return ''
    }
    try {
        return String(JSON.stringify(value)).slice(0, 120 * 1024)
    } catch {
        return String(value).slice(0, 120 * 1024)
    }
}

class TauriConsoleLogger extends ConsoleLogger {
    // ConsoleLogger exposes a protected constructor; this subclass intentionally
    // makes it public for the host LogService factory.
    // eslint-disable-next-line @typescript-eslint/no-useless-constructor
    constructor (name: string, private bridge: TauriHostBridge) {
        super(name)
    }

    protected doLog (level: string, ...args: any[]): void {
        super.doLog(level, ...args)
        const message = formatMessage(args[0])
        const request: DiagnosticsAppendRequest = {
            level,
            source: this.name,
            message,
            fields: {
                argumentCount: args.length,
                additionalArgumentTypes: args.slice(1).map(value => typeof value),
            },
        }
        void this.bridge.invoke('diagnostics.append', request).catch(() => undefined)
    }
}

@Injectable()
export class TauriLogService extends LogService {
    constructor (private bridge: TauriHostBridge) {
        super()
    }

    create (name: string): Logger {
        return new TauriConsoleLogger(name, this.bridge)
    }
}
