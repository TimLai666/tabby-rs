import { Injectable } from '@angular/core'
import { BaseTabComponent, HostAppService, MenuItemOptions, Platform, TabContextMenuItemProvider, TranslateService } from 'tabby-core'
import { TauriWinSCPService } from './services/winscp.service'
import { TauriSshTabComponent } from './ssh/tab.component'

@Injectable()
export class TauriSftpContextMenu extends TabContextMenuItemProvider {
    weight = 10

    constructor (
        private hostApp: HostAppService,
        private winscp: TauriWinSCPService,
        private translate: TranslateService,
    ) {
        super()
    }

    async getItems (tab: BaseTabComponent): Promise<MenuItemOptions[]> {
        if (!(tab instanceof TauriSshTabComponent)) { return [] }
        const items: MenuItemOptions[] = [{
            label: this.translate.instant('Open SFTP panel'),
            click: () => void tab.openSFTP(),
        }]
        if (this.hostApp.platform === Platform.Windows && this.winscp.getWinSCPPath()) {
            items.push({
                label: this.translate.instant('Launch WinSCP'),
                click: () => void tab.launchWinSCP(),
            })
        }
        return items
    }
}
