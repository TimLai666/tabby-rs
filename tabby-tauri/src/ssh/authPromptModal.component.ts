import { Component } from '@angular/core'
import { NgbActiveModal } from '@ng-bootstrap/ng-bootstrap'

import { SshAuthPrompt } from '../api/hostBridge'

@Component({
    selector: 'tauri-ssh-auth-prompt',
    template: `
        <div class="modal-header"><h4 class="modal-title">SSH authentication</h4></div>
        <div class="modal-body">
            <p *ngIf="prompt.name">{{ prompt.name }}</p>
            <p *ngIf="prompt.instructions">{{ prompt.instructions }}</p>
            <div class="form-group" *ngFor="let item of prompt.prompts; let i = index">
                <label>{{ item.text }}</label>
                <input class="form-control" [type]="item.echo ? 'text' : 'password'" [(ngModel)]="responses[i]" autocomplete="off">
            </div>
        </div>
        <div class="modal-footer">
            <button class="btn btn-secondary" (click)="cancel()">Cancel</button>
            <button class="btn btn-primary" (click)="submit()">Continue</button>
        </div>
    `,
})
export class TauriSshAuthPromptModalComponent {
    prompt!: SshAuthPrompt
    responses: string[] = []

    constructor (private modal: NgbActiveModal) { }

    submit (): void {
        this.modal.close(this.responses)
    }

    cancel (): void {
        this.modal.dismiss()
    }
}
