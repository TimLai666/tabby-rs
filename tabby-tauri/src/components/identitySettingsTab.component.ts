import { Component, HostBinding, Injectable, OnInit } from '@angular/core'
import { SettingsTabProvider } from 'tabby-settings'

import { AppIdentity, CliAliasStatus, HostBridge } from '../api/hostBridge'

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
        <div *ngIf="error" class="alert alert-danger">{{ error }}</div>

        <button
            class="btn btn-primary"
            [disabled]="busy || !aliasStatus?.supported || !!aliasStatus?.conflict"
            (click)="toggleAlias()"
        >
            {{ aliasStatus?.enabled ? 'Disable tabby alias' : 'Enable tabby alias' }}
        </button>
        <span *ngIf="aliasStatus?.aliasPath" class="ms-3"><code>{{ aliasStatus?.aliasPath }}</code></span>
    `,
})
export class IdentitySettingsTabComponent implements OnInit {
    @HostBinding('class.content-box') readonly contentBox = true

    identity: AppIdentity | null = null
    aliasStatus: CliAliasStatus | null = null
    busy = false
    error: string | null = null

    constructor (private bridge: HostBridge) { }

    async ngOnInit (): Promise<void> {
        try {
            const [identity, aliasStatus] = await Promise.all([
                this.bridge.invoke('identity.get', {}),
                this.bridge.invoke('identity.aliasStatus', {}),
            ])
            this.identity = identity
            this.aliasStatus = aliasStatus
        } catch (error) {
            this.error = String(error)
        }
    }

    async toggleAlias (): Promise<void> {
        if (!this.aliasStatus || this.busy) {
            return
        }
        this.busy = true
        this.error = null
        try {
            this.aliasStatus = await this.bridge.invoke('identity.setAlias', {
                enabled: !this.aliasStatus.enabled,
            })
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
