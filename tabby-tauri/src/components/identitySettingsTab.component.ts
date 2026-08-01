import { Component, HostBinding, Injectable, OnInit } from '@angular/core'
import { SettingsTabProvider } from 'tabby-settings'

import {
    AppIdentity,
    BackupManifest,
    CliAliasStatus,
    HostBridge,
    ImportPlan,
    ImportReport,
} from '../api/hostBridge'
import {
    SecretImporter,
    SecretImportItem,
    SecretImportPlan,
    SecretImportReport,
} from '../api/secretImporter'

@Component({
    selector: 'tabby-rs-identity-settings-tab',
    template: `
        <h3>Tabby RS</h3>

        <div *ngIf="identity" class="mb-4">
            <div class="mb-2"><strong>Application ID</strong><br><code>{{ identity.appIdentifier }}</code></div>
            <div class="mb-2"><strong>CLI</strong><br><code>{{ identity.cliName }}</code></div>
            <div class="mb-2"><strong>URL scheme</strong><br><code>{{ identity.urlScheme }}://</code></div>
            <div class="mb-2"><strong>Data directory</strong><br><code>{{ identity.dataDir }}</code></div>
            <div *ngIf="identity.portable" class="alert alert-info">
                Portable mode is active. All Tabby RS data stays under
                <code>{{ identity.portableRoot }}</code>.
            </div>
        </div>

        <div *ngIf="error" class="alert alert-danger">{{ error }}</div>

        <h4>Optional <code>tabby</code> command</h4>
        <p>
            The default command remains <code>tabby-rs</code>. The shorter alias is created only
            when this executable is already on PATH and no other <code>tabby</code> command exists.
        </p>
        <div *ngIf="aliasStatus?.conflict" class="alert alert-warning">
            Existing command: <code>{{ aliasStatus?.conflict }}</code>. Tabby RS will not overwrite it.
        </div>
        <div *ngIf="aliasStatus?.message" class="alert alert-secondary">
            {{ aliasStatus?.message }}
        </div>
        <button
            class="btn btn-primary"
            [disabled]="busy || !aliasStatus?.supported || !!aliasStatus?.conflict"
            (click)="toggleAlias()"
        >
            {{ aliasStatus?.enabled ? 'Disable tabby alias' : 'Enable tabby alias' }}
        </button>
        <span *ngIf="aliasStatus?.aliasPath" class="ms-3"><code>{{ aliasStatus?.aliasPath }}</code></span>

        <hr class="my-4">
        <h4>Import from Tabby</h4>
        <p>
            Import is one-way. The original Tabby directory is read-only and remains unchanged.
            Secret values are never included in the configuration migration report.
        </p>
        <div *ngIf="!importPlans.length" class="alert alert-secondary">
            No readable Tabby configuration was detected.
        </div>
        <div *ngFor="let plan of importPlans" class="card mb-3">
            <div class="card-body">
                <h5 class="card-title">Detected Tabby data</h5>
                <p class="text-break"><code>{{ plan.sourceDataDir }}</code></p>
                <div class="form-check mb-2">
                    <input
                        class="form-check-input"
                        type="checkbox"
                        [checked]="isConfigSelected(plan)"
                        [disabled]="busy"
                        (change)="toggleConfig(plan)"
                    >
                    <label class="form-check-label">
                        Configuration, including {{ plan.profiles }} detected profile(s)
                    </label>
                </div>
                <div *ngIf="plan.plugins.length" class="mb-2">
                    <strong>Plugins to reinstall later</strong>
                    <div *ngFor="let plugin of plan.plugins" class="form-check">
                        <input
                            class="form-check-input"
                            type="checkbox"
                            [checked]="isPluginSelected(plan, plugin)"
                            [disabled]="busy"
                            (change)="togglePlugin(plan, plugin)"
                        >
                        <label class="form-check-label"><code>{{ plugin }}</code></label>
                    </div>
                </div>
                <div *ngIf="plan.secretReferences.length" class="alert alert-warning mt-3">
                    {{ plan.secretReferences.length }} secret-bearing field(s) were detected.
                    Use the separate authorized secret import below after configuration migration.
                </div>
                <button
                    class="btn btn-primary"
                    [disabled]="busy || !hasImportSelection(plan)"
                    (click)="runImport(plan)"
                >
                    Import selected configuration items
                </button>
            </div>
        </div>
        <div *ngIf="lastImportReport" class="alert alert-success">
            Imported {{ lastImportReport.imported.length }} item(s).
            Report: <code>{{ lastImportReport.reportPath }}</code>
            <div *ngIf="lastImportReport.requiresSecretReentry.length" class="mt-2">
                Secret fields requiring review: {{ lastImportReport.requiresSecretReentry.length }}
            </div>
        </div>

        <h4 class="mt-4">Authorized secret import</h4>
        <p>
            Planning reads metadata only. Secret values are read only after you select items,
            enable authorization, and confirm the import. Original Vault and keychain entries stay unchanged.
        </p>
        <div *ngIf="!secretPlans.length && importPlans.length" class="alert alert-secondary">
            No transferable Vault or keychain item was detected.
        </div>
        <div *ngFor="let plan of secretPlans" class="border rounded p-3 mb-3">
            <div class="text-break mb-2"><code>{{ plan.sourceDataDir }}</code></div>
            <div *ngFor="let item of plan.items" class="form-check mb-2">
                <input
                    class="form-check-input"
                    type="checkbox"
                    [checked]="isSecretItemSelected(plan, item)"
                    [disabled]="busy"
                    (change)="toggleSecretItem(plan, item)"
                >
                <label class="form-check-label">
                    {{ item.label }}
                    <span class="badge bg-secondary ms-2">{{ item.kind }}</span>
                </label>
            </div>
            <div *ngIf="hasVaultSelection(plan)" class="mb-3">
                <label class="form-label">Original Vault master password</label>
                <input
                    class="form-control"
                    type="password"
                    autocomplete="current-password"
                    [value]="vaultPassphrases.get(plan.sourceDataDir) ?? ''"
                    [disabled]="busy"
                    (input)="setVaultPassphrase(plan, $event)"
                >
                <div class="form-text">
                    The password is sent only to the Rust import command and is not written to logs or reports.
                </div>
            </div>
            <div class="form-check mb-3">
                <input
                    class="form-check-input"
                    type="checkbox"
                    [checked]="isSecretImportAuthorized(plan)"
                    [disabled]="busy"
                    (change)="toggleSecretImportAuthorization(plan)"
                >
                <label class="form-check-label">
                    I authorize Tabby RS to read the selected original secrets once and copy them to its own storage.
                </label>
            </div>
            <button
                class="btn btn-warning"
                [disabled]="busy || !canRunSecretImport(plan)"
                (click)="runSecretImport(plan)"
            >
                Import selected secrets
            </button>
        </div>
        <div *ngIf="lastSecretImportReport" class="alert alert-info">
            Imported {{ lastSecretImportReport.imported.length }} secret item(s).
            <div *ngIf="lastSecretImportReport.requiresReentry.length">
                Requires re-entry: {{ lastSecretImportReport.requiresReentry.length }}
            </div>
            <div *ngIf="lastSecretImportReport.failed.length">
                Failed: {{ lastSecretImportReport.failed.length }}
                <ul class="mb-0 mt-2">
                    <li *ngFor="let failure of lastSecretImportReport.failed">
                        <code>{{ failure.id }}</code>: {{ failure.publicError }}
                    </li>
                </ul>
            </div>
        </div>

        <hr class="my-4">
        <h4>Versioned backups</h4>
        <p>
            Backups contain only Tabby RS managed configuration and internal state.
            Restore verifies every checksum and creates another safety backup first.
        </p>
        <button class="btn btn-secondary mb-3" [disabled]="busy" (click)="createBackup()">
            Create backup now
        </button>
        <div *ngIf="!backups.length" class="alert alert-secondary">No backups yet.</div>
        <div *ngFor="let backup of backups" class="border rounded p-3 mb-2">
            <div><strong>{{ backup.reason }}</strong></div>
            <div>{{ backup.createdAt | date:'medium' }}</div>
            <div><code>{{ backup.backupId }}</code></div>
            <div>{{ backup.files.length }} stored file(s), {{ backup.absent.length }} absent marker(s)</div>
            <button
                class="btn btn-sm btn-outline-warning mt-2"
                [disabled]="busy"
                (click)="restoreBackup(backup)"
            >
                Restore this backup
            </button>
        </div>
    `,
})
export class IdentitySettingsTabComponent implements OnInit {
    @HostBinding('class.content-box') readonly contentBox = true

    identity: AppIdentity | null = null
    aliasStatus: CliAliasStatus | null = null
    importPlans: ImportPlan[] = []
    secretPlans: SecretImportPlan[] = []
    backups: BackupManifest[] = []
    lastImportReport: ImportReport | null = null
    lastSecretImportReport: SecretImportReport | null = null
    busy = false
    error: string | null = null
    vaultPassphrases = new Map<string, string>()

    private selectedConfig = new Set<string>()
    private selectedPlugins = new Map<string, Set<string>>()
    private selectedSecretItems = new Map<string, Set<string>>()
    private authorizedSecretSources = new Set<string>()

    constructor (
        private bridge: HostBridge,
        private secretImporter: SecretImporter,
    ) { }

    async ngOnInit (): Promise<void> {
        await this.refreshAll()
    }

    isConfigSelected (plan: ImportPlan): boolean {
        return this.selectedConfig.has(plan.sourceDataDir)
    }

    toggleConfig (plan: ImportPlan): void {
        if (this.selectedConfig.has(plan.sourceDataDir)) {
            this.selectedConfig.delete(plan.sourceDataDir)
        } else {
            this.selectedConfig.add(plan.sourceDataDir)
        }
    }

    isPluginSelected (plan: ImportPlan, plugin: string): boolean {
        return this.selectedPlugins.get(plan.sourceDataDir)?.has(plugin) ?? false
    }

    togglePlugin (plan: ImportPlan, plugin: string): void {
        const plugins = this.selectedPlugins.get(plan.sourceDataDir) ?? new Set<string>()
        if (plugins.has(plugin)) {
            plugins.delete(plugin)
        } else {
            plugins.add(plugin)
        }
        this.selectedPlugins.set(plan.sourceDataDir, plugins)
    }

    hasImportSelection (plan: ImportPlan): boolean {
        return this.isConfigSelected(plan)
            || (this.selectedPlugins.get(plan.sourceDataDir)?.size ?? 0) > 0
    }

    isSecretItemSelected (plan: SecretImportPlan, item: SecretImportItem): boolean {
        return this.selectedSecretItems.get(plan.sourceDataDir)?.has(item.id) ?? false
    }

    toggleSecretItem (plan: SecretImportPlan, item: SecretImportItem): void {
        const items = this.selectedSecretItems.get(plan.sourceDataDir) ?? new Set<string>()
        if (items.has(item.id)) {
            items.delete(item.id)
        } else {
            items.add(item.id)
        }
        this.selectedSecretItems.set(plan.sourceDataDir, items)
    }

    hasVaultSelection (plan: SecretImportPlan): boolean {
        return plan.items.some(item => item.source === 'vault' && this.isSecretItemSelected(plan, item))
    }

    setVaultPassphrase (plan: SecretImportPlan, event: Event): void {
        this.vaultPassphrases.set(
            plan.sourceDataDir,
            (event.target as HTMLInputElement).value,
        )
    }

    isSecretImportAuthorized (plan: SecretImportPlan): boolean {
        return this.authorizedSecretSources.has(plan.sourceDataDir)
    }

    toggleSecretImportAuthorization (plan: SecretImportPlan): void {
        if (this.authorizedSecretSources.has(plan.sourceDataDir)) {
            this.authorizedSecretSources.delete(plan.sourceDataDir)
        } else {
            this.authorizedSecretSources.add(plan.sourceDataDir)
        }
    }

    canRunSecretImport (plan: SecretImportPlan): boolean {
        const selected = this.selectedSecretItems.get(plan.sourceDataDir)?.size ?? 0
        const hasPassphrase = !this.hasVaultSelection(plan)
            || !!this.vaultPassphrases.get(plan.sourceDataDir)
        return selected > 0 && this.isSecretImportAuthorized(plan) && hasPassphrase
    }

    async toggleAlias (): Promise<void> {
        if (!this.aliasStatus || this.busy) {
            return
        }
        await this.runBusy(async () => {
            this.aliasStatus = await this.bridge.invoke('identity.setAlias', {
                enabled: !this.aliasStatus!.enabled,
            })
        })
    }

    async runImport (plan: ImportPlan): Promise<void> {
        const plugins = [...this.selectedPlugins.get(plan.sourceDataDir) ?? new Set<string>()]
        const config = this.isConfigSelected(plan)
        const summary = [
            config ? `configuration with ${plan.profiles} profile(s)` : null,
            plugins.length ? `${plugins.length} plugin name(s)` : null,
        ].filter(Boolean).join(' and ')
        if (!window.confirm(`Import ${summary} from the detected Tabby directory?`)) {
            return
        }
        await this.runBusy(async () => {
            this.lastImportReport = await this.bridge.invoke('migration.execute', {
                sourceDataDir: plan.sourceDataDir,
                config,
                plugins,
            })
            window.alert('Import completed. Tabby RS will reload the imported configuration.')
            window.location.reload()
        })
    }

    async runSecretImport (plan: SecretImportPlan): Promise<void> {
        const itemIds = [...this.selectedSecretItems.get(plan.sourceDataDir) ?? new Set<string>()]
        if (!window.confirm(
            `Authorize one-time access to ${itemIds.length} selected original secret item(s)?`,
        )) {
            return
        }
        await this.runBusy(async () => {
            this.lastSecretImportReport = await this.secretImporter.execute({
                sourceDataDir: plan.sourceDataDir,
                authorized: true,
                itemIds,
                sourceVaultPassphrase: this.hasVaultSelection(plan)
                    ? this.vaultPassphrases.get(plan.sourceDataDir)
                    : null,
                rememberForSeconds: 300,
            })
            this.vaultPassphrases.delete(plan.sourceDataDir)
            this.authorizedSecretSources.delete(plan.sourceDataDir)
        })
    }

    async createBackup (): Promise<void> {
        await this.runBusy(async () => {
            await this.bridge.invoke('backup.create', { reason: 'manual' })
            this.backups = await this.bridge.invoke('backup.list', {})
        })
    }

    async restoreBackup (backup: BackupManifest): Promise<void> {
        if (!window.confirm(`Restore backup ${backup.backupId}? A safety backup will be created first.`)) {
            return
        }
        await this.runBusy(async () => {
            await this.bridge.invoke('backup.restore', { backupId: backup.backupId })
            window.alert('Backup restored. Tabby RS will reload the restored configuration.')
            window.location.reload()
        })
    }

    private async refreshAll (): Promise<void> {
        await this.runBusy(async () => {
            const [identity, aliasStatus, importPlans, backups] = await Promise.all([
                this.bridge.invoke('identity.get', {}),
                this.bridge.invoke('identity.aliasStatus', {}),
                this.bridge.invoke('migration.detect', {}),
                this.bridge.invoke('backup.list', {}),
            ])
            const secretPlans = await Promise.all(importPlans.map(async plan => {
                try {
                    return await this.secretImporter.plan(plan.sourceDataDir)
                } catch {
                    return null
                }
            }))
            this.identity = identity
            this.aliasStatus = aliasStatus
            this.importPlans = importPlans
            this.secretPlans = secretPlans.filter((plan): plan is SecretImportPlan => !!plan?.items.length)
            this.backups = backups
            this.resetImportSelection(importPlans)
            this.resetSecretImportSelection(this.secretPlans)
        })
    }

    private resetImportSelection (plans: ImportPlan[]): void {
        this.selectedConfig.clear()
        this.selectedPlugins.clear()
        for (const plan of plans) {
            if (plan.config) {
                this.selectedConfig.add(plan.sourceDataDir)
            }
            this.selectedPlugins.set(plan.sourceDataDir, new Set(plan.plugins))
        }
    }

    private resetSecretImportSelection (plans: SecretImportPlan[]): void {
        this.selectedSecretItems.clear()
        this.authorizedSecretSources.clear()
        this.vaultPassphrases.clear()
        for (const plan of plans) {
            this.selectedSecretItems.set(
                plan.sourceDataDir,
                new Set(plan.items.map(item => item.id)),
            )
        }
    }

    private async runBusy (operation: () => Promise<void>): Promise<void> {
        if (this.busy) {
            return
        }
        this.busy = true
        this.error = null
        try {
            await operation()
        } catch (error) {
            this.error = String(error)
        } finally {
            this.busy = false
        }
    }
}

@Injectable()
export class IdentitySettingsTabProvider extends SettingsTabProvider {
    id = 'tabby-rs'
    icon = 'terminal'
    title = 'Tabby RS'
    weight = 50

    getComponentType (): any {
        return IdentitySettingsTabComponent
    }
}
