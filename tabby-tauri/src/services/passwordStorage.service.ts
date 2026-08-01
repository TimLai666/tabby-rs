import { Injectable } from '@angular/core'
import { VaultService } from 'tabby-core'
import {
    PasswordStorageService,
    SSHProfile,
} from 'tabby-ssh'

import { HostBridge } from '../api/hostBridge'
import '../api/keychain'

const VAULT_SECRET_TYPE_PASSWORD = 'ssh:password'
const VAULT_SECRET_TYPE_PASSPHRASE = 'ssh:key-passphrase'

@Injectable({ providedIn: 'root' })
export class TauriPasswordStorageService extends PasswordStorageService {
    constructor (
        private tauriVault: VaultService,
        private bridge: HostBridge,
    ) {
        super(tauriVault)
    }

    override async savePassword (
        profile: SSHProfile,
        password: string,
        username?: string,
    ): Promise<void> {
        const account = username ?? profile.options.user
        if (this.tauriVault.isEnabled()) {
            await this.tauriVault.addSecret({
                type: VAULT_SECRET_TYPE_PASSWORD,
                key: this.vaultConnectionKey(profile, account),
                value: password,
            })
            return
        }
        if (!account) {
            return
        }
        await this.bridge.invoke('keychain.put', {
            service: this.connectionService(profile),
            account,
            value: password,
        })
    }

    override async deletePassword (profile: SSHProfile, username?: string): Promise<void> {
        const account = username ?? profile.options.user
        if (this.tauriVault.isEnabled()) {
            await this.tauriVault.removeSecret(
                VAULT_SECRET_TYPE_PASSWORD,
                this.vaultConnectionKey(profile, account),
            )
            return
        }
        if (!account) {
            return
        }
        await this.bridge.invoke('keychain.delete', {
            service: this.connectionService(profile),
            account,
        })
    }

    override async loadPassword (
        profile: SSHProfile,
        username?: string,
    ): Promise<string|null> {
        const account = username ?? profile.options.user
        if (this.tauriVault.isEnabled()) {
            return (await this.tauriVault.getSecret(
                VAULT_SECRET_TYPE_PASSWORD,
                this.vaultConnectionKey(profile, account),
            ))?.value ?? null
        }
        if (!account) {
            return null
        }
        try {
            return await this.bridge.invoke('keychain.get', {
                service: this.connectionService(profile),
                account,
            })
        } catch {
            return null
        }
    }

    override async savePrivateKeyPassword (id: string, password: string): Promise<void> {
        if (this.tauriVault.isEnabled()) {
            await this.tauriVault.addSecret({
                type: VAULT_SECRET_TYPE_PASSPHRASE,
                key: { hash: id },
                value: password,
            })
            return
        }
        await this.bridge.invoke('keychain.put', {
            service: this.privateKeyService(id),
            account: 'user',
            value: password,
        })
    }

    override async deletePrivateKeyPassword (id: string): Promise<void> {
        if (this.tauriVault.isEnabled()) {
            await this.tauriVault.removeSecret(
                VAULT_SECRET_TYPE_PASSPHRASE,
                { hash: id },
            )
            return
        }
        await this.bridge.invoke('keychain.delete', {
            service: this.privateKeyService(id),
            account: 'user',
        })
    }

    override async loadPrivateKeyPassword (id: string): Promise<string|null> {
        if (this.tauriVault.isEnabled()) {
            return (await this.tauriVault.getSecret(
                VAULT_SECRET_TYPE_PASSPHRASE,
                { hash: id },
            ))?.value ?? null
        }
        try {
            return await this.bridge.invoke('keychain.get', {
                service: this.privateKeyService(id),
                account: 'user',
            })
        } catch {
            return null
        }
    }

    private connectionService (profile: SSHProfile): string {
        const port = profile.options.port
        return port
            ? `ssh@${profile.options.host}:${port}`
            : `ssh@${profile.options.host}`
    }

    private privateKeyService (id: string): string {
        return `ssh-private-key:${id}`
    }

    private vaultConnectionKey (profile: SSHProfile, username?: string) {
        return {
            user: username ?? profile.options.user,
            host: profile.options.host,
            port: profile.options.port,
        }
    }
}
