import { marker as _ } from '@biesbjerg/ngx-translate-extract-marker'
import { Component, Injector } from '@angular/core'
import { Platform, SelectorService } from 'tabby-core'
import { BaseTerminalTabComponent, ConnectableTerminalTabComponent } from 'tabby-terminal'

import { HostBridge } from '../api/hostBridge'
import { BAUD_RATES, TauriSerialProfile } from './profile'
import { TauriSerialSession } from './session'

const colors = require('ansi-colors')

@Component({
    selector: 'tauri-serial-tab',
    template: `${BaseTerminalTabComponent.template} ${require('./tab.component.pug')}`,
    styleUrls: ['./tab.component.scss', ...BaseTerminalTabComponent.styles],
    animations: BaseTerminalTabComponent.animations,
})
export class TauriSerialTabComponent extends ConnectableTerminalTabComponent<TauriSerialProfile> {
    Platform = Platform
    session: TauriSerialSession|null = null
    private reconnectAttempts = 0
    private reconnectTimer: ReturnType<typeof setTimeout>|null = null

    constructor (injector: Injector, private bridge: HostBridge, private selector: SelectorService) {
        super(injector)
        this.enableToolbar = true
    }

    ngOnInit (): void {
        this.subscribeUntilDestroyed(this.hotkeys.hotkey$, hotkey => {
            if (!this.hasFocus) {
                return
            }
            switch (hotkey) {
                case 'home':
                    this.sendInput('\x1b[H')
                    break
                case 'end':
                    this.sendInput('\x1b[F')
                    break
                case 'restart-serial-session':
                    void this.reconnect()
                    break
            }
        })
        super.ngOnInit()
        setImmediate(() => this.setTitle(this.profile.name))
    }

    async changeBaudRate (): Promise<void> {
        const rate = await this.selector.show(
            this.translate.instant(_('Baud rate')),
            BAUD_RATES.map(x => ({ name: x.toString(), result: x, weight: x })),
        ).catch(() => null)
        if (rate === null) {
            return
        }
        const session = this.session
        const update = session?.setBaudRate(rate)
        this.profile.options.baudRate = rate
        try {
            await update
        } catch (error) {
            const message = typeof error?.details === 'string' ? error.details
                : typeof error?.message === 'string' ? error.message : String(error)
            this.notifications.error(message)
            await session?.destroy()
        }
    }

    protected isSessionExplicitlyTerminated (): boolean {
        return super.isSessionExplicitlyTerminated() ||
            this.recentInputs.endsWith('close\r') ||
            this.recentInputs.endsWith('quit\r')
    }

    async initializeSession (): Promise<void> {
        this.cancelReconnectTimer()
        await super.initializeSession()
        const session = new TauriSerialSession(this.injector, this.bridge, this.profile)
        this.setSession(session)
        this.startSpinner(this.translate.instant(_('Connecting')))
        this.attachSessionHandler(session.serviceMessage$, message => {
            this.write(`\r SERIAL  ${message}\r\n`)
            session.resize()
        })
        try {
            await session.start()
            session.resize()
            this.reconnectAttempts = 0
            this.cancelReconnectTimer()
            this.stopSpinner()
        } catch (error) {
            this.stopSpinner()
            const message = typeof error?.details === 'string' ? error.details
                : typeof error?.message === 'string' ? error.message : String(error)
            this.write(colors.black.bgRed(' X ') + ' ' + colors.red(message) + '\r\n')
            await session.destroy()
        }
    }

    protected onSessionDestroyed (): void {
        if (this.frontend && this.profile.behaviorOnSessionEnd === 'reconnect' && !this.isDisconnectedByHand) {
            if (this.reconnectAttempts < 5) {
                const delay = Math.min(30_000, 1_000 * 2 ** this.reconnectAttempts)
                this.reconnectAttempts++
                this.write(`\r\nSerial reconnecting in ${Math.ceil(delay / 1000)}s (${this.reconnectAttempts}/5)\r\n`)
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

    async disconnect (): Promise<void> {
        this.cancelReconnectTimer()
        await super.disconnect()
    }

    ngOnDestroy (): void {
        this.cancelReconnectTimer()
        super.ngOnDestroy()
    }

    private cancelReconnectTimer (): void {
        if (this.reconnectTimer !== null) {
            clearTimeout(this.reconnectTimer)
            this.reconnectTimer = null
        }
    }
}
