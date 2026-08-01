import { Injectable } from '@angular/core'
import * as keytar from 'keytar'
import { VaultService } from 'tabby-core'
import {
    PasswordStorageService,
    SSHProfile,
    VAULT_SECRET_TYPE_PASSWORD,
    VAULT_SECRET_TYPE_PASSPHRASE,
} from 'tabby-ssh'

@Injectable()
export class ElectronPasswordStorageService extends PasswordStorageService {
    constructor (private vault: VaultService) {
        super()
    }

    async savePassword (
        profile: SSHProfile,
        password: string,
        username?: string,
    ): Promise<void> {
        const account = username ?? profile.options.user
        if (this.vault.isEnabled()) {
            await this.vault.addSecret({
                type: VAULT_SECRET_TYPE_PASSWORD,
                key: this.getVaultKeyForConnection(profile, account),
                value: password,
            })
            return
        }
        if (!account) {
            return
        }
        await keytar.setPassword(this.getKeytarKeyForConnection(profile), account, password)
    }

    async deletePassword (profile: SSHProfile, username?: string): Promise<void> {
        const account = username ?? profile.options.user
        if (this.vault.isEnabled()) {
            await this.vault.removeSecret(
                VAULT_SECRET_TYPE_PASSWORD,
                this.getVaultKeyForConnection(profile, account),
            )
            return
        }
        if (!account) {
            return
        }
        await keytar.deletePassword(this.getKeytarKeyForConnection(profile), account)
    }

    async loadPassword (
        profile: SSHProfile,
        username?: string,
    ): Promise<string|null> {
        const account = username ?? profile.options.user
        if (this.vault.isEnabled()) {
            return (await this.vault.getSecret(
                VAULT_SECRET_TYPE_PASSWORD,
                this.getVaultKeyForConnection(profile, account),
            ))?.value ?? null
        }
        if (!account) {
            return null
        }
        try {
            return await keytar.getPassword(this.getKeytarKeyForConnection(profile), account)
        } catch (error) {
            console.warn(
                `Failed to load stored password for ${account}@${profile.options.host}:${profile.options.port ?? 22}`,
                error,
            )
            return null
        }
    }

    async savePrivateKeyPassword (id: string, password: string): Promise<void> {
        if (this.vault.isEnabled()) {
            await this.vault.addSecret({
                type: VAULT_SECRET_TYPE_PASSPHRASE,
                key: { hash: id },
                value: password,
            })
            return
        }
        await keytar.setPassword(this.getKeytarKeyForPrivateKey(id), 'user', password)
    }

    async deletePrivateKeyPassword (id: string): Promise<void> {
        if (this.vault.isEnabled()) {
            await this.vault.removeSecret(VAULT_SECRET_TYPE_PASSPHRASE, { hash: id })
            return
        }
        await keytar.deletePassword(this.getKeytarKeyForPrivateKey(id), 'user')
    }

    async loadPrivateKeyPassword (id: string): Promise<string|null> {
        if (this.vault.isEnabled()) {
            return (await this.vault.getSecret(
                VAULT_SECRET_TYPE_PASSPHRASE,
                { hash: id },
            ))?.value ?? null
        }
        return keytar.getPassword(this.getKeytarKeyForPrivateKey(id), 'user')
    }

    private getKeytarKeyForConnection (profile: SSHProfile): string {
        return profile.options.port
            ? `ssh@${profile.options.host}:${profile.options.port}`
            : `ssh@${profile.options.host}`
    }

    private getKeytarKeyForPrivateKey (id: string): string {
        return `ssh-private-key:${id}`
    }

    private getVaultKeyForConnection (profile: SSHProfile, username?: string) {
        return {
            user: username ?? profile.options.user,
            host: profile.options.host,
            port: profile.options.port,
        }
    }
}
