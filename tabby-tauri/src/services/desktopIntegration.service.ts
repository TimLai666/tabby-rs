import { Injectable } from '@angular/core'
import { AppService, ConfigService, DockingService, aggregateTabProgress, TabProgressState } from 'tabby-core'
import { auditTime, Subscription } from 'rxjs'

import { HostBridge, WindowStatePatch } from '../api/hostBridge'
import { TauriHostWindowService } from './hostWindow.service'

@Injectable()
export class TauriDesktopIntegrationService {
    private lastProgress = ''

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

        const progressStates = new Map<string, TabProgressState>()
        const progressSubscriptions = new Map<string, Subscription>()
        const refreshProgress = () => {
            const aggregate = aggregateTabProgress(this.app.tabs.map(tab => ({
                tabId: tab.tabId,
                active: tab === this.app.activeTab,
                progress: progressStates.get(tab.tabId) ?? { value: null, state: 'none', source: 'process' },
            })))
            const value = aggregate.state === 'normal' && aggregate.value !== null ? aggregate.value / 100 : -1
            const key = `${aggregate.state}:${value}`
            if (key === this.lastProgress) {
                return
            }
            this.hostWindow.setProgressBar(value)
            this.lastProgress = key
        }
        const trackProgress = tab => {
            progressSubscriptions.get(tab.tabId)?.unsubscribe()
            progressSubscriptions.set(tab.tabId, tab.progressState$.pipe(auditTime(250)).subscribe(progress => {
                progressStates.set(tab.tabId, progress)
                refreshProgress()
            }))
        }
        this.app.tabs.forEach(trackProgress)
        this.app.tabOpened$.subscribe(trackProgress)
        this.app.tabRemoved$.subscribe(tab => {
            progressSubscriptions.get(tab.tabId)?.unsubscribe()
            progressSubscriptions.delete(tab.tabId)
            progressStates.delete(tab.tabId)
            refreshProgress()
        })
        this.app.activeTabChange$.subscribe(refreshProgress)

        await this.bridge.listen('desktop:windowFocused', focused => {
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
