import { marker as _ } from '@biesbjerg/ngx-translate-extract-marker'
import { Component, Injector } from '@angular/core'
import { Platform } from 'tabby-core'
import { BaseTerminalTabComponent, ConnectableTerminalTabComponent } from 'tabby-terminal'

import { HostBridge } from '../api/hostBridge'
import { TauriTelnetProfile } from './profile'
import { TauriTelnetSession } from './session'

@Component({
    selector: 'tauri-telnet-tab',
    template: `${BaseTerminalTabComponent.template} ${require('./tab.component.pug')}`,
    styleUrls: ['./tab.component.scss', ...BaseTerminalTabComponent.styles],
    animations: BaseTerminalTabComponent.animations,
})
export class TauriTelnetTabComponent extends ConnectableTerminalTabComponent<TauriTelnetProfile> {
    Platform = Platform
    session: TauriTelnetSession|null = null
    private reconnectAttempts = 0

    constructor (injector: Injector, private bridge: HostBridge) {
        super(injector)
        this.enableToolbar = true
    }

    ngOnInit (): void {
        this.subscribeUntilDestroyed(this.hotkeys.hotkey$, hotkey => {
            if (this.hasFocus && hotkey === 'restart-telnet-session') {
                void this.reconnect()
            }
        })
        super.ngOnInit()
    }

    async initializeSession (): Promise<void> {
        await super.initializeSession()
        const session = new TauriTelnetSession(this.injector, this.bridge, this.profile)
        this.setSession(session)
        this.startSpinner(this.translate.instant(_('Connecting')))
        this.attachSessionHandler(session.serviceMessage$, message => {
            this.write(`\r TELNET  ${message}\r\n`)
            session.resize(this.size.columns, this.size.rows)
        })
        try {
            await session.start()
            session.resize(this.size.columns, this.size.rows)
            this.reconnectAttempts = 0
            this.stopSpinner()
            this.write('\r\n TELNET  Unencrypted connection\r\n')
        } catch (error) {
            this.stopSpinner()
            this.write(' X  ' + String(error) + '\r\n')
            await session.destroy()
        }
    }

    protected onSessionDestroyed (): void {
        if (this.frontend && this.profile.behaviorOnSessionEnd === 'reconnect' && !this.isDisconnectedByHand) {
            if (this.reconnectAttempts < 5) {
                const delay = Math.min(30_000, 1_000 * 2 ** this.reconnectAttempts)
                this.reconnectAttempts++
                this.write(`\r\nTelnet reconnecting in ${Math.ceil(delay / 1000)}s (${this.reconnectAttempts}/5)\r\n`)
                setTimeout(() => void this.reconnect(), delay)
            } else {
                this.offerReconnection()
            }
            return
        }
        super.onSessionDestroyed()
    }

    protected isSessionExplicitlyTerminated (): boolean {
        return super.isSessionExplicitlyTerminated() || this.recentInputs.endsWith('close\r') || this.recentInputs.endsWith('quit\r')
    }
}
