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
            slowSend: false,
            readTimeoutMs: 250,
            reconnect: { enabled: false, maxAttempts: 5, maxDelayMs: 30_000 },
            inputMode: null,
            outputMode: null,
            inputNewlines: null,
            outputNewlines: null,
            maxInputLineLength: 64 * 1024,
            preserveOutputHexdumpOffset: true,
            scripts: [],
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

    // Mirrors upstream tabby-serial c9a7a54f: `port@baud` or `port:baud`,
    // an unparsable rate falls back to 115200.
    quickConnect (query: string): PartialProfile<TauriSerialProfile> {
        let port = query
        let baudRate = 115200
        if (query.includes('@')) {
            baudRate = parseInt(port.split('@')[1]) || baudRate
            port = port.split('@')[0]
        } else if (query.includes(':')) {
            baudRate = parseInt(port.split(':')[1]) || baudRate
            port = port.split(':')[0]
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
        // Like upstream, the default rate renders bare and parses back to 115200.
        const { port, baudRate } = profile.options
        return baudRate && baudRate !== 115200 ? `${port}@${baudRate}` : port
    }
}
