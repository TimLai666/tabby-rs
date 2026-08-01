import { VaultMutationResult } from './hostBridge'

export type SecretImportItemSource = 'vault'|'keychain'

export interface SecretImportItem {
    id: string
    source: SecretImportItemSource
    kind: string
    label: string
    requiresPassphrase: boolean
}

export interface SecretImportPlan {
    sourceDataDir: string
    items: SecretImportItem[]
    requiresAuthorization: boolean
}

export interface SecretImportSelection {
    sourceDataDir: string
    authorized: boolean
    itemIds: string[]
    sourceVaultPassphrase?: string | null
    rememberForSeconds?: number
}

export interface SecretImportFailure {
    id: string
    publicError: string
}

export interface SecretImportReport {
    imported: string[]
    requiresReentry: string[]
    failed: SecretImportFailure[]
    vaultMutation?: VaultMutationResult
}

declare module './hostBridge' {
    interface HostRequestMap {
        'secretImport.plan': {
            request: { sourceDataDir: string }
            response: SecretImportPlan
        }
        'secretImport.execute': {
            request: SecretImportSelection
            response: SecretImportReport
        }
    }
}

/**
 * Coordination boundary between config migration and the dedicated Vault/keychain implementation.
 * Implementations must never return, log, or persist plaintext secret values through this API.
 */
export abstract class SecretImporter {
    abstract plan (sourceDataDir: string): Promise<SecretImportPlan>

    abstract execute (selection: SecretImportSelection): Promise<SecretImportReport>
}
