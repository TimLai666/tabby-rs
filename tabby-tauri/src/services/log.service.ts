import { Injectable } from '@angular/core'
import { ConsoleLogger, Logger, LogService } from 'tabby-core'

class TauriConsoleLogger extends ConsoleLogger {
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
