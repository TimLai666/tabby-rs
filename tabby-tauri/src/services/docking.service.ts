import { Inject, Injectable } from '@angular/core'
import {
    BOOTSTRAP_DATA,
    BootstrapData,
    ConfigService,
    DockingService,
    PlatformService,
    Screen,
} from 'tabby-core'

import { HostBridge, ScreenInfo } from '../api/hostBridge'

@Injectable()
export class TauriDockingService extends DockingService {
    /** null until the native window reports its capabilities; false on Wayland and other hosts without docking. */
    dockingSupported: boolean | null = null
    private screens: ScreenInfo[] = []

    constructor (
        private bridge: HostBridge,
        private config: ConfigService,
        platform: PlatformService,
        @Inject(BOOTSTRAP_DATA) private bootstrapData: BootstrapData,
    ) {
        super()
        void this.refreshScreens()
        void this.loadDockingCapability()
        platform.displayMetricsChanged$.subscribe(() => {
            void this.refreshScreens().then(() => this.dock())
        })
    }

    dock (): void {
        // Wait for the capability; loadDockingCapability applies the dock once it is known.
        if (this.dockingSupported === null) {
            return
        }
        const appearance = this.config.store.appearance
        const side = this.bootstrapData.isMainWindow && this.dockingSupported ? appearance.dock : 'off'
        const screenId = typeof appearance.dockScreen === 'number' ? appearance.dockScreen : null

        void this.bridge.invoke('window.setDocking', {
            side,
            screenId,
            fill: appearance.dockFill,
            space: appearance.dockSpace,
            alwaysOnTop: side !== 'off' && appearance.dockAlwaysOnTop,
            minWidth: 400,
            minHeight: 300,
        }).catch(error => {
            if (side !== 'off') {
                console.info('Window docking is unavailable:', error)
            }
        })
    }

    getScreens (): Screen[] {
        return this.screens.map(screen => ({
            id: screen.id,
            name: screen.name,
        }))
    }

    private async loadDockingCapability (): Promise<void> {
        try {
            const state = await this.bridge.invoke('window.getState', {})
            this.dockingSupported = state.capabilities.docking
        } catch (error) {
            console.info('Could not read window capabilities; docking is off:', error)
            this.dockingSupported = false
        }
        if (!this.dockingSupported && this.bootstrapData.isMainWindow && this.config.store.appearance.dock !== 'off') {
            console.info('Window docking is unavailable in this display session; the dock setting is ignored')
        }
        this.dock()
    }

    private async refreshScreens (): Promise<void> {
        try {
            this.screens = await this.bridge.invoke('window.listScreens', {})
            this.screensChanged.next()
        } catch (error) {
            console.warn('Could not enumerate Tauri displays', error)
        }
    }
}
