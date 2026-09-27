import { Component, Injector } from '@angular/core'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { GetRecoveryTokenOptions, RecoveryToken, VaultService } from 'tabby-core'
import { ConnectableTerminalTabComponent, BaseTerminalTabComponent } from 'tabby-terminal'
import { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'

import { HostBridge } from '../api/hostBridge'
import { TauriWinSCPService } from '../services/winscp.service'
import { KeyboardInteractivePrompt } from '../../../tabby-ssh/src/api/keyboardInteractivePrompt'
import { TauriSshSession } from './session'

@Component({
    selector: 'tauri-ssh-tab',
    template: `${BaseTerminalTabComponent.template}<tauri-sftp-panel *ngIf="sftpPanelVisible" [session]="session" [(path)]="sftpPath" (close)="sftpPanelVisible = false"></tauri-sftp-panel>
        <keyboard-interactive-auth-panel class="bg-dark" *ngIf="activeKIPrompt"
            [prompt]="activeKIPrompt" [profile]="activeKIProfile"
            (click)="$event.stopPropagation()" (done)="frontend?.focus()">
        </keyboard-interactive-auth-panel>`,
    styles: BaseTerminalTabComponent.styles,
    animations: BaseTerminalTabComponent.animations,
})
export class TauriSshTabComponent extends ConnectableTerminalTabComponent<SSHProfile> {
    declare session: TauriSshSession|null
    sftpPanelVisible = false
    sftpPath = '/'
    activeKIPrompt: KeyboardInteractivePrompt|null = null
    activeKIProfile: SSHProfile|null = null
    private activeKIRequestId: string|null = null
    private reconnectAttempts = 0
    private reconnectTimer: ReturnType<typeof setTimeout>|null = null

    constructor (
        injector: Injector,
        private bridge: HostBridge,
        private vault: VaultService,
        private modals: NgbModal,
        private winscp: TauriWinSCPService,
    ) {
        super(injector)
    }

    ngOnInit (): void {
        this.subscribeUntilDestroyed(this.hotkeys.hotkey$, hotkey => {
            if (!this.hasFocus) { return }
            switch (hotkey) {
                case 'home': this.sendInput('\x1bOH'); break
                case 'end': this.sendInput('\x1bOF'); break
                case 'restart-ssh-session': void this.reconnect(); break
                case 'open-sftp': void this.openSFTP(); break
                case 'launch-winscp': void this.launchWinSCP(); break
            }
        })
        super.ngOnInit()
    }

    async launchWinSCP (): Promise<void> {
        if (!this.session) { return }
        try {
            await this.winscp.launchWinSCP(this.session)
        } catch {
            this.notifications.error(this.translate.instant('Could not launch WinSCP'))
        }
    }

    async initializeSession (): Promise<void> {
        this.clearAuthPrompt()
        this.cancelReconnectTimer()
        await super.initializeSession()
        const session = new TauriSshSession(
            this.injector,
            this.bridge,
            this.vault,
            this.profile,
            this.modals,
        )
        this.setSession(session)
        this.attachSessionHandler(session.authPrompt$, prompt => void this.showAuthPrompt(prompt, session))
        this.attachSessionHandler(session.serviceMessage$, message => {
            message = message.replace(/\n/g, '\r\n      ')
            this.write(`\r\x1b[30m\x1b[47m SSH \x1b[49m\x1b[39m ${message}\r\n`)
        })
        try {
            await session.start()
            session.resize(this.size.columns, this.size.rows)
            this.reconnectAttempts = 0
            this.cancelReconnectTimer()
        } catch (error) {
            if (session.isClosing || this.session !== session) {
                await session.destroy()
                return
            }
            if (this.session === session) {
                this.clearAuthPrompt()
            }
            const message = typeof error?.details === 'string' ? error.details : String(error)
            this.write(`\r\nSSH connection failed: ${message}\r\n`)
            await session.destroy()
        }
    }

    protected onSessionDestroyed (): void {
        this.clearAuthPrompt()
        if (this.frontend && this.profile.behaviorOnSessionEnd === 'reconnect' && !this.isDisconnectedByHand) {
            if (this.reconnectAttempts < 5) {
                const delay = Math.min(30_000, 1_000 * 2 ** this.reconnectAttempts)
                this.reconnectAttempts++
                this.write(`\r\nSSH reconnecting in ${Math.ceil(delay / 1000)}s (${this.reconnectAttempts}/5)\r\n`)
                this.cancelReconnectTimer()
                this.reconnectTimer = setTimeout(() => {
                    this.reconnectTimer = null
                    if (this.isDisconnectedByHand || !this.frontend) {
                        return
                    }
                    void this.reconnect()
                }, delay)
            } else {
                this.offerReconnection()
            }
            return
        }
        super.onSessionDestroyed()
    }

    async canClose (): Promise<boolean> {
        if (!this.session?.open) {
            return true
        }
        if (!(this.profile.options.warnOnClose ?? this.config.store.ssh.warnOnClose)) {
            return true
        }
        return (await this.platform.showMessageBox(
            {
                type: 'warning',
                message: this.translate.instant('Disconnect from {host}?', this.profile.options),
                buttons: [
                    this.translate.instant('Disconnect'),
                    this.translate.instant('Do not close'),
                ],
                defaultId: 0,
                cancelId: 1,
            },
        )).response === 0
    }

    async disconnect (): Promise<void> {
        this.clearAuthPrompt()
        this.cancelReconnectTimer()
        await super.disconnect()
    }

    async destroy (): Promise<void> {
        this.isDisconnectedByHand = true
        this.cancelReconnectTimer()
        const pending = !this.session?.open ? this.session?.destroy() : undefined
        this.clearAuthPrompt()
        await super.destroy()
        await pending
    }

    ngOnDestroy (): void {
        this.clearAuthPrompt()
        this.cancelReconnectTimer()
        super.ngOnDestroy()
    }

    async openSFTP (): Promise<void> {
        if (!this.session?.open) { return }
        this.sftpPath = await this.session.getWorkingDirectory() ?? this.sftpPath
        this.sftpPanelVisible = true
    }

    async getRecoveryToken (options?: GetRecoveryTokenOptions): Promise<RecoveryToken> {
        const token = await super.getRecoveryToken(options)
        const profile = token.profile as SSHProfile
        const safeOptions = Object.fromEntries(
            Object.entries(profile.options).filter(([key]) => key !== 'password'),
        ) as SSHProfile['options']
        token.profile = { ...profile, options: safeOptions }
        return token
    }

    private async showAuthPrompt (prompt: import('../api/hostBridge').SshAuthPrompt, session: TauriSshSession): Promise<void> {
        if (session !== this.session || prompt.requestId === this.activeKIRequestId) {
            return
        }
        this.clearAuthPrompt()
        const target = prompt.keyboardInteractive
        if (!target) {
            await this.bridge.invoke('ssh.authResponse', { requestId: prompt.requestId, responses: [] })
                .catch(error => this.logger.warn('SSH authentication response failed', error))
            return
        }
        const interactive = new KeyboardInteractivePrompt(prompt.name, prompt.instructions,
            prompt.prompts.map(item => ({ prompt: item.text, echo: item.echo })))
        if (prompt.savedPassword) {
            for (let i = 0; i < interactive.prompts.length; i++) {
                if (interactive.isAPasswordPrompt(i)) {
                    interactive.responses[i] = prompt.savedPassword
                }
            }
        }
        this.activeKIProfile = {
            ...this.profile,
            options: { ...this.profile.options, host: target.host, port: target.port, user: target.username, password: undefined },
        }
        this.activeKIPrompt = interactive
        this.activeKIRequestId = prompt.requestId
        const responses = await interactive.promise.catch(() => null)
        const isCurrent = this.activeKIPrompt === interactive && this.session === session
        if (this.activeKIPrompt === interactive) {
            this.activeKIPrompt = null
            this.activeKIProfile = null
            this.activeKIRequestId = null
        }
        await this.bridge.invoke('ssh.authResponse', {
            requestId: prompt.requestId,
            responses: isCurrent ? responses ?? [] : [],
            ...!isCurrent ? { abort: true } : {},
        }).catch(error => this.logger.warn('SSH authentication response failed', error))
    }

    private clearAuthPrompt (): void {
        this.activeKIPrompt?.reject()
        this.activeKIPrompt = null
        this.activeKIProfile = null
        this.activeKIRequestId = null
    }

    private cancelReconnectTimer (): void {
        if (this.reconnectTimer !== null) {
            clearTimeout(this.reconnectTimer)
            this.reconnectTimer = null
        }
    }
}
