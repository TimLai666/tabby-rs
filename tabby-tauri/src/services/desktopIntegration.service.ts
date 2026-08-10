import { Injectable } from '@angular/core'
import { AppService, ConfigService, DockingService } from 'tabby-core'
import { auditTime } from 'rxjs'

import { HostBridge, WindowStatePatch } from '../api/hostBridge'
import { TauriHostWindowService } from './hostWindow.service'

@Injectable()
export class TauriDesktopIntegrationService {
    private lastProgress: number | null = null

    constructor (
        private app: AppService,
        private bridge: HostBridge,
        private config: ConfigService,
        private docking: DockingService,
        private hostWindow: TauriHostWindowService,
    ) { }

    async initialize (): Promise<void> {
        await this.config.ready$.toPromise()

        this.docking.dock()
        await Promise.all([
            this.registerGlobalHotkey(),
            this.applyAppearance(),
        ])

        this.hostWindow.windowShown$.subscribe(() => this.docking.dock())
        this.config.changed$.subscribe(() => {
            this.docking.dock()
            void this.registerGlobalHotkey()
            void this.applyAppearance()
        })

        this.app.tabOpened$.subscribe(tab => {
            tab.progress$.pipe(auditTime(250)).subscribe(progress => {
                if (progress === this.lastProgress) {
                    return
                }
                this.hostWindow.setProgressBar(progress === null ? -1 : progress / 100)
                this.lastProgress = progress
            })
        })

        await this.bridge.listen('desktop.windowFocused', focused => {
            if (focused) {
                return
            }
            const appearance = this.config.store.appearance
            if (appearance.dock !== 'off' && appearance.dockHideOnBlur) {
                void this.bridge.invoke('window.applyState', { visible: false })
            }
        })
    }

    private async registerGlobalHotkey (): Promise<void> {
        let configured = this.config.store.hotkeys['toggle-window'] || []
        if (typeof configured === 'string') {
            configured = [configured]
        }

        const accelerators: string[] = []
        for (const item of configured) {
            const stroke = typeof item === 'string' ? item : item[0]
            if (stroke) {
                accelerators.push(stroke)
            }
        }

        try {
            await this.bridge.invoke('hotkey.replace', {
                id: 'toggle-window',
                accelerators,
            })
        } catch (error) {
            console.info('Global toggle shortcut is unavailable:', error)
        }
    }

    private async applyAppearance (): Promise<void> {
        const appearance = this.config.store.appearance
        const state = await this.bridge.invoke('window.getState', {})
        const patch: WindowStatePatch = {}

        const mode = appearance.colorSchemeMode
        patch.colorScheme = mode === 'light' || mode === 'dark' ? mode : 'system'

        if (state.capabilities.opacity) {
            patch.opacity = appearance.opacity
        }
        if (state.capabilities.vibrancy) {
            patch.vibrancy = {
                enabled: appearance.vibrancy,
                effect: appearance.vibrancyType || null,
            }
        }

        try {
            await this.bridge.invoke('window.applyState', patch)
        } catch (error) {
            console.info('Optional desktop window effects are unavailable:', error)
        }
    }
}
