import { Component, Inject, Input, Optional } from '@angular/core'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { DirectoryDownload, DirectoryUpload, FileUpload, MenuItemOptions, NotificationsService, PlatformService, TranslateService } from 'tabby-core'
import { posix as posixPath } from 'path'
import { SFTPFile } from '../../../tabby-ssh/src/api/sftp'
import { SFTPContextMenuItemProvider } from '../../../tabby-ssh/src/api/sftpContextMenu'
import { TauriSftpEditor } from './sftpEditor'
import { SFTPPanelController } from '../../../tabby-ssh/src/components/sftpPanel.controller'

import { RemoteFileEntry } from '../api/hostBridge'
import { TauriSshSession } from './session'
import { TauriSftpPanelTransport } from './sftpPanelTransport'
import { TauriSftpDeleteModalComponent } from './sftpDeleteModal.component'

@Component({
    selector: 'tauri-sftp-panel',
    template: require('../../../tabby-ssh/src/components/sftpPanel.component.pug'),
    styles: [require('../../../tabby-ssh/src/components/sftpPanel.component.scss')],
})
export class TauriSftpPanelComponent extends SFTPPanelController<TauriSftpPanelTransport> {
    @Input() session!: TauriSshSession

    constructor (
        platform: PlatformService,
        notifications: NotificationsService,
        ngbModal: NgbModal,
        private translate: TranslateService,
        @Optional() @Inject(SFTPContextMenuItemProvider) private contextMenuProviders: SFTPContextMenuItemProvider<TauriSftpPanelComponent>[]|null = [],
        @Optional() private editor?: TauriSftpEditor,
    ) {
        super(ngbModal, notifications, platform)
    }

    protected async openSFTP (): Promise<TauriSftpPanelTransport> {
        return new TauriSftpPanelTransport(await this.session.openSFTP())
    }

    override async ngOnInit (): Promise<void> {
        try { await super.ngOnInit() } catch (error) { this.showError(error) }
    }

    override getDisplayName (item: SFTPFile): string {
        return 'isOperable' in item && !item.isOperable ? `${item.name} (display-only)` : item.name
    }

    async open (item: SFTPFile|RemoteFileEntry): Promise<void> {
        if ('isOperable' in item && !item.isOperable) {
            this.showError(new Error(item.unoperableReason ?? 'This remote name cannot be operated on'))
            return
        }
        if (item.isSymlink) {
            try {
                const target = posixPath.resolve(posixPath.dirname(item.fullPath), await this.sftp.readlink(item.fullPath))
                const stat = await this.sftp.stat(target, true)
                if (stat.isDirectory) {
                    await this.navigate(item.fullPath)
                } else {
                    const transfer = await this.platform.startDownload(item.name, stat.mode, stat.size)
                    if (transfer) { await this.sftp.download(item.fullPath, transfer) }
                }
            } catch (error) {
                this.showError(error)
            }
        } else if (item.isDirectory) {
            await this.navigate(item.fullPath)
        } else {
            await this.download(item)
        }
    }

    async download (item: SFTPFile|RemoteFileEntry|string, requestedMode?: number, requestedSize?: number): Promise<void> {
        if (typeof item === 'string') {
            try {
                const transfer = await this.platform.startDownload(posixPath.basename(item), requestedMode ?? 0o644, requestedSize ?? 0)
                if (transfer) { await this.sftp.download(item, transfer) }
            } catch (error) { this.showError(error) }
            return
        }
        if ('isOperable' in item && !item.isOperable) {
            this.showError(new Error(item.unoperableReason ?? 'This remote name cannot be downloaded'))
            return
        }
        let directory = item.isDirectory
        let mode = item.mode
        let size = item.size
        if (item.isSymlink) {
            try {
                const target = posixPath.resolve(posixPath.dirname(item.fullPath), await this.sftp.readlink(item.fullPath))
                const stat = await this.sftp.stat(target, true)
                directory = stat.isDirectory
                mode = stat.mode
                size = stat.size
            } catch (error) {
                this.showError(error)
                return
            }
        }
        if (directory) {
            const folder: SFTPFile = { ...item, modified: item.modified instanceof Date ? item.modified : new Date((item.modified ?? 0) * 1000) }
            try { await super.downloadFolder(folder) } catch { /* The shared controller already reports the error. */ }
            return
        }
        try {
            const transfer = await this.platform.startDownload(item.name, mode, size)
            if (transfer) { await this.sftp.download(item.fullPath, transfer) }
        } catch (error) { this.showError(error) }
    }

    protected override async calculateFolderSizeAndUpdate (folder: SFTPFile, transfer: DirectoryDownload): Promise<number> {
        let totalSize = 0
        for (const item of await this.sftp.readdir(folder.fullPath)) {
            if (!item.isOperable) { continue }
            if (transfer.isCancelled()) { throw new Error('Download cancelled') }
            totalSize += item.isDirectory ? await this.calculateFolderSizeAndUpdate(item, transfer)
                : item.isSymlink ? (await this.sftp.stat(item.fullPath, true)).size : item.size
            transfer.setTotalSize(totalSize)
        }
        return totalSize
    }

    protected override async downloadFolderRecursive (folder: SFTPFile, transfer: DirectoryDownload, relativePath: string): Promise<void> {
        await this.downloadDirectory(folder, transfer, relativePath)
    }

    private async downloadDirectory (folder: SFTPFile|RemoteFileEntry, transfer: DirectoryDownload, relativePath: string): Promise<void> {
        for (const item of await this.sftp.readdir(folder.fullPath)) {
            if (!item.isOperable) { continue }
            if (transfer.isCancelled()) { throw new Error('Download cancelled') }
            const next = relativePath ? `${relativePath}/${item.name}` : item.name
            transfer.setStatus(next)
            if (item.isDirectory) {
                await transfer.createDirectory(next)
                await this.downloadDirectory(item, transfer, next)
            } else {
                const size = item.isSymlink ? (await this.sftp.stat(item.fullPath, true)).size : item.size
                const file = await transfer.createFile(next, item.mode, size)
                await this.sftp.download(item.fullPath, file)
            }
        }
    }

    override async downloadItem (item: SFTPFile): Promise<void> {
        await this.download(item)
    }

    override async downloadFolder (folder: SFTPFile): Promise<void> {
        await this.download(folder)
    }

    override async upload (): Promise<void> {
        try { await super.upload() } catch (error) { this.showError(error) }
    }

    override async uploadOneFolder (transfer: DirectoryUpload): Promise<void> {
        const savedPath = this.path
        try {
            await this.uploadDirectory(transfer, savedPath)
            if (this.path === savedPath) { await this.navigate(savedPath) }
        } catch (error) {
            this.cancelPendingUploads(transfer)
            this.showError(error)
        }
    }

    private cancelPendingUploads (directory: DirectoryUpload): void {
        for (const child of directory.getChildrens()) {
            if (child instanceof DirectoryUpload) {
                this.cancelPendingUploads(child)
            } else if (child.getState() === 'pending' || child.getState() === 'running') {
                child.cancel()
            }
        }
    }

    override async uploadOne (transfer: FileUpload, remotePath = this.path): Promise<void> {
        const destination = posixPath.join(remotePath, transfer.getName())
        try {
            await this.sftp.upload(destination, transfer, 'skip')
        } catch (error) {
            const message = typeof error?.details === 'string' ? error.details
                : typeof error?.message === 'string' ? error.message : String(error)
            if (!message.includes('already exists') || (await this.platform.showMessageBox({
                type: 'warning',
                message: this.translate.instant('{path} already exists. Overwrite it?', { path: destination }),
                buttons: [this.translate.instant('Overwrite'), this.translate.instant('Cancel')],
                defaultId: 0,
                cancelId: 1,
            })).response !== 0) {
                transfer.cancel()
                throw error
            }
            await this.sftp.upload(destination, transfer, 'overwrite')
        }
        if (this.path === remotePath) { await this.navigate(remotePath) }
    }

    private async uploadDirectory (directory: DirectoryUpload, remotePath: string): Promise<void> {
        const destination = posixPath.join(remotePath, directory.getName())
        if (directory.getName()) { await this.sftp.mkdir(destination).catch(() => undefined) }
        for (const child of directory.getChildrens()) {
            if (child instanceof DirectoryUpload) { await this.uploadDirectory(child, destination) } else { await this.uploadOne(child, destination) }
        }
    }

    async buildContextMenu (item: SFTPFile): Promise<MenuItemOptions[]> {
        const editorItems: MenuItemOptions[] = [
            { label: this.translate.instant('Copy full path'), click: () => this.platform.setClipboard({ text: item.fullPath, html: '' }) },
        ]
        if (!item.isDirectory && this.editor) {
            editorItems.push({ label: this.translate.instant('Edit locally'), click: () => this.editor!.edit(item, this.sftp) })
        }
        const commonItems: MenuItemOptions[] = [
            { label: this.translate.instant('Create directory'), click: () => this.openCreateDirectoryModal() },
            { label: this.translate.instant(item.isDirectory ? 'Download directory' : 'Download'), click: () => this.downloadItem(item) },
            { label: this.translate.instant('Delete'), click: async () => {
                if ((await this.platform.showMessageBox({
                    type: 'warning',
                    message: this.translate.instant('Delete {fullPath}?', item),
                    buttons: [this.translate.instant('Delete'), this.translate.instant('Cancel')],
                    defaultId: 0,
                    cancelId: 1,
                })).response !== 0) { return }
                const savedPath = this.path
                const modal = this.ngbModal.open(TauriSftpDeleteModalComponent)
                const operation: TauriSftpDeleteModalComponent = modal.componentInstance
                try {
                    operation.item = item
                    operation.sftp = this.sftp
                    await modal.result
                } catch (error) { this.showError(error) } finally {
                    await operation.settled
                    if (operation.changed && this.path === savedPath) { await this.navigate(savedPath) }
                }
            } },
        ]
        const providers = [
            { weight: 0, getItems: async () => editorItems },
            { weight: 10, getItems: async () => commonItems },
            ...this.contextMenuProviders ?? [],
        ].sort((a, b) => a.weight - b.weight)
        const sections = await Promise.all(providers.map(provider => provider.getItems(item, this)))
        return sections.flatMap(items => items.length ? [{ type: 'separator' as const }, ...items] : []).slice(1)
    }

    override async showContextMenu (item: SFTPFile, event: MouseEvent): Promise<void> {
        event.preventDefault()
        if ('isOperable' in item && !item.isOperable) {
            this.showError(new Error('unoperableReason' in item && typeof item.unoperableReason === 'string' ? item.unoperableReason : 'This remote name cannot be modified'))
            return
        }
        this.platform.popupContextMenu(await this.buildContextMenu(item), event)
    }

    private showError (error: unknown): void {
        const details = typeof error === 'object' && error !== null && 'details' in error ? error.details : null
        const message = typeof error === 'object' && error !== null && 'message' in error ? error.message : null
        this.notifications.error(typeof details === 'string' ? details : typeof message === 'string' ? message : String(error))
    }
}
