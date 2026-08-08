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
    private screens: ScreenInfo[] = []

    constructor (
        private bridge: HostBridge,
        private config: ConfigService,
        platform: PlatformService,
        @Inject(BOOTSTRAP_DATA) private bootstrapData: BootstrapData,
    ) {
        super()
        void this.refreshScreens()
        platform.displayMetricsChanged$.subscribe(() => {
            void this.refreshScreens().then(() => this.dock())
        })
    }

    dock (): void {
        const appearance = this.config.store.appearance
        const side = appearance.dock
        void this.bridge.invoke('window.setDocking', {
            side,
            screenId: appearance.dockScreen ?? null,
            fill: appearance.dockFill,
            space: appearance.dockSpace,
            alwaysOnTop: side !== 'off' && appearance.dockAlwaysOnTop,
            minWidth: 400,
            minHeight: 300,
        }).catch(error => {
            if (side !== 'off' && this.bootstrapData.isMainWindow) {
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

    private async refreshScreens (): Promise<void> {
        try {
            this.screens = await this.bridge.invoke('window.listScreens', {})
            this.screensChanged.next()
        } catch (error) {
            console.warn('Could not enumerate Tauri displays', error)
        }
    }
}
