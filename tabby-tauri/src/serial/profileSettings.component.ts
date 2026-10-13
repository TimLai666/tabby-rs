import { Component, OnDestroy } from '@angular/core'
import { FullyDefined, ProfileSettingsComponent } from 'tabby-core'

import { HostBridge, SerialPortInfo } from '../api/hostBridge'
import { TauriSerialProfile } from './profile'
import { TauriSerialProfilesService } from './profiles'

@Component({
    templateUrl: './profileSettings.component.pug',
})
export class TauriSerialProfileSettingsComponent implements ProfileSettingsComponent<TauriSerialProfile, TauriSerialProfilesService>, OnDestroy {
    profile: FullyDefined<TauriSerialProfile>
    ports: SerialPortInfo[] = []
    private stopListening: (() => void)|null = null
    private destroyed = false

    constructor (private bridge: HostBridge) { }

    async ngOnInit (): Promise<void> {
        try {
            const ports = await this.bridge.invoke('serial.list', {})
            if (this.destroyed) {
                return
            }
            this.ports = ports
        } catch {
            if (this.destroyed) {
                return
            }
            this.ports = []
        }
        const stopListening = await this.bridge.listen('serial:portsChanged', ports => {
            if (this.destroyed) {
                return
            }
            this.ports = ports
        })
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroyed) {
            stopListening()
        } else {
            this.stopListening = stopListening
        }
    }

    ngOnDestroy (): void {
        this.destroyed = true
        this.stopListening?.()
        this.stopListening = null
    }
}
