import { Component } from '@angular/core'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { PromptModalComponent } from 'tabby-core'

import { SSHProfile } from '../../../tabby-ssh/src/api/interfaces'
import { PasswordStorageService } from '../../../tabby-ssh/src/services/passwordStorage.service'
import { HostBridge } from '../api/hostBridge'
import { TauriSshImportModalComponent } from './importModal.component'

@Component({
    selector: 'tauri-ssh-profile-settings',
    template: `
        <div class="form-group">
            <label>Host</label>
            <input class="form-control" [(ngModel)]="profile.options.host" autocomplete="off">
        </div>
        <div class="form-group">
            <label>Port</label>
            <input class="form-control" type="number" min="1" max="65535" [(ngModel)]="profile.options.port">
        </div>
        <div class="form-group">
            <label>User</label>
            <input class="form-control" [(ngModel)]="profile.options.user" autocomplete="username">
        </div>
        <div class="form-group">
            <label>Terminal type</label>
            <input class="form-control" placeholder="xterm-256color" [(ngModel)]="profile.options.term" autocomplete="off">
            <small class="form-text text-muted">Value of $TERM reported to the remote host (e.g. xterm-256color, vt100).</small>
        </div>
        <div class="form-group">
            <label>Authentication</label>
            <select class="form-control" [(ngModel)]="profile.options.auth">
                <option [ngValue]="null">None</option>
                <option value="password">Password</option>
                <option value="publicKey">Private key</option>
                <option value="agent">SSH agent</option>
                <option value="keyboardInteractive">Keyboard-interactive</option>
            </select>
        </div>
        <div class="form-group" *ngIf="profile.options.auth === 'publicKey'">
            <label>Private key paths</label>
            <textarea class="form-control" rows="3" [(ngModel)]="privateKeysText"></textarea>
            <small class="form-text text-muted">One local path per line. Vault references use vault://.</small>
        </div>
        <div class="form-group" *ngIf="profile.options.auth === 'password'">
            <button class="btn btn-secondary mr-2" type="button" (click)="setPassword()">Set saved password</button>
            <button class="btn btn-outline-secondary" type="button" *ngIf="hasSavedPassword" (click)="clearSavedPassword()">Clear saved password</button>
        </div>
        <div class="form-group">
            <label>Jump host profile ID</label>
            <input class="form-control" [(ngModel)]="profile.options.jumpHost" autocomplete="off">
            <small class="form-text text-muted">Leave empty for a direct connection.</small>
        </div>
        <div class="form-check mb-2">
            <input class="form-check-input" type="checkbox" [(ngModel)]="profile.options.agentForward" id="tauri-ssh-agent-forward">
            <label class="form-check-label" for="tauri-ssh-agent-forward">Forward SSH agent</label>
        </div>
        <div class="form-check mb-3">
            <input class="form-check-input" type="checkbox" [(ngModel)]="profile.options.x11" id="tauri-ssh-x11">
            <label class="form-check-label" for="tauri-ssh-x11">Forward X11</label>
        </div>
        <div class="form-group">
            <label>Port forwarding (JSON)</label>
            <textarea class="form-control" rows="5" [(ngModel)]="forwardedPortsText"></textarea>
            <small class="form-text text-muted">Use type Local, Remote, or Dynamic. Port 0 selects a free local port.</small>
        </div>
        <div class="form-group">
            <label>Login scripts</label>
            <textarea class="form-control" rows="5" [(ngModel)]="scriptsText"></textarea>
            <small class="form-text text-muted">JSON array with expect, send, optional, and isRegex fields.</small>
        </div>
        <div class="form-group">
            <button class="btn btn-outline-primary" type="button" (click)="importOpenSshConfig()">Import SSH config or profiles</button>
        </div>
    `,
})
export class TauriSshProfileSettingsComponent {
    profile: any
    hasSavedPassword = false
    forwardedPortsText = '[]'
    scriptsText = '[]'

    constructor (
        private passwordStorage: PasswordStorageService,
        private ngbModal: NgbModal,
        private bridge: HostBridge,
    ) { }

    async ngOnInit (): Promise<void> {
        this.forwardedPortsText = JSON.stringify(this.profile.options.forwardedPorts ?? [], null, 2)
        this.scriptsText = JSON.stringify(this.profile.options.scripts ?? [], null, 2)
        if (this.profile.options.user) {
            // A stored blank password (empty string) is still a saved password.
            const savedPassword = await this.passwordStorage.loadPassword(this.profile as SSHProfile).catch(() => null)
            this.hasSavedPassword = savedPassword != null
        }
    }

    get privateKeysText (): string {
        return this.profile.options.privateKeys.join('\n')
    }

    set privateKeysText (value: string) {
        this.profile.options.privateKeys = value
            .split(/\r?\n/g)
            .map(path => path.trim())
            .filter(Boolean)
    }

    save (): void {
        this.profile.options.port = Math.max(1, Math.min(65535, Number(this.profile.options.port) || 22))
        try {
            const forwardedPorts = JSON.parse(this.forwardedPortsText)
            if (Array.isArray(forwardedPorts)) {
                this.profile.options.forwardedPorts = forwardedPorts
            }
            const scripts = JSON.parse(this.scriptsText)
            if (Array.isArray(scripts)) {
                this.profile.options.scripts = scripts
            }
        } catch {
            window.alert('Port forwarding and login scripts must be valid JSON arrays.')
        }
    }

    async setPassword (): Promise<void> {
        const modal = this.ngbModal.open(PromptModalComponent)
        modal.componentInstance.prompt = `Password for ${this.profile.options.user}@${this.profile.options.host}`
        modal.componentInstance.password = true
        const result = await modal.result.catch(() => null)
        if (typeof result?.value !== 'string') {
            return
        }
        // An empty field is more often an accidental OK than a deliberate blank password,
        // and a stored blank is then offered on every connection, so confirm it first.
        if (result.value === '' && !await this.confirmBlankPassword()) {
            return
        }
        await this.passwordStorage.savePassword(this.profile as SSHProfile, result.value)
        this.hasSavedPassword = true
    }

    private async confirmBlankPassword (): Promise<boolean> {
        const confirmation = await this.bridge.invoke('dialog.message', {
            type: 'warning',
            message: 'Save an empty password?',
            detail: 'This only works if the server permits empty passwords. Tabby will offer it on every connection.',
            buttons: ['Save', 'Cancel'],
            defaultId: 0,
            cancelId: 1,
        })
        return confirmation.response === 0
    }

    clearSavedPassword (): void {
        this.hasSavedPassword = false
        void this.passwordStorage.deletePassword(this.profile as SSHProfile)
    }

    async importOpenSshConfig (): Promise<void> {
        const paths = await this.bridge.invoke('dialog.open', {
            multiple: false,
            directory: false,
            title: 'Import SSH configuration or profiles',
        })
        const path = paths[0]
        if (!path) {
            return
        }
        const preview = await this.bridge.invoke('ssh.importPreview', { path })
        if (!preview.profiles.length) {
            window.alert('No supported OpenSSH profiles were found.')
            return
        }
        const modal = this.ngbModal.open(TauriSshImportModalComponent, { size: 'lg' })
        modal.componentInstance.preview = preview
        const selection = await modal.result.catch(() => null) as {
            selections: { profileId: string; action: 'skip'|'duplicate'|'overwrite' }[]
        }|null
        if (!selection) {
            return
        }
        const report = await this.bridge.invoke('ssh.importApply', {
            path,
            expectedRevision: preview.revision,
            selections: selection.selections,
        })
        window.alert(`Imported ${report.imported.length} SSH profile(s). Tabby RS will reload.`)
        window.location.reload()
    }
}
