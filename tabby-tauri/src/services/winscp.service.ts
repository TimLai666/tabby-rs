import { Injectable } from '@angular/core'
import { ConfigService, FileProvidersService, HostAppService, Platform, PlatformService } from 'tabby-core'

import type { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'
import { PasswordStorageService } from '../../../tabby-ssh/src/services/passwordStorage.service'
import { HostBridge, WinSCPConnectionOptions, WinSCPKeyInput } from '../api/hostBridge'
import type { TauriSshSession } from '../ssh/session'

/**
 * Passphrase assumed for a key whose passphrase was never stored.
 *
 * WinSCP would otherwise stop on its own passphrase prompt, which nothing can
 * answer once the session is handed to a running WinSCP instance.
 */
const DEFAULT_PASSPHRASE = 'tabby'

/**
 * The part of a session the original service reads its connection from.
 *
 * `TauriSshSession` keeps its profile private, and the profile decides the
 * password, the port, the jump host and the key candidates, so it is taken
 * through this one cast rather than by copying every session field.
 */
interface SessionProfile { profile: SSHProfile }

/**
 * Opens a WinSCP session for an SSH session, following
 * `tabby-ssh/src/services/ssh.service.ts`.
 *
 * Keys are converted through separate bridge calls, stopping at the first
 * accepted key. The final call starts WinSCP with the prepared content.
 */
@Injectable({ providedIn: 'root' })
export class TauriWinSCPService {
    private readonly windows: boolean
    private detectedWinSCPPath: string|null = null

    constructor (
        private passwordStorage: PasswordStorageService,
        private config: ConfigService,
        hostApp: HostAppService,
        platform: PlatformService,
        private fileProviders: FileProvidersService,
        private bridge: HostBridge,
    ) {
        this.windows = hostApp.platform === Platform.Windows
        if (this.windows) {
            this.detectedWinSCPPath = platform.getWinSCPPath()
        }
    }

    getWinSCPPath (): string|null {
        return this.detectedWinSCPPath ?? this.config.store.ssh.winSCPPath ?? null
    }

    async launchWinSCP (session: TauriSshSession): Promise<void> {
        const executable = this.getWinSCPPath()
        if (!this.windows || !executable) {
            return
        }
        const profile = (session as unknown as SessionProfile).profile
        const target: WinSCPConnectionOptions = {
            host: profile.options.host,
            port: profile.options.port ?? 22,
            username: session.authUsername ?? profile.options.user,
            password: await this.passwordStorage.loadPassword(profile, session.authUsername ?? undefined),
            privateKey: null,
        }
        const jumpProfile = profile.options.jumpHost
            ? this.config.store.profiles.find(candidate => candidate.id === profile.options.jumpHost) ?? null
            : null
        const jump = jumpProfile ? await this.jumpOptions(executable, jumpProfile) : null
        // Prepare the jump host's content before trying the target's keys.
        if (session.activePrivateKey && profile.options.privateKeys.length > 0) {
            target.privateKey = await this.convertFirstKey(executable, profile.options.privateKeys)
        }
        await this.bridge.invoke('winscp.launch', { executable, target, jump })
    }

    private async jumpOptions (executable: string, profile: SSHProfile): Promise<WinSCPConnectionOptions> {
        const jump: WinSCPConnectionOptions = {
            host: profile.options.host,
            port: profile.options.port ?? 22,
            username: profile.options.user,
            password: null,
            privateKey: null,
        }
        if (profile.options.auth === 'password') {
            jump.password = await this.passwordStorage.loadPassword(profile)
        }
        if (profile.options.auth === 'publicKey' && profile.options.privateKeys.length > 0) {
            jump.privateKey = await this.convertFirstKey(executable, profile.options.privateKeys)
        }
        return jump
    }

    /**
     * Converts the keys in order and returns the first one WinSCP accepted.
     *
     * A rejected key is dropped without a reason, exactly as the sequential
     * conversion of the original service does, so the next file is the only
     * thing that can be tried. `null` means no key is offered at all, which
     * leaves WinSCP to ask for one interactively.
     */
    private async convertFirstKey (executable: string, refs: string[]): Promise<WinSCPKeyInput|null> {
        for (const ref of refs) {
            // Native SSH profiles store paths; file providers use URI references.
            const reference = ref.includes('://') ? ref : `file://${ref}`
            const content = (await this.fileProviders.retrieveFile(reference)).toString()
            const hash = await this.keyHash(content)
            const passphrase = await this.passwordStorage.loadPrivateKeyPassword(hash) ?? DEFAULT_PASSPHRASE
            const converted = await this.bridge.invoke('winscp.convertKey', { executable, key: { content, passphrase } })
            if (converted) {
                return converted
            }
        }
        return null
    }

    private async keyHash (content: string): Promise<string> {
        const digest = await window.crypto.subtle.digest('SHA-512', new TextEncoder().encode(content))
        return Array.from(new Uint8Array(digest)).map(byte => byte.toString(16).padStart(2, '0')).join('')
    }
}
