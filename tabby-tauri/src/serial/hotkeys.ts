import { Injectable } from '@angular/core'
import { HotkeyDescription, HotkeyProvider, TranslateService } from 'tabby-core'

@Injectable()
export class TauriSerialHotkeyProvider extends HotkeyProvider {
    constructor (private translate: TranslateService) { super() }

    async provide (): Promise<HotkeyDescription[]> {
        return [{
            id: 'restart-serial-session',
            name: this.translate.instant('Restart current serial session'),
        }]
    }
}
