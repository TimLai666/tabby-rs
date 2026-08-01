import './hostBridge'

export interface CredentialAddress {
    service: string
    account: string
}

declare module './hostBridge' {
    interface HostRequestMap {
        'keychain.get': {
            request: CredentialAddress
            response: string | null
        }
        'keychain.put': {
            request: CredentialAddress & { value: string }
            response: null
        }
        'keychain.delete': {
            request: CredentialAddress
            response: boolean
        }
    }
}

export { }
