import { Injectable } from '@angular/core'

import { HostBridge } from '../api/hostBridge'

@Injectable()
export class TauriNotificationsService {
    constructor (private bridge: HostBridge) { }

    notice (text: string): void {
        this.show('Tabby RS', text)
    }

    info (text: string, details?: string): void {
        this.show(text, details)
    }

    error (text: string, details?: string): void {
        this.show(text, details)
    }

    private show (title: string, body?: string): void {
        void this.bridge.invoke('notification.show', {
            title,
            body: body ?? null,
        }).catch(error => console.info('Desktop notification is unavailable:', error))
    }
}
