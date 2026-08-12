import { Component, Injector } from '@angular/core'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { GetRecoveryTokenOptions, RecoveryToken, VaultService } from 'tabby-core'
import { ConnectableTerminalTabComponent, BaseTerminalTabComponent } from 'tabby-terminal'
import { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'

import { HostBridge } from '../api/hostBridge'
import { TauriSshAuthPromptModalComponent } from './authPromptModal.component'
import { TauriSshSession } from './session'

@Component({
    selector: 'tauri-ssh-tab',
    template: `${BaseTerminalTabComponent.template}<tauri-sftp-panel *ngIf="sftpPanelVisible" [session]="session" [(path)]="sftpPath" (close)="sftpPanelVisible = false"></tauri-sftp-panel>`,
    styles: BaseTerminalTabComponent.styles,
    animations: BaseTerminalTabComponent.animations,
})
export class TauriSshTabComponent extends ConnectableTerminalTabComponent<SSHProfile> {
    declare session: TauriSshSession|null
    sftpPanelVisible = false
    sftpPath = '/'
    private reconnectAttempts = 0

    constructor (
        injector: Injector,
        private bridge: HostBridge,
        private vault: VaultService,
        private modals: NgbModal,
    ) {
        super(injector)
    }

    async initializeSession (): Promise<void> {
        await super.initializeSession()
        const session = new TauriSshSession(
            this.injector,
            this.bridge,
            this.vault,
            this.profile,
            this.modals,
        )
        this.setSession(session)
        this.attachSessionHandler(session.authPrompt$, prompt => void this.showAuthPrompt(prompt))
        this.attachSessionHandler(session.serviceMessage$, message => this.write(`\r\n${message}\r\n`))
        try {
            await session.start()
            session.resize(this.size.columns, this.size.rows)
            this.reconnectAttempts = 0
        } catch (error) {
            this.write(`\r\nSSH connection failed: ${String(error)}\r\n`)
            await session.destroy()
        }
    }

    protected onSessionDestroyed (): void {
        if (this.frontend && this.profile.behaviorOnSessionEnd === 'reconnect' && !this.isDisconnectedByHand) {
            if (this.reconnectAttempts < 5) {
                const delay = Math.min(30_000, 1_000 * 2 ** this.reconnectAttempts)
                this.reconnectAttempts++
                this.write(`\r\nSSH reconnecting in ${Math.ceil(delay / 1000)}s (${this.reconnectAttempts}/5)\r\n`)
                setTimeout(() => void this.reconnect(), delay)
            } else {
                this.offerReconnection()
            }
            return
        }
        super.onSessionDestroyed()
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

    private async showAuthPrompt (prompt: import('../api/hostBridge').SshAuthPrompt): Promise<void> {
        const modal = this.modals.open(TauriSshAuthPromptModalComponent)
        modal.componentInstance.prompt = prompt
        const responses = await modal.result.catch(() => null) as string[]|null
        await this.bridge.invoke('ssh.authResponse', {
            requestId: prompt.requestId,
            responses: responses ?? [],
        }).catch(error => this.logger.warn('SSH authentication response failed', error))
    }
}
