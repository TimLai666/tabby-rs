import { Component, Input, Inject, Optional } from '@angular/core'
import { NotificationsService, PlatformService, MenuItemOptions } from 'tabby-core'
import { SFTPSession, SFTPFile } from '../session/sftp'
import { SSHSession } from '../session/ssh'
import { SFTPContextMenuItemProvider } from '../api/contextMenu'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { SFTPPanelController } from './sftpPanel.controller'

@Component({
    selector: 'sftp-panel',
    templateUrl: './sftpPanel.component.pug',
    styleUrls: ['./sftpPanel.component.scss'],
})
export class SFTPPanelComponent extends SFTPPanelController<SFTPSession> {
    @Input() session: SSHSession

    constructor (
        ngbModal: NgbModal,
        notifications: NotificationsService,
        platform: PlatformService,
        @Optional() @Inject(SFTPContextMenuItemProvider) protected contextMenuProviders: SFTPContextMenuItemProvider[],
    ) {
        super(ngbModal, notifications, platform)
        this.contextMenuProviders.sort((a, b) => a.weight - b.weight)
    }

    protected openSFTP (): Promise<SFTPSession> {
        return this.session.openSFTP()
    }

    async buildContextMenu (item: SFTPFile): Promise<MenuItemOptions[]> {
        let items: MenuItemOptions[] = []
        for (const section of await Promise.all(this.contextMenuProviders.map(x => x.getItems(item, this)))) {
            items.push({ type: 'separator' })
            items = items.concat(section)
        }
        return items.slice(1)
    }
}
