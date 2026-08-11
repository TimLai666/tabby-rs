export type CapabilityName =
    'filesystem'|'keychain'|'localPty'|'pluginInstall'|'serial'|'updater'

export class UnsupportedCapabilityError extends Error {
    readonly code = 'UNSUPPORTED_CAPABILITY'

    constructor (public readonly capability: CapabilityName) {
        super(`Capability "${capability}" is not available in this host`)
        this.name = 'UnsupportedCapabilityError'
    }
}
