import { Injectable } from '@angular/core'
import { ConfigService } from 'tabby-core'

import { HostBridge } from '../api/hostBridge'
import {
    SecretImporter,
    SecretImportPlan,
    SecretImportReport,
    SecretImportSelection,
} from '../api/secretImporter'
import { TauriVaultService } from './vault.service'

@Injectable({ providedIn: 'root' })
export class TauriSecretImporter extends SecretImporter {
    constructor (
        private bridge: HostBridge,
        private config: ConfigService,
        private vault: TauriVaultService,
    ) {
        super()
    }

    plan (sourceDataDir: string): Promise<SecretImportPlan> {
        return this.bridge.invoke('secretImport.plan', { sourceDataDir })
    }

    async execute (selection: SecretImportSelection): Promise<SecretImportReport> {
        const rememberForSeconds = selection.rememberForSeconds ?? 300
        const report = await this.bridge.invoke('secretImport.execute', {
            ...selection,
            rememberForSeconds,
        })
        if (report.vaultMutation) {
            this.vault.acceptImportedMutation(report.vaultMutation, rememberForSeconds)
            this.config.store.vault = report.vaultMutation.stored
            await this.config.save()
        }
        return report
    }
}
