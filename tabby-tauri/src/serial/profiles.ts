import { Injectable } from '@angular/core'
import { NewTabParameters, PartialProfile, QuickConnectProfileProvider, TranslateService } from 'tabby-core'

import { TauriSerialProfile } from './profile'
import { TauriSerialProfileSettingsComponent } from './profileSettings.component'
import { TauriSerialTabComponent } from './tab.component'

@Injectable({ providedIn: 'root' })
export class TauriSerialProfilesService extends QuickConnectProfileProvider<TauriSerialProfile> {
    id = 'serial'
    name = 'Serial'
    settingsComponent = TauriSerialProfileSettingsComponent
    configDefaults = {
        options: {
            port: null,
            baudRate: 115200,
            dataBits: 8,
            stopBits: 1,
            parity: 'none',
            flowControl: 'none',
            readTimeoutMs: 250,
            reconnect: { enabled: false, maxAttempts: 5, maxDelayMs: 30_000 },
            inputMode: null,
            outputMode: null,
            inputNewlines: null,
            outputNewlines: null,
            maxInputLineLength: 64 * 1024,
            preserveOutputHexdumpOffset: true,
            input: { backspace: 'backspace' },
        },
        clearServiceMessagesOnConnect: false,
    }

    constructor (private translate: TranslateService) { super() }

    async getBuiltinProfiles (): Promise<PartialProfile<TauriSerialProfile>[]> {
        return [{
            id: 'serial:template',
            type: 'serial',
            name: this.translate.instant('Serial connection'),
            icon: 'fas fa-microchip',
            isBuiltin: true,
            isTemplate: true,
        }]
    }

    async getNewTabParameters (profile: TauriSerialProfile): Promise<NewTabParameters<TauriSerialTabComponent>> {
        return { type: TauriSerialTabComponent, inputs: { profile } }
    }

    getSuggestedName (profile: TauriSerialProfile): string|null {
        return this.getDescription(profile) || null
    }

    getDescription (profile: PartialProfile<TauriSerialProfile>): string {
        return profile.options?.port ?? ''
    }

    quickConnect (query: string): PartialProfile<TauriSerialProfile> {
        let port = query.trim()
        let baudRate = 115200
        const separator = port.lastIndexOf('@')
        if (separator > 0) {
            const parsed = Number(port.slice(separator + 1))
            if (Number.isInteger(parsed) && parsed > 0) {
                baudRate = parsed
                port = port.slice(0, separator)
            }
        }
        return {
            name: query,
            type: 'serial',
            options: { port, baudRate },
        }
    }

    intoQuickConnectString (profile: TauriSerialProfile): string|null {
        if (!profile.options.port) {
            return null
        }
        return `${profile.options.port}@${profile.options.baudRate ?? 115200}`
    }
}
