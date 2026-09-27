import { Injector } from '@angular/core'
import { NgbModal, NgbModalRef } from '@ng-bootstrap/ng-bootstrap'
import { Observable, Subject } from 'rxjs'
import { ConfigService, LogService, ProfilesService, PromptModalComponent, VaultService } from 'tabby-core'
import { BaseSession, InputProcessor, UTF8SplitterMiddleware } from 'tabby-terminal'

import { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'
import {
    HostBridge,
    SshAuthPrompt,
    SshAuthMethodRef,
    SshConnectRequest,
    SshConnectError,
    SshExitEvent,
    SshHostKeyPrompt,
    SshJumpRequest,
} from '../api/hostBridge'
import { TauriSshHostKeyPromptModalComponent } from './hostKeyPromptModal.component'
import { TauriSftpSession } from './sftp'
import { TauriPasswordStorageService } from '../services/passwordStorage.service'

export class TauriSshSession extends BaseSession {
    private id: string|null = null
    private destroying = false
    private connecting = false
    authUsername: string|null = null
    activePrivateKey = false
    private readonly connectionId = window.crypto.randomUUID()
    private pendingOutput: { data: number[]; extended: boolean }[] = []
    private pendingExit: SshExitEvent|null = null
    private unlisteners: (() => void)[] = []
    private readonly authPrompt = new Subject<SshAuthPrompt>()
    private readonly serviceMessage = new Subject<string>()
    private forwardingIds: string[] = []
    private sftp: TauriSftpSession|null = null
    private credentialModals = new Map<string, NgbModalRef|null>()
    private hostKeyModals = new Set<NgbModalRef>()
    private pendingPasswords = new Map<string, { profile: SSHProfile; value: string; username: string }>()
    private pendingPassphrases = new Map<string, { hash: string; value: string }>()

    get authPrompt$ (): Observable<SshAuthPrompt> {
        return this.authPrompt.asObservable()
    }

    get isClosing (): boolean {
        return this.destroying
    }

    get serviceMessage$ (): Observable<string> {
        return this.serviceMessage.asObservable()
    }

    constructor (
        private injector: Injector,
        private bridge: HostBridge,
        private vault: VaultService,
        private profile: SSHProfile,
        private modals: NgbModal,
    ) {
        super(injector.get(LogService).create(`ssh-tauri-${profile.options.host}-${profile.options.port ?? 22}`))
        this.setLoginScriptsOptions(profile.options)
        this.middleware.push(new UTF8SplitterMiddleware())
        this.middleware.push(new InputProcessor(profile.options.input))
    }

    async start (): Promise<void> {
        if (this.open || this.destroying) {
            return
        }
        const unlisteners = await Promise.all([
            this.bridge.listen('ssh:output', event => {
                if (event.connectionId === this.connectionId) {
                    if (!this.id) {
                        this.pendingOutput.push({ data: event.data, extended: event.extended })
                        return
                    }
                    this.emitOutput(Buffer.from(event.data))
                }
            }),
            this.bridge.listen('ssh:exit', event => {
                if (event.connectionId === this.connectionId) {
                    if (!this.id) {
                        this.pendingExit = event
                    } else if (this.open) {
                        this.reportExit(event)
                        void this.destroy()
                    }
                }
            }),
            this.bridge.listen('ssh:hostKeyPrompt', prompt => {
                if (prompt.connectionId === this.connectionId) {
                    void this.handleHostKeyPrompt(prompt)
                }
            }),
            this.bridge.listen('ssh:authPrompt', prompt => {
                if (prompt.connectionId === this.connectionId) {
                    if (prompt.username === true || Boolean(prompt.password ?? prompt.privateKeyHash)) {
                        void this.handleCredentialPrompt(prompt)
                    } else {
                        this.authPrompt.next(prompt)
                    }
                }
            }),
            this.bridge.listen('ssh:passwordAccepted', event => {
                if (event.connectionId === this.connectionId) {
                    void this.saveAcceptedPassword(event.requestId)
                }
            }),
            this.bridge.listen('ssh:privateKeyUnlocked', event => {
                if (event.connectionId === this.connectionId) {
                    void this.saveUnlockedPassphrase(event.requestId)
                }
            }),
        ])
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) {
            for (const unlisten of unlisteners) { unlisten() }
            return
        }
        this.unlisteners.push(...unlisteners)

        const request = await this.connectRequest()
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) { return }
        const unlistenConnecting = await this.bridge.listen('ssh:connecting', event => {
            if (event.connectionId === this.connectionId && this.destroying) {
                void this.cancelPendingConnection()
            }
        })
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) {
            unlistenConnecting()
            return
        }

        this.connecting = true
        const info = await this.bridge.invoke('ssh.connect', request).catch(error => {
            this.clearCredentialPrompts()
            void this.removeRejectedPassword(error)
            throw error
        }).finally(() => {
            this.connecting = false
            unlistenConnecting()
        })
        // The session can be destroyed while the bridge connection is still opening.
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) {
            await this.bridge.invoke('ssh.close', { id: info.id }).catch(error => {
                this.logger.debug('SSH close failed after cancelled start', error)
            })
            return
        }
        this.authUsername = info.username
        this.activePrivateKey = info.usedPrivateKey ?? false
        this.id = info.id
        this.open = true
        try {
            await this.startForwardings(info.id)
        } catch (error) {
            await this.destroy()
            throw error
        }
        for (const output of this.pendingOutput.splice(0)) {
            this.emitOutput(Buffer.from(output.data))
        }
        if (this.pendingExit) {
            this.reportExit(this.pendingExit)
            this.pendingExit = null
            void this.destroy()
            return
        }
        this.loginScriptProcessor?.executeUnconditionalScripts()
    }

    resize (columns: number, rows: number): void {
        if (!this.id) {
            return
        }
        void this.bridge.invoke('ssh.resize', {
            id: this.id,
            columns,
            rows,
            pixelWidth: null,
            pixelHeight: null,
        }).catch(error => this.logger.warn('SSH resize failed', error))
    }

    write (data: Buffer): void {
        if (!this.id || data.length === 0) {
            return
        }
        void this.bridge.invoke('ssh.write', {
            id: this.id,
            data: Array.from(data),
        }).catch(error => this.logger.warn('SSH write failed', error))
    }

    kill (_signal?: string): void {
        void this.destroy()
    }

    async destroy (): Promise<void> {
        if (this.destroying) {
            return
        }
        this.destroying = true
        this.clearCredentialPrompts()
        for (const modal of this.hostKeyModals) { modal.dismiss() }
        this.hostKeyModals.clear()
        if (this.connecting) { await this.cancelPendingConnection() }
        const id = this.id
        this.id = null
        // Keep the last authenticated username and key flag for launching file transfers
        // from a disconnected tab, matching upstream.
        if (id) {
            await this.sftp?.close().catch(error => this.logger.debug('SFTP close failed after session end', error))
            this.sftp = null
            await Promise.all(this.forwardingIds.splice(0).map(forwardingId => this.bridge.invoke('ssh.forwardingStop', {
                id: forwardingId,
            }).catch(error => this.logger.debug('SSH forwarding close failed', error))))
            await this.bridge.invoke('ssh.close', { id }).catch(error => {
                this.logger.debug('SSH close failed after session end', error)
            })
        }
        for (const unlisten of this.unlisteners.splice(0)) {
            unlisten()
        }
        this.authPrompt.complete()
        this.serviceMessage.complete()
        await super.destroy()
    }

    async gracefullyKillProcess (): Promise<void> {
        await this.destroy()
    }

    supportsWorkingDirectory (): boolean {
        return !!this.reportedCWD
    }

    async getWorkingDirectory (): Promise<string|null> {
        return this.reportedCWD ?? null
    }

    private reportExit (event: SshExitEvent): void {
        if (event.exitCode !== null) {
            this.serviceMessage.next(`SSH session exited with code ${event.exitCode}`)
        } else if (event.signal !== null) {
            this.serviceMessage.next(`SSH session terminated by ${event.signal}`)
        }
    }

    async openSFTP (): Promise<TauriSftpSession> {
        if (!this.id || !this.open) {
            throw new Error('SSH session is not open')
        }
        if (!this.sftp) {
            this.sftp = await TauriSftpSession.open(this.bridge, this.id)
        }
        return this.sftp
    }

    private async resolveAgentSocket (): Promise<string|null> {
        const configService = this.injector.get(ConfigService)
        const agentType = configService.store.ssh.agentType ?? null
        const agentPath = configService.store.ssh.agentPath ?? null
        return this.bridge.invoke('ssh.resolveAgentSocket', { agentType, agentPath })
    }

    private async connectRequest (): Promise<SshConnectRequest> {
        const options = this.profile.options
        const auth = await this.authForOptions(options)
        let agentForwarding: SshConnectRequest['agentForwarding'] = null
        if (options.agentForward) {
            const existingAgent = auth.find(a => a.type === 'agent')
            const socket = existingAgent?.type === 'agent'
                ? existingAgent.socket ?? null
                : await this.resolveAgentSocket()
            agentForwarding = { socket }
        }
        return {
            profileId: this.profile.id,
            connectionId: this.connectionId,
            host: options.host,
            port: options.port ?? 22,
            username: options.user || null,
            auth,
            terminal: {
                term: 'xterm-256color',
                columns: 80,
                rows: 30,
                pixelWidth: null,
                pixelHeight: null,
            },
            keepalive: options.keepaliveInterval > 0 ? {
                intervalMs: options.keepaliveInterval,
                maxCount: options.keepaliveCountMax,
            } : null,
            environment: options.environment ?? {},
            x11: !!options.x11,
            x11Display: this.injector.get(ConfigService).store.ssh.x11Display || null,
            agentForward: !!options.agentForward,
            agentForwarding,
            jumpChain: await this.jumpChain(options.jumpHost),
        }
    }

    private async authForOptions (options: SSHProfile['options']): Promise<SshAuthMethodRef[]> {
        const auth: SshAuthMethodRef[] = []
        const authMode = String(options.auth ?? '')
        if (!options.auth) {
            const privateKeys = options.privateKeys.length
                ? options.privateKeys
                : await this.bridge.invoke('ssh.listPrivateKeys', {})
            for (const fileRef of privateKeys) {
                auth.push({ type: 'privateKey', fileRef, passphraseRef: null })
            }
            const socket = await this.resolveAgentSocket()
            auth.push({ type: 'agent', socket })
            if (options.password) {
                auth.push({ type: 'providedPassword', password: options.password })
            }
            if (options.password) {
                auth.push({ type: 'keyboardInteractive', password: options.password })
            }
            auth.push({ type: 'keyboardInteractive', secretRef: this.passwordSecretRef() })
            auth.push({ type: 'password', secretRef: this.passwordSecretRef() })
            auth.push({ type: 'promptPassword' })
        } else if (options.auth === 'password') {
            if (options.password) {
                auth.push({ type: 'providedPassword', password: options.password })
            }
            auth.push({ type: 'password', secretRef: this.passwordSecretRef() })
            auth.push({ type: 'promptPassword' })
        } else if (options.auth === 'publicKey') {
            const privateKeys = options.privateKeys.length
                ? options.privateKeys
                : await this.bridge.invoke('ssh.listPrivateKeys', {})
            for (const fileRef of privateKeys) {
                auth.push({ type: 'privateKey', fileRef, passphraseRef: null })
            }
        } else if (options.auth === 'agent') {
            const socket = await this.resolveAgentSocket()
            auth.push({ type: 'agent', socket })
        } else if (authMode === 'keyboardInteractive') {
            if (options.password) {
                auth.push({ type: 'keyboardInteractive', password: options.password })
            }
            auth.push({ type: 'keyboardInteractive', secretRef: this.passwordSecretRef() })
        }
        return auth
    }

    private async jumpChain (jumpHost: string|null): Promise<SshJumpRequest[]> {
        if (!jumpHost) {
            return []
        }
        const chain: SshJumpRequest[] = []
        const seen = new Set<string>()
        let current: string|null = jumpHost
        const profilesService = this.injector.get(ProfilesService)
        const profiles = (await profilesService.getProfiles())
            .filter(profile => profile.type === 'ssh')
        while (current) {
            if (seen.has(current) || current === this.profile.id) {
                throw new Error('SSH jump host configuration contains a cycle')
            }
            seen.add(current)
            const currentId = current
            const jump = profiles.find(profile => profile.id === currentId)
            if (!jump) {
                throw new Error(`SSH jump host "${currentId}" was not found in the profile list`)
            }
            const jumpOptions = profilesService.getConfigProxyForProfile<SSHProfile>(jump).options
            chain.push({
                host: jumpOptions.host,
                port: jumpOptions.port ?? 22,
                username: jumpOptions.user || null,
                auth: await this.authForOptions(jumpOptions),
            })
            current = jumpOptions.jumpHost
        }
        return chain.reverse()
    }

    private async startForwardings (sessionId: string): Promise<void> {
        for (const forwarding of this.profile.options.forwardedPorts) {
            const kind = forwarding.type.toLowerCase() as 'local'|'remote'|'dynamic'
            const info = await this.bridge.invoke('ssh.forwardingStart', {
                sessionId,
                kind,
                bindHost: forwarding.host || '127.0.0.1',
                bindPort: forwarding.port || 0,
                targetAddress: forwarding.targetAddress || '',
                targetPort: forwarding.targetPort || 0,
            })
            this.forwardingIds.push(info.id)
        }
    }

    private passwordSecretRef (): string {
        return this.vault.isEnabled() ? 'ssh-password://vault' : 'ssh-password://keychain'
    }

    private async handleCredentialPrompt (prompt: SshAuthPrompt): Promise<void> {
        const hasTarget = prompt.username === true || Boolean(prompt.password ?? prompt.privateKeyHash)
        if (!hasTarget || this.destroying || this.credentialModals.has(prompt.requestId)) {
            return
        }
        this.credentialModals.set(prompt.requestId, null)
        let responses: string[] = []
        let abort = false
        try {
            const target = prompt.password
            const profile: SSHProfile|undefined = target ? {
                ...this.profile,
                options: { ...this.profile.options, host: target.host, port: target.port, user: target.username },
            } : undefined
            const storage = this.injector.get(TauriPasswordStorageService)
            let saved: string|null = null
            if (prompt.privateKeyHash) {
                for (const [id, pending] of this.pendingPassphrases) {
                    if (pending.hash === prompt.privateKeyHash) { this.pendingPassphrases.delete(id) }
                }
                await storage.deletePrivateKeyPassword(prompt.privateKeyHash).catch(() => {
                    this.serviceMessage.next('SSH saved private-key passphrase could not be removed')
                })
            } else if (profile && target) {
                saved = await storage.loadPassword(profile, target.username).catch(() => null)
            }
            if (!this.credentialModals.has(prompt.requestId)) { return }
            const modal = this.modals.open(PromptModalComponent)
            this.credentialModals.set(prompt.requestId, modal)
            const component = modal.componentInstance as PromptModalComponent
            Object.assign(component, {
                prompt: prompt.name, password: !prompt.username, showRememberCheckbox: !prompt.username, remember: false, value: saved ?? '',
            })
            const result = await modal.result.catch(() => null) as { value: string; remember: boolean }|null
            component.value = ''
            if (result && this.credentialModals.has(prompt.requestId)) {
                const value = result.value
                responses = [value]
                if (result.remember && !prompt.username) {
                    if (prompt.privateKeyHash) {
                        this.pendingPassphrases.set(prompt.requestId, { hash: prompt.privateKeyHash, value })
                    } else if (profile && target) {
                        this.pendingPasswords.set(prompt.requestId, { profile, value, username: target.username })
                    }
                }
            }
        } catch {
            abort = true
            this.serviceMessage.next('SSH credential prompt could not be opened')
        } finally {
            if (this.credentialModals.delete(prompt.requestId)) {
                await this.bridge.invoke('ssh.authResponse', { requestId: prompt.requestId, responses, ...abort ? { abort: true } : {} }).catch(() => {
                    this.pendingPasswords.delete(prompt.requestId)
                    this.pendingPassphrases.delete(prompt.requestId)
                    this.logger.warn('SSH credential response could not be sent')
                })
            }
        }
    }

    private async removeRejectedPassword (error: unknown): Promise<void> {
        const rejected = error as Partial<SshConnectError>|null
        const target = rejected?.passwordDeletionTarget
        if (this.destroying || rejected?.code !== 'permissionDenied' || !target ||
            typeof target.host !== 'string' || !target.host ||
            typeof target.username !== 'string' || !target.username ||
            !Number.isInteger(target.port) || target.port < 1 || target.port > 65535) {
            return
        }
        const profile: SSHProfile = {
            ...this.profile,
            options: { ...this.profile.options, host: target.host, port: target.port, user: target.username },
        }
        try {
            await this.injector.get(TauriPasswordStorageService).deletePassword(profile, target.username)
        } catch {
            this.serviceMessage.next('SSH saved password could not be removed')
        }
    }

    private async saveAcceptedPassword (requestId: string): Promise<void> {
        const pending = this.pendingPasswords.get(requestId)
        if (!pending) { return }
        this.pendingPasswords.delete(requestId)
        try {
            await this.injector.get(TauriPasswordStorageService).savePassword(pending.profile, pending.value, pending.username)
        } catch {
            this.serviceMessage.next('SSH password could not be saved')
        }
    }

    private clearCredentialPrompts (): void {
        this.pendingPasswords.clear()
        this.pendingPassphrases.clear()
        for (const [requestId, modal] of this.credentialModals) {
            modal?.dismiss()
            void this.bridge.invoke('ssh.authResponse', { requestId, responses: [], abort: true }).catch(() => undefined)
        }
        this.credentialModals.clear()
    }

    private async saveUnlockedPassphrase (requestId: string): Promise<void> {
        const pending = this.pendingPassphrases.get(requestId)
        if (!pending) { return }
        this.pendingPassphrases.delete(requestId)
        try {
            await this.injector.get(TauriPasswordStorageService).savePrivateKeyPassword(pending.hash, pending.value)
        } catch {
            this.serviceMessage.next('SSH private-key passphrase could not be saved')
        }
    }

    private async handleHostKeyPrompt (prompt: SshHostKeyPrompt): Promise<void> {
        if (this.destroying) { return }
        const modal = this.modals.open(TauriSshHostKeyPromptModalComponent)
        this.hostKeyModals.add(modal)
        modal.componentInstance.prompt = prompt
        const decision = await modal.result.catch(() => 'reject') as 'once'|'save'|'reject'
        this.hostKeyModals.delete(modal)
        await this.bridge.invoke('ssh.hostKeyDecision', {
            requestId: prompt.requestId,
            decision,
        }).catch(error => this.logger.warn('SSH host key decision failed', error))
    }

    private async cancelPendingConnection (): Promise<void> {
        await this.bridge.invoke('ssh.cancelConnect', { connectionId: this.connectionId }).catch(error => {
            this.logger.debug('SSH pending connection cancellation failed', error)
        })
    }
}
