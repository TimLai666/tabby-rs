import { Injectable } from '@angular/core'
import { NewTabParameters, PartialProfile, QuickConnectProfileProvider, TranslateService } from 'tabby-core'
import { PasswordStorageService } from '../../../tabby-ssh/src/services/passwordStorage.service'
import { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'

import { TauriSshProfileSettingsComponent } from './profileSettings.component'
import { TauriSshTabComponent } from './tab.component'
import { HostBridge } from '../api/hostBridge'

@Injectable({ providedIn: 'root' })
export class TauriSshProfilesService extends QuickConnectProfileProvider<SSHProfile> {
    id = 'ssh'
    name = 'SSH'
    settingsComponent = TauriSshProfileSettingsComponent
    configDefaults = {
        options: {
            host: '', port: 22, user: 'root', auth: null, password: null, privateKeys: [],
            keepaliveInterval: 5000, keepaliveCountMax: 10, readyTimeout: null,
            x11: false, skipBanner: false, jumpHost: null, agentForward: false,
            warnOnClose: null, algorithms: { hmac: [], kex: [], cipher: [], serverHostKey: [], compression: [] },
            proxyCommand: null, forwardedPorts: [], scripts: [], socksProxyHost: null,
            socksProxyPort: null, httpProxyHost: null, httpProxyPort: null,
            reuseSession: true, input: { backspace: 'backspace' },
        },
        clearServiceMessagesOnConnect: true,
    }

    constructor (
        private translate: TranslateService,
        private passwordStorage: PasswordStorageService,
        private bridge: HostBridge,
    ) {
        super()
    }

    async getBuiltinProfiles (): Promise<PartialProfile<SSHProfile>[]> {
        const template = {
            id: 'ssh:template',
            type: 'ssh',
            name: this.translate.instant('SSH connection'),
            icon: 'fas fa-desktop',
            options: { host: '', port: 22, user: 'root' },
            isBuiltin: true,
            isTemplate: true,
            weight: -1,
        }
        try {
            const preview = await this.bridge.invoke('ssh.importPreview', { path: '~/.ssh/config' })
            return [template, ...preview.profiles.map(profile => ({
                id: profile.id,
                name: profile.name,
                type: 'ssh',
                group: 'Imported from .ssh/config',
                isBuiltin: true,
                options: {
                    host: profile.host,
                    port: profile.port,
                    user: profile.user ?? 'root',
                    auth: profile.privateKeys.length > 0 ? 'publicKey' as const : null,
                    privateKeys: profile.privateKeys,
                },
            }))]
        } catch {
            return [template]
        }
    }

    async getNewTabParameters (profile: SSHProfile): Promise<NewTabParameters<TauriSshTabComponent>> {
        return { type: TauriSshTabComponent, inputs: { profile } }
    }

    getSuggestedName (profile: SSHProfile): string {
        return `${profile.options.user}@${profile.options.host}:${profile.options.port ?? 22}`
    }

    getDescription (profile: PartialProfile<SSHProfile>): string {
        return profile.options?.host ?? ''
    }

    deleteProfile (profile: SSHProfile): void {
        void this.passwordStorage.deletePassword(profile)
    }

    quickConnect (query: string): PartialProfile<SSHProfile> {
        let user: string|undefined = undefined
        let host = query.trim()
        let port = 22
        const at = host.lastIndexOf('@')
        if (at >= 0) {
            user = host.substring(0, at)
            host = host.substring(at + 1)
        }
        const match = /^\[([^\]]+)\]:(\d+)$/.exec(host) ?? /^([^:]+):(\d+)$/.exec(host)
        if (match) {
            host = match[1]
            port = Number(match[2]) || 22
        }
        return { name: query, type: 'ssh', options: { host, user, port } }
    }

    intoQuickConnectString (profile: SSHProfile): string|null {
        let value = profile.options.host
        if (profile.options.user && profile.options.user !== 'root') {
            value = `${profile.options.user}@${value}`
        }
        if ((profile.options.port ?? 22) !== 22) {
            value = `${value}:${profile.options.port}`
        }
        return value
    }
}
