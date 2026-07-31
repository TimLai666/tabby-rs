import { Injectable } from '@angular/core'
import { ConsoleLogger, Logger, LogService } from 'tabby-core'

@Injectable()
export class TauriLogService extends LogService {
    create (name: string): Logger {
        return new ConsoleLogger(name)
    }
}
