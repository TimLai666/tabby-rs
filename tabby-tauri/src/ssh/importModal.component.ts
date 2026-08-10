import { Component, Input, OnInit } from '@angular/core'
import { NgbActiveModal } from '@ng-bootstrap/ng-bootstrap'

import { SshImportPreview } from '../api/hostBridge'

type SshImportAction = 'skip'|'duplicate'|'overwrite'

@Component({
    selector: 'tauri-ssh-import-modal',
    template: `
        <div class="modal-header">
            <h5 class="modal-title">Import OpenSSH profiles</h5>
        </div>
        <div class="modal-body">
            <p>Choose how to handle profiles already in Tabby RS.</p>
            <div class="table-responsive">
                <table class="table table-sm">
                    <thead><tr><th>Name</th><th>Host</th><th>Action</th></tr></thead>
                    <tbody>
                        <tr *ngFor="let profile of preview.profiles">
                            <td>{{ profile.name }}</td>
                            <td>{{ profile.user ? profile.user + '@' : '' }}{{ profile.host }}:{{ profile.port }}</td>
                            <td>
                                <select class="form-control form-control-sm" [(ngModel)]="actions[profile.id]">
                                    <option value="duplicate">Import</option>
                                    <option value="skip" *ngIf="isConflict(profile.id)">Skip existing</option>
                                    <option value="overwrite" *ngIf="isConflict(profile.id)">Overwrite existing</option>
                                </select>
                            </td>
                        </tr>
                    </tbody>
                </table>
            </div>
        </div>
        <div class="modal-footer">
            <button class="btn btn-outline-secondary" type="button" (click)="cancel()">Cancel</button>
            <button class="btn btn-primary" type="button" (click)="apply()">Import selected profiles</button>
        </div>
    `,
})
export class TauriSshImportModalComponent implements OnInit {
    @Input() preview!: SshImportPreview
    actions: Record<string, SshImportAction> = {}
    private conflictIds = new Set<string>()

    constructor (private activeModal: NgbActiveModal) { }

    ngOnInit (): void {
        this.conflictIds = new Set(this.preview.conflicts.map(conflict => conflict.profileId))
        for (const profile of this.preview.profiles) {
            this.actions[profile.id] = this.isConflict(profile.id) ? 'skip' : 'duplicate'
        }
    }

    isConflict (profileId: string): boolean {
        return this.conflictIds.has(profileId)
    }

    cancel (): void {
        this.activeModal.dismiss()
    }

    apply (): void {
        this.activeModal.close({
            selections: Object.entries(this.actions).map(([profileId, action]) => ({ profileId, action })),
        })
    }
}
