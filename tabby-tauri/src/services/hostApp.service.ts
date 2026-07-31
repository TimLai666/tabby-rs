import { Inject, Injectable, Injector } from '@angular/core'
import { HostAppService, Platform } from 'tabby-core'

import { HostBridge, RuntimeInfo, TAURI_RUNTIME_INFO } from '../api/hostBridge'

function mapPlatform (platform: string): Platform {
    switch (platform.toLowerCase()) {
        case 'windows':
        case 'win32':
            return Platform.Windows
        case 'macos':
        case 'darwin':
            return Platform.macOS
        case 'linux':
            return Platform.Linux
        default:
            return Platform.Web
    }
}

@Injectable()
export class TauriHostAppService extends HostAppService {
    readonly platform: Platform
    readonly configPlatform: Platform

    constructor (
        injector: Injector,
        private bridge: HostBridge,
        @Inject(TAURI_RUNTIME_INFO) runtimeInfo: RuntimeInfo,
    ) {
        super(injector)
        this.platform = mapPlatform(runtimeInfo.platform)
        this.configPlatform = this.platform
    }

    newWindow (): void {
        this.logger.warn('Opening additional windows is not implemented by the Tauri foundation yet')
    }

    relaunch (): void {
        window.location.reload()
    }

    quit (): void {
        void this.bridge.invoke('app.quit', {})
    }
}
