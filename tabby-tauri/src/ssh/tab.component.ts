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
    template: BaseTerminalTabComponent.template,
    styles: BaseTerminalTabComponent.styles,
    animations: BaseTerminalTabComponent.animations,
})
export class TauriSshTabComponent extends ConnectableTerminalTabComponent<SSHProfile> {
    declare session: TauriSshSession|null

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
        try {
            await session.start()
            session.resize(this.size.columns, this.size.rows)
        } catch (error) {
            this.write(`\r\nSSH connection failed: ${String(error)}\r\n`)
            await session.destroy()
        }
    }

    async getRecoveryToken (options?: GetRecoveryTokenOptions): Promise<RecoveryToken> {
        const token = await super.getRecoveryToken(options)
        const profile = token.profile as SSHProfile
        const safeOptions = { ...profile.options } as SSHProfile['options'] & { password?: string|null }
        delete safeOptions.password
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
