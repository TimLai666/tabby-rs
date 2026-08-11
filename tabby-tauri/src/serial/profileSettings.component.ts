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

    constructor (private bridge: HostBridge) { }

    async ngOnInit (): Promise<void> {
        try {
            this.ports = await this.bridge.invoke('serial.list', {})
        } catch {
            this.ports = []
        }
        this.stopListening = await this.bridge.listen('serial.portsChanged', ports => {
            this.ports = ports
        })
    }

    ngOnDestroy (): void {
        this.stopListening?.()
        this.stopListening = null
    }
}
