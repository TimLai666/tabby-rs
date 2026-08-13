import { Injectable } from '@angular/core'
import { RuntimeCapabilitiesService, type RuntimeCapabilities } from 'tabby-core'

@Injectable()
export class TauriRuntimeCapabilitiesService extends RuntimeCapabilitiesService {
    readonly capabilities: RuntimeCapabilities = {
        host: 'tauri',
        localPty: true,
        filesystem: true,
        keychain: true,
        updater: true,
        pluginInstall: true,
        serial: true,
        desktopNotifications: true,
    }
}
