import { marker as _ } from '@biesbjerg/ngx-translate-extract-marker'
import { Component, Injector } from '@angular/core'
import { Platform } from 'tabby-core'
import { BaseTerminalTabComponent, ConnectableTerminalTabComponent } from 'tabby-terminal'

import { HostBridge } from '../api/hostBridge'
import { TauriTelnetProfile } from './profile'
import { TauriTelnetSession } from './session'

const colors = require('ansi-colors')

@Component({
    selector: 'tauri-telnet-tab',
    template: `${BaseTerminalTabComponent.template} ${require('./tab.component.pug')}`,
    styleUrls: ['./tab.component.scss', ...BaseTerminalTabComponent.styles],
    animations: BaseTerminalTabComponent.animations,
})
export class TauriTelnetTabComponent extends ConnectableTerminalTabComponent<TauriTelnetProfile> {
    Platform = Platform
    session: TauriTelnetSession|null = null

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
            this.stopSpinner()
            this.write('\r\n TELNET  Unencrypted connection\r\n')
        } catch (error) {
            this.stopSpinner()
            const message = typeof error?.details === 'string' ? error.details
                : typeof error?.message === 'string' ? error.message : String(error)
            this.write(colors.black.bgRed(' X ') + ' ' + colors.red(message) + '\r\n')
            await session.destroy()
        }
    }

    protected onSessionDestroyed (): void {
        if (this.frontend) {
            this.write('\r\n' + colors.black.bgWhite(' TELNET ') + ` ${this.session?.profile.options.host}: session closed\r\n`)
            super.onSessionDestroyed()
        }
    }

    async canClose (): Promise<boolean> {
        if (!this.session?.open) {
            return true
        }
        return (await this.platform.showMessageBox(
            {
                type: 'warning',
                message: this.translate.instant(_('Disconnect from {host}?'), this.profile.options),
                buttons: [
                    this.translate.instant(_('Disconnect')),
                    this.translate.instant(_('Do not close')),
                ],
                defaultId: 0,
                cancelId: 1,
            },
        )).response === 0
    }

    protected isSessionExplicitlyTerminated (): boolean {
        return super.isSessionExplicitlyTerminated() || this.recentInputs.endsWith('close\r') || this.recentInputs.endsWith('quit\r')
    }
}
