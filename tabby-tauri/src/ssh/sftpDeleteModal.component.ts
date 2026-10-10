import { Component } from '@angular/core'
import { NgbActiveModal } from '@ng-bootstrap/ng-bootstrap'
import { BaseComponent } from 'tabby-core'
import { SFTPFile } from '../../../tabby-ssh/src/api/sftp'
import { TauriSftpPanelTransport } from './sftpPanelTransport'

@Component({
    template: require('../../../tabby-ssh/src/components/sftpDeleteModal.component.pug'),
})
export class TauriSftpDeleteModalComponent extends BaseComponent {
    sftp: TauriSftpPanelTransport
    item: SFTPFile
    progressMessage = ''
    cancelled = false
    changed = false
    readonly settled: Promise<void>
    private resolveSettled: () => void

    constructor (private modalInstance: NgbActiveModal) {
        super()
        this.settled = new Promise(resolve => { this.resolveSettled = resolve })
    }

    async ngOnInit (): Promise<void> {
        this.destroyed$.subscribe(() => { this.cancelled = true })
        try {
            await this.run(this.item)
            if (!this.isCancelled()) { this.modalInstance.close(true) }
        } catch (error) {
            if (!this.isCancelled()) { this.modalInstance.dismiss(error) }
        } finally {
            this.resolveSettled()
        }
    }

    cancel (): void {
        if (this.isCancelled()) { return }
        this.cancelled = true
        this.modalInstance.close(false)
    }

    private isCancelled (): boolean { return this.cancelled }

    async run (file: SFTPFile): Promise<void> {
        if (this.isCancelled()) { return }
        if ('isOperable' in file && !file.isOperable) {
            throw new Error('unoperableReason' in file && typeof file.unoperableReason === 'string' ? file.unoperableReason : 'This remote name cannot be deleted')
        }
        this.progressMessage = file.fullPath
        if (file.isDirectory) {
            for (const child of await this.sftp.readdir(file.fullPath)) {
                if (this.isCancelled()) { return }
                await this.run(child)
            }
        }
        if (!this.isCancelled()) {
            this.changed = true
            await this.sftp.remove(file.fullPath, false)
        }
    }
}
