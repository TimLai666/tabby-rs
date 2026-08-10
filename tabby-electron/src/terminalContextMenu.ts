import * as fs from 'fs'
import { Injectable } from '@angular/core'
import { MenuItemOptions, NotificationsService, TranslateService } from 'tabby-core'
import { BaseTerminalTabComponent, TerminalContextMenuItemProvider } from 'tabby-terminal'
import { ElectronService } from './services/electron.service'

/** @hidden */
@Injectable()
export class ExportTerminalContextMenu extends TerminalContextMenuItemProvider {
    weight = 0

    constructor (
        private electron: ElectronService,
        private notifications: NotificationsService,
        private translate: TranslateService,
    ) {
        super()
    }

    async getItems (tab: BaseTerminalTabComponent<any>): Promise<MenuItemOptions[]> {
        return [
            {
                label: this.translate.instant('Export to file'),
                click: async () => {
                    const frontend = tab.frontend
                    if (!frontend) {
                        return
                    }
                    const result = await this.electron.dialog.showSaveDialog({
                        defaultPath: 'terminal.txt',
                    })
                    if (!result.filePath) {
                        return
                    }
                    const file = await fs.promises.open(result.filePath, 'w')
                    try {
                        for await (const chunk of frontend.getTextChunks()) {
                            await file.write(chunk)
                        }
                    } finally {
                        await file.close()
                    }
                    this.notifications.info(this.translate.instant('Saved to {path}', { path: result.filePath }))
                },
            },
        ]
    }
}
