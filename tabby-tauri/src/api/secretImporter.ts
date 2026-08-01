import { SecretReference } from './hostBridge'

export interface SecretImportPlan {
    sourceDataDir: string
    references: SecretReference[]
    requiresAuthorization: boolean
}

export interface SecretImportSelection {
    sourceDataDir: string
    references: SecretReference[]
}

export interface SecretImportFailure {
    path: string
    publicError: string
}

export interface SecretImportReport {
    imported: string[]
    requiresReentry: string[]
    failed: SecretImportFailure[]
}

/**
 * Coordination boundary between config migration and the dedicated Vault/keychain implementation.
 * Implementations must never return, log, or persist plaintext secret values through this API.
 */
export abstract class SecretImporter {
    abstract plan (
        sourceDataDir: string,
        references: SecretReference[],
    ): Promise<SecretImportPlan>

    abstract execute (selection: SecretImportSelection): Promise<SecretImportReport>
}
