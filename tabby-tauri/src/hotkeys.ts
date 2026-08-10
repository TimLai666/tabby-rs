import { Injectable } from '@angular/core'
import { HotkeyDescription, HotkeyProvider } from 'tabby-core'

@Injectable()
export class TauriHotkeyProvider extends HotkeyProvider {
    async provide (): Promise<HotkeyDescription[]> {
        return [
            {
                id: 'new-window',
                name: 'New window',
            },
            {
                id: 'toggle-window',
                name: 'Show or hide window',
            },
        ]
    }
}
