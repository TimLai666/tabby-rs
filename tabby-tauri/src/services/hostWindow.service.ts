import { Injectable } from '@angular/core'
import { HostWindowBounds, HostWindowService } from 'tabby-core'

import { HostBridge, WindowStateSnapshot } from '../api/hostBridge'

@Injectable()
export class TauriHostWindowService extends HostWindowService {
    private state: WindowStateSnapshot = {
        visible: true,
        alwaysOnTop: false,
        fullscreen: false,
        maximized: false,
        minimized: false,
        focused: true,
        bounds: { x: 0, y: 0, width: 1100, height: 720 },
        scaleFactor: 1,
        capabilities: {
            absolutePositioning: true,
            docking: true,
            globalHotkey: true,
            opacity: false,
            vibrancy: false,
            progress: true,
            clipboard: true,
            dialogs: true,
            notifications: true,
        },
    }

    constructor (private bridge: HostBridge) {
        super()
        void this.initialize()
    }

    get isFullscreen (): boolean {
        return this.state.fullscreen
    }

    reload (): void {
        void this.invoke('window.reload', {})
    }

    setTitle (title = 'Tabby RS'): void {
        void this.apply({ title })
    }

    toggleFullscreen (): void {
        this.state.fullscreen = !this.state.fullscreen
        void this.apply({ fullscreen: this.state.fullscreen })
    }

    minimize (): void {
        this.state.minimized = true
        void this.invoke('window.minimize', {})
    }

    isMaximized (): boolean {
        return this.state.maximized
    }

    toggleMaximize (): void {
        this.state.maximized = !this.state.maximized
        void this.invoke('window.toggleMaximize', {}).then(() => this.refreshState())
    }

    close (): void {
        void this.invoke('window.close', {})
    }

    openDevTools (): void {
        void this.invoke('window.openDevtools', {})
    }

    bringToFront (): void {
        this.state.visible = true
        this.state.minimized = false
        void this.invoke('window.bringToFront', {}).then(() => this.refreshState())
    }

    setBounds (bounds: HostWindowBounds): void {
        this.state.bounds = bounds
        void this.apply({ bounds })
    }

    setAlwaysOnTop (enabled: boolean): void {
        this.state.alwaysOnTop = enabled
        void this.apply({ alwaysOnTop: enabled })
    }

    setOpacity (opacity: number): void {
        if (!this.state.capabilities.opacity) {
            return
        }
        void this.apply({ opacity })
    }

    setProgressBar (progress: number): void {
        void this.apply({ progress: progress < 0 ? null : progress })
    }

    get snapshot (): WindowStateSnapshot {
        return this.state
    }

    private async initialize (): Promise<void> {
        await Promise.all([
            this.bridge.listen('desktop.windowFocused', focused => {
                this.state.focused = focused
                if (focused) {
                    this.windowFocused.next()
                }
            }),
            this.bridge.listen('desktop.windowMoved', position => {
                this.state.bounds.x = position.x
                this.state.bounds.y = position.y
                this.windowMoved.next()
            }),
            this.bridge.listen('desktop.windowResized', size => {
                this.state.bounds.width = size.width
                this.state.bounds.height = size.height
            }),
            this.bridge.listen('desktop.windowCloseRequested', () => this.windowCloseRequest.next()),
        ])
        await this.refreshState()
        this.windowShown.next()
    }

    private async refreshState (): Promise<void> {
        try {
            this.state = await this.bridge.invoke('window.getState', {})
        } catch (error) {
            console.warn('Could not refresh Tauri window state', error)
        }
    }

    private async apply (patch: Parameters<HostBridge['invoke']>[1] & Record<string, unknown>): Promise<void> {
        try {
            await this.bridge.invoke('window.applyState', patch)
            await this.refreshState()
        } catch (error) {
            console.warn('Could not apply Tauri window state', error)
            await this.refreshState()
        }
    }

    private async invoke<K extends keyof import('../api/hostBridge').HostRequestMap> (
        command: K,
        request: import('../api/hostBridge').HostRequestMap[K]['request'],
    ): Promise<import('../api/hostBridge').HostRequestMap[K]['response']> {
        try {
            return await this.bridge.invoke(command, request)
        } catch (error) {
            console.warn(`Tauri host command ${String(command)} failed`, error)
            throw error
        }
    }
}
