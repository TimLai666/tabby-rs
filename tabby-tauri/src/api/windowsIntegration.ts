import './hostBridge'

export interface WindowsIntegrationStatus {
    available: boolean
    clinkPath: string|null
    uacHelperPath: string|null
    warnings: string[]
}

declare module './hostBridge' {
    interface HostRequestMap {
        'windows.integrationStatus': {
            request: Record<string, never>
            response: WindowsIntegrationStatus
        }
    }
}
