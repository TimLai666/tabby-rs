import { Injectable } from '@angular/core'
import { RuntimeCapabilitiesService } from 'tabby-core'
import type { RuntimeCapabilities } from 'tabby-core'

@Injectable()
export class ElectronRuntimeCapabilitiesService extends RuntimeCapabilitiesService {
    readonly capabilities: RuntimeCapabilities = {
        host: 'electron',
        localPty: true,
        filesystem: true,
        keychain: true,
        updater: true,
        pluginInstall: true,
        serial: true,
        desktopNotifications: true,
    }
}
