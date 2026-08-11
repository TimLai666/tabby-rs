import { Component, EventEmitter, Input, Output } from '@angular/core'
import { DirectoryDownload, DirectoryUpload, FileUpload, NotificationsService, PlatformService } from 'tabby-core'
import { posix as posixPath } from 'path'

import { RemoteFileEntry } from '../api/hostBridge'
import { TauriSshSession } from './session'
import { TauriSftpSession } from './sftp'

@Component({
    selector: 'tauri-sftp-panel',
    template: `
        <section class="tauri-sftp-panel" (click)="$event.stopPropagation()">
            <header class="d-flex align-items-center gap-2 mb-2">
                <button class="btn btn-sm btn-secondary" type="button" (click)="goUp()">↑</button>
                <input class="form-control form-control-sm" [(ngModel)]="path" (keyup.enter)="navigate(path)">
                <button class="btn btn-sm btn-secondary" type="button" (click)="refresh()">Refresh</button>
                <button class="btn btn-sm btn-primary" type="button" (click)="upload()">Upload</button>
                <button class="btn btn-sm btn-primary" type="button" (click)="uploadFolder()">Upload folder</button>
                <button class="btn btn-sm btn-outline-primary" type="button" (click)="mkdir()">New directory</button>
                <button class="btn btn-sm btn-outline-secondary" type="button" (click)="close.emit()">Close</button>
            </header>
            <div *ngIf="error" class="alert alert-danger py-1">{{ error }}</div>
            <div *ngIf="loading" class="text-muted">Loading…</div>
            <table *ngIf="!loading" class="table table-sm table-hover mb-0">
                <thead><tr><th>Name</th><th>Size</th><th>Modified</th><th></th></tr></thead>
                <tbody>
                    <tr *ngFor="let item of files" (dblclick)="open(item)" (contextmenu)="showMenu(item, $event)">
                        <td><span [class]="item.isDirectory ? 'fas fa-folder text-info mr-2' : item.isSymlink ? 'fas fa-link text-warning mr-2' : 'fas fa-file mr-2'"></span>{{ item.name }}<small *ngIf="!item.isOperable" class="text-warning ml-2">(display-only)</small></td>
                        <td>{{ item.isDirectory ? '—' : item.size }}</td>
                        <td>{{ item.modified ? (item.modified * 1000 | date:'short') : '—' }}</td>
                        <td class="text-right"><button class="btn btn-sm btn-link" type="button" (click)="download(item)">{{ item.isDirectory ? 'Download' : 'Download' }}</button></td>
                    </tr>
                </tbody>
            </table>
        </section>
    `,
    styles: [`
        .tauri-sftp-panel { background: var(--bs-body-bg, #202124); border: 1px solid var(--bs-border-color, #555); border-radius: 4px; padding: 10px; position: absolute; inset: 10% 2% 8%; overflow: auto; z-index: 10; }
        .tauri-sftp-panel header { position: sticky; top: 0; background: var(--bs-body-bg, #202124); z-index: 1; }
        .tauri-sftp-panel td, .tauri-sftp-panel th { vertical-align: middle; }
    `],
})
export class TauriSftpPanelComponent {
    @Input() session!: TauriSshSession
    @Input() path = '/'
    @Output() pathChange = new EventEmitter<string>()
    @Output() close = new EventEmitter<void>()

    files: RemoteFileEntry[] = []
    loading = false
    error = ''
    private sftp: TauriSftpSession|null = null

    constructor (
        private platform: PlatformService,
        private notifications: NotificationsService,
    ) { }

    async ngOnInit (): Promise<void> {
        try {
            this.sftp = await this.session.openSFTP()
            await this.navigate(this.path)
        } catch (error) {
            this.showError(error)
        }
    }

    async navigate (value: string): Promise<void> {
        if (!this.sftp) { return }
        this.loading = true
        this.error = ''
        try {
            this.path = value || '/'
            this.pathChange.emit(this.path)
            this.files = (await this.sftp.readdir(this.path)).sort((left, right) =>
                Number(right.isDirectory) - Number(left.isDirectory) || left.name.localeCompare(right.name))
        } catch (error) {
            this.showError(error)
        } finally {
            this.loading = false
        }
    }

    refresh (): Promise<void> { return this.navigate(this.path) }

    goUp (): Promise<void> { return this.navigate(posixPath.dirname(this.path)) }

    async open (item: RemoteFileEntry): Promise<void> {
        if (!item.isOperable) {
            this.showError(new Error(item.unoperableReason ?? 'This remote name cannot be operated on'))
            return
        }
        if (item.isSymlink) {
            this.showError(new Error('Symbolic links are not followed by the SFTP browser'))
        } else if (item.isDirectory) {
            await this.navigate(item.fullPath)
        } else {
            await this.download(item)
        }
    }

    async upload (): Promise<void> {
        if (!this.sftp) { return }
        for (const transfer of await this.platform.startUpload({ multiple: true })) {
            await this.uploadOne(transfer)
        }
        await this.refresh()
    }

    async uploadFolder (): Promise<void> {
        if (!this.sftp) { return }
        const transfer = await this.platform.startUploadDirectory()
        await this.uploadDirectory(transfer)
        await this.refresh()
    }

    async mkdir (): Promise<void> {
        const name = window.prompt('Remote directory name')?.trim()
        if (!name || !this.sftp) { return }
        try {
            await this.sftp.mkdir(posixPath.join(this.path, name))
            await this.refresh()
        } catch (error) { this.showError(error) }
    }

    async download (item: RemoteFileEntry): Promise<void> {
        if (!this.sftp) { return }
        if (!item.isOperable) {
            this.showError(new Error(item.unoperableReason ?? 'This remote name cannot be downloaded'))
            return
        }
        if (item.isSymlink) {
            this.showError(new Error('Symbolic links are not downloaded'))
            return
        }
        if (item.isDirectory) {
            const transfer = await this.platform.startDownloadDirectory(item.name, 0)
            if (transfer) {
                try {
                    await this.downloadDirectory(item, transfer, '')
                } catch (error) {
                    transfer.cancel()
                    this.showError(error)
                } finally {
                    await transfer.closeAsync()
                }
            }
            return
        }
        const transfer = await this.platform.startDownload(item.name, item.mode, item.size)
        if (transfer) { await this.sftp.download(item.fullPath, transfer) }
    }

    async showMenu (item: RemoteFileEntry, event: MouseEvent): Promise<void> {
        event.preventDefault()
        if (!item.isOperable) {
            this.showError(new Error(item.unoperableReason ?? 'This remote name cannot be modified'))
            return
        }
        const action = window.prompt(`${item.name}\nType: rename, delete, or download`)
        if (action === 'rename') {
            const name = window.prompt('New remote name')?.trim()
            if (name && this.sftp) { await this.sftp.rename(item.fullPath, posixPath.join(this.path, name)) }
        } else if (action === 'delete') {
            const recursive = item.isDirectory && window.confirm(`Delete ${item.fullPath} recursively?`)
            if (this.sftp) { await this.sftp.remove(item.fullPath, recursive) }
        } else if (action === 'download') {
            await this.download(item)
        }
        await this.refresh()
    }

    private async uploadOne (transfer: FileUpload, remotePath = this.path): Promise<void> {
        if (!this.sftp) { return }
        const destination = posixPath.join(remotePath, transfer.getName())
        try {
            await this.sftp.upload(destination, transfer, 'skip')
        } catch (error) {
            if (String(error).includes('already exists') && window.confirm(`${destination} already exists. Overwrite it?`)) {
                await this.sftp.upload(destination, transfer, 'overwrite')
            } else {
                transfer.cancel()
                throw error
            }
        }
    }

    private async uploadDirectory (directory: DirectoryUpload, remotePath = this.path): Promise<void> {
        if (!this.sftp) { return }
        const destination = posixPath.join(remotePath, directory.getName())
        if (directory.getName()) { await this.sftp.mkdir(destination).catch(() => undefined) }
        for (const child of directory.getChildrens()) {
            if (child instanceof DirectoryUpload) { await this.uploadDirectory(child, destination) } else { await this.uploadOne(child, destination) }
        }
    }

    private async downloadDirectory (folder: RemoteFileEntry, transfer: DirectoryDownload, relativePath: string): Promise<void> {
        if (!this.sftp) { return }
        for (const item of await this.sftp.readdir(folder.fullPath)) {
            if (item.isSymlink || !item.isOperable) { continue }
            const next = relativePath ? `${relativePath}/${item.name}` : item.name
            if (item.isDirectory) {
                await transfer.createDirectory(next)
                await this.downloadDirectory(item, transfer, next)
            } else {
                const file = await transfer.createFile(next, item.mode, item.size)
                await this.sftp.download(item.fullPath, file)
            }
        }
    }

    private showError (error: unknown): void {
        const message = error instanceof Error ? error.message : String(error)
        this.error = message
        this.notifications.error(message)
    }
}
