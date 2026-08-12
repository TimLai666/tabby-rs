export type CapabilityName =
    'filesystem'|'keychain'|'localPty'|'pluginInstall'|'serial'|'updater'|'desktopNotifications'

export type RuntimeHost = 'electron' | 'tauri' | 'web'

export interface RuntimeCapabilities {
    host: RuntimeHost
    localPty: boolean
    filesystem: boolean
    keychain: boolean
    updater: boolean
    pluginInstall: boolean
    serial: boolean
    desktopNotifications: boolean
}

export abstract class RuntimeCapabilitiesService {
    abstract readonly capabilities: RuntimeCapabilities
}

export class UnsupportedCapabilityError extends Error {
    readonly code = 'UNSUPPORTED_CAPABILITY'

    constructor (public readonly capability: CapabilityName) {
        super(`Capability "${capability}" is not available in this host`)
        this.name = 'UnsupportedCapabilityError'
    }
}
