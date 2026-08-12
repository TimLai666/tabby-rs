import { Injectable } from '@angular/core'
import { RuntimeCapabilitiesService } from 'tabby-core'
import type { RuntimeCapabilities } from 'tabby-core'

@Injectable()
export class WebRuntimeCapabilitiesService extends RuntimeCapabilitiesService {
    readonly capabilities: RuntimeCapabilities = {
        host: 'web',
        localPty: false,
        filesystem: false,
        keychain: false,
        updater: false,
        pluginInstall: false,
        serial: false,
        desktopNotifications: false,
    }
}
