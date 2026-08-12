import { Component, Input } from '@angular/core'
import { NgbActiveModal } from '@ng-bootstrap/ng-bootstrap'

import { SshHostKeyPrompt } from '../api/hostBridge'

@Component({
    selector: 'tauri-ssh-host-key-prompt-modal',
    template: `
        <div class="modal-header">
            <h5 class="modal-title">SSH host key verification</h5>
        </div>
        <div class="modal-body">
            <p>{{ prompt.status === 'changed' ? 'The host key changed.' : 'This host key is not known yet.' }}</p>
            <p><strong>{{ prompt.host }}:{{ prompt.port }}</strong></p>
            <p>Algorithm: {{ prompt.algorithm }}</p>
            <p class="text-break">Fingerprint: {{ prompt.fingerprintSha256 }}</p>
            <p class="text-break" *ngIf="prompt.previousFingerprints.length">
                Previous fingerprints: {{ prompt.previousFingerprints.join(', ') }}
            </p>
        </div>
        <div class="modal-footer">
            <button class="btn btn-outline-secondary" type="button" (click)="close('reject')">Reject</button>
            <button class="btn btn-primary" type="button" *ngIf="prompt.status === 'unknown'" (click)="close('once')">Accept once</button>
            <button class="btn btn-primary" type="button" *ngIf="prompt.status === 'unknown'" (click)="close('save')">Save key</button>
        </div>
    `,
})
export class TauriSshHostKeyPromptModalComponent {
    @Input() prompt!: SshHostKeyPrompt

    constructor (private activeModal: NgbActiveModal) { }

    close (decision: 'once'|'save'|'reject'): void {
        this.activeModal.close(decision)
    }
}
