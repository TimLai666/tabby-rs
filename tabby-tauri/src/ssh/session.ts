import { Injector } from '@angular/core'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { Observable, Subject } from 'rxjs'
import { LogService, ProfilesService, VaultService } from 'tabby-core'
import { BaseSession, InputProcessor, UTF8SplitterMiddleware } from 'tabby-terminal'

import { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'
import {
    HostBridge,
    SshAuthPrompt,
    SshAuthMethodRef,
    SshConnectRequest,
    SshHostKeyPrompt,
    SshJumpRequest,
} from '../api/hostBridge'
import { TauriSshHostKeyPromptModalComponent } from './hostKeyPromptModal.component'
import { TauriSftpSession } from './sftp'

function base64Json (value: unknown): string {
    const bytes = new TextEncoder().encode(JSON.stringify(value))
    let binary = ''
    for (const byte of bytes) {
        binary += String.fromCharCode(byte)
    }
    return btoa(binary)
}

export class TauriSshSession extends BaseSession {
    private id: string|null = null
    private destroying = false
    private readonly connectionId = window.crypto.randomUUID()
    private pendingOutput: { data: number[]; extended: boolean }[] = []
    private pendingExit = false
    private unlisteners: (() => void)[] = []
    private readonly authPrompt = new Subject<SshAuthPrompt>()
    private forwardingIds: string[] = []
    private sftp: TauriSftpSession|null = null

    get authPrompt$ (): Observable<SshAuthPrompt> {
        return this.authPrompt.asObservable()
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
        this.unlisteners.push(...await Promise.all([
            this.bridge.listen('ssh.output', event => {
                if (event.connectionId === this.connectionId) {
                    if (!this.id) {
                        this.pendingOutput.push({ data: event.data, extended: event.extended })
                        return
                    }
                    this.emitOutput(Buffer.from(event.data))
                }
            }),
            this.bridge.listen('ssh.exit', event => {
                if (event.connectionId === this.connectionId) {
                    if (!this.id) {
                        this.pendingExit = true
                    } else if (this.open) {
                        void this.destroy()
                    }
                }
            }),
            this.bridge.listen('ssh.hostKeyPrompt', prompt => {
                if (prompt.connectionId === this.connectionId) {
                    void this.handleHostKeyPrompt(prompt)
                }
            }),
            this.bridge.listen('ssh.authPrompt', prompt => {
                if (prompt.connectionId === this.connectionId) {
                    this.authPrompt.next(prompt)
                }
            }),
        ]))

        const info = await this.bridge.invoke('ssh.connect', await this.connectRequest())
        // The session can be destroyed while the bridge connection is still opening.
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
        if (this.destroying) {
            await this.bridge.invoke('ssh.close', { id: info.id }).catch(error => {
                this.logger.debug('SSH close failed after cancelled start', error)
            })
            return
        }
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
        const id = this.id
        this.id = null
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

    async openSFTP (): Promise<TauriSftpSession> {
        if (!this.id || !this.open) {
            throw new Error('SSH session is not open')
        }
        if (!this.sftp) {
            this.sftp = await TauriSftpSession.open(this.bridge, this.id)
        }
        return this.sftp
    }

    private async connectRequest (): Promise<SshConnectRequest> {
        const options = this.profile.options
        const auth = await this.authForOptions(options)
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
            environment: {},
            x11: !!options.x11,
            x11Display: null,
            agentForward: !!options.agentForward,
            jumpChain: await this.jumpChain(options.jumpHost),
        }
    }

    private async authForOptions (options: SSHProfile['options']): Promise<SshAuthMethodRef[]> {
        const auth: SshAuthMethodRef[] = []
        if (options.auth === 'password') {
            auth.push({ type: 'password', secretRef: await this.passwordSecretRef(options) })
        } else if (options.auth === 'publicKey') {
            const privateKeys = options.privateKeys.length
                ? options.privateKeys
                : await this.bridge.invoke('ssh.listPrivateKeys', {})
            for (const fileRef of privateKeys) {
                auth.push({ type: 'privateKey', fileRef, passphraseRef: null })
            }
        } else if (options.auth === 'agent') {
            auth.push({ type: 'agent', socket: null })
        } else if (options.auth === 'keyboardInteractive') {
            auth.push({ type: 'keyboardInteractive' })
        }
        return auth
    }

    private async jumpChain (jumpHost: string|null): Promise<SshJumpRequest[]> {
        const chain: SshJumpRequest[] = []
        const seen = new Set<string>()
        let current = jumpHost
        const profiles = (await this.injector.get(ProfilesService).getProfiles({ includeBuiltin: false }))
            .filter(profile => profile.type === 'ssh')
        while (current) {
            if (seen.has(current) || current === this.profile.id) {
                throw new Error('SSH jump host configuration contains a cycle')
            }
            seen.add(current)
            if (chain.length >= 3) {
                throw new Error('SSH jump host configuration supports at most three hops')
            }
            const currentId = current
            const jump = profiles.find(profile => profile.id === currentId)
            if (!jump) {
                throw new Error(`SSH jump host "${currentId}" was not found in the profile list`)
            }
            const jumpOptions = jump.options as SSHProfile['options']
            chain.push({
                host: jumpOptions.host,
                port: jumpOptions.port,
                username: jumpOptions.user || null,
                auth: await this.authForOptions(jumpOptions),
            })
            current = jumpOptions.jumpHost
        }
        return chain
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

    private async passwordSecretRef (options = this.profile.options): Promise<string> {
        const account = options.user
        if (!this.vault.isEnabled()) {
            return `keychain://ssh@${options.host}:${options.port ?? 22}/${account}`
        }
        const selector = {
            type: 'password',
            key: { user: account, host: options.host, port: options.port ?? 22 },
        }
        return `vault-secret://${base64Json(selector)}`
    }

    private async handleHostKeyPrompt (prompt: SshHostKeyPrompt): Promise<void> {
        const modal = this.modals.open(TauriSshHostKeyPromptModalComponent)
        modal.componentInstance.prompt = prompt
        const decision = await modal.result.catch(() => 'reject') as 'once'|'save'|'reject'
        await this.bridge.invoke('ssh.hostKeyDecision', {
            requestId: prompt.requestId,
            decision,
        }).catch(error => this.logger.warn('SSH host key decision failed', error))
    }
}
