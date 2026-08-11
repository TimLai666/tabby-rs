import { Injectable } from '@angular/core'
import { MenuItemOptions, TranslateService } from 'tabby-core'
import { BaseTerminalTabComponent, TerminalContextMenuItemProvider } from 'tabby-terminal'

@Injectable()
export class TauriSftpContextMenu extends TerminalContextMenuItemProvider {
    weight = 10

    constructor (private translate: TranslateService) {
        super()
    }

    async getItems (tab: BaseTerminalTabComponent<any>): Promise<MenuItemOptions[]> {
        const sshTab = tab as BaseTerminalTabComponent<any> & { openSFTP?: () => Promise<void>; session?: { open?: boolean } }
        if (!sshTab.session.open || !sshTab.openSFTP) { return [] }
        return [{
            label: this.translate.instant('Open SFTP panel'),
            click: () => void sshTab.openSFTP?.(),
        }]
    }
}
