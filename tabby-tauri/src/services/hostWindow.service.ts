import { Injectable } from '@angular/core'
import { HostWindowService } from 'tabby-core'

import { HostBridge } from '../api/hostBridge'

@Injectable()
export class TauriHostWindowService extends HostWindowService {
    constructor (private bridge: HostBridge) {
        super()
        setTimeout(() => this.windowShown.next())
        window.addEventListener('focus', () => this.windowFocused.next())
        window.addEventListener('beforeunload', () => this.windowCloseRequest.next())
    }

    get isFullscreen (): boolean {
        return document.fullscreenElement !== null
    }

    reload (): void {
        window.location.reload()
    }

    setTitle (title = 'Tabby RS'): void {
        document.title = title
    }

    toggleFullscreen (): void {
        if (document.fullscreenElement) {
            void document.exitFullscreen()
        } else {
            void document.documentElement.requestFullscreen()
        }
    }

    minimize (): void {
        console.warn('Window minimization is not implemented by the Tauri foundation yet')
    }

    isMaximized (): boolean {
        return false
    }

    toggleMaximize (): void {
        console.warn('Window maximization is not implemented by the Tauri foundation yet')
    }

    close (): void {
        void this.bridge.invoke('app.quit', {})
    }

    bringToFront (): void {
        window.focus()
    }
}
