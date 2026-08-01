import { Injectable } from '@angular/core'
import { SSHProfile } from '../api'

export const VAULT_SECRET_TYPE_PASSWORD = 'ssh:password'
export const VAULT_SECRET_TYPE_PASSPHRASE = 'ssh:key-passphrase'

/**
 * Host-provided secret storage contract.
 *
 * Electron binds this token to its keytar implementation. Tauri binds the same
 * token to the Rust keychain and Vault bridge, keeping SSH callers and plugins
 * independent from the desktop host.
 */
@Injectable()
export abstract class PasswordStorageService {
    abstract savePassword (
        profile: SSHProfile,
        password: string,
        username?: string,
    ): Promise<void>

    abstract deletePassword (profile: SSHProfile, username?: string): Promise<void>

    abstract loadPassword (profile: SSHProfile, username?: string): Promise<string|null>

    abstract savePrivateKeyPassword (id: string, password: string): Promise<void>

    abstract deletePrivateKeyPassword (id: string): Promise<void>

    abstract loadPrivateKeyPassword (id: string): Promise<string|null>
}
