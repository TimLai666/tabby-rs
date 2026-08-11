import { Injectable } from '@angular/core'
import { NewTabParameters, PartialProfile, QuickConnectProfileProvider, TranslateService } from 'tabby-core'

import { TauriTelnetProfile } from './profile'
import { TauriTelnetProfileSettingsComponent } from './profileSettings.component'
import { TauriTelnetTabComponent } from './tab.component'

@Injectable({ providedIn: 'root' })
export class TauriTelnetProfilesService extends QuickConnectProfileProvider<TauriTelnetProfile> {
    id = 'telnet'
    name = 'Telnet'
    settingsComponent = TauriTelnetProfileSettingsComponent
    configDefaults = {
        options: {
            host: '',
            port: 23,
            terminalType: 'xterm-256color',
            encoding: 'utf-8',
            connectTimeoutMs: 10_000,
            keepaliveInterval: 0,
            keepaliveCountMax: 3,
            inputMode: 'local-echo',
            outputMode: null,
            inputNewlines: null,
            outputNewlines: 'crlf',
            input: { backspace: 'backspace' },
        },
        clearServiceMessagesOnConnect: false,
    }

    constructor (private translate: TranslateService) { super() }

    async getBuiltinProfiles (): Promise<PartialProfile<TauriTelnetProfile>[]> {
        return [
            {
                id: 'telnet:template',
                type: 'telnet',
                name: this.translate.instant('Telnet session'),
                icon: 'fas fa-network-wired',
                options: { host: '', port: 23, terminalType: 'xterm-256color', encoding: 'utf-8' },
                isBuiltin: true,
                isTemplate: true,
            },
            {
                id: 'socket:template',
                type: 'telnet',
                name: this.translate.instant('Raw socket connection'),
                icon: 'fas fa-network-wired',
                options: { host: '', port: 1234, terminalType: 'xterm-256color', encoding: 'utf-8' },
                isBuiltin: true,
                isTemplate: true,
            },
        ]
    }

    async getNewTabParameters (profile: TauriTelnetProfile): Promise<NewTabParameters<TauriTelnetTabComponent>> {
        return { type: TauriTelnetTabComponent, inputs: { profile } }
    }

    getSuggestedName (profile: TauriTelnetProfile): string|null {
        return this.getDescription(profile) || null
    }

    getDescription (profile: PartialProfile<TauriTelnetProfile>): string {
        return profile.options?.host ? `${profile.options.host}:${profile.options.port ?? 23}` : ''
    }

    quickConnect (query: string): PartialProfile<TauriTelnetProfile> {
        let host = query.trim()
        let port = 23
        const bracketed = /^\[([^\]]+)\](?::(\d+))?$/.exec(host)
        const plain = /^([^:]+)(?::(\d+))?$/.exec(host)
        const match = bracketed ?? plain
        if (match) {
            host = match[1]
            port = Number(match[2]) || 23
        }
        return {
            name: query,
            type: 'telnet',
            options: { host, port, terminalType: 'xterm-256color', encoding: 'utf-8', inputMode: 'readline' },
        }
    }

    intoQuickConnectString (profile: TauriTelnetProfile): string|null {
        let value = profile.options.host
        if ((profile.options.port ?? 23) !== 23) {
            value = `${value}:${profile.options.port}`
        }
        return value
    }
}
