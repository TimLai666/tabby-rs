import { Injectable } from '@angular/core'
import { ConsoleLogger, Logger, LogService } from 'tabby-core'

class TauriConsoleLogger extends ConsoleLogger {
    // ConsoleLogger exposes a protected constructor; this subclass intentionally
    // makes it public for the host LogService factory.
    // eslint-disable-next-line @typescript-eslint/no-useless-constructor
    constructor (name: string) {
        super(name)
    }
}

@Injectable()
export class TauriLogService extends LogService {
    create (name: string): Logger {
        return new TauriConsoleLogger(name)
    }
}
