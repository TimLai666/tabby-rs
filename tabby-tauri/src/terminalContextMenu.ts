import { Injectable } from '@angular/core'
import { MenuItemOptions, NotificationsService, TranslateService } from 'tabby-core'
import { BaseTerminalTabComponent, TerminalContextMenuItemProvider } from 'tabby-terminal'

import { HostBridge } from './api/hostBridge'

@Injectable()
export class TauriExportTerminalContextMenu extends TerminalContextMenuItemProvider {
    weight = 0

    constructor (
        private bridge: HostBridge,
        private notifications: NotificationsService,
        private translate: TranslateService,
    ) {
        super()
    }

    async getItems (tab: BaseTerminalTabComponent<any>): Promise<MenuItemOptions[]> {
        return [{
            label: this.translate.instant('Export to file'),
            click: async () => {
                const path = await this.bridge.invoke('dialog.save', {
                    fileName: 'terminal.txt',
                    title: null,
                })
                if (!path || !tab.frontend) {
                    return
                }
                const transfer = await this.bridge.invoke('terminal.export', { destination: path })
                try {
                    for await (const chunk of tab.frontend.getTextChunks()) {
                        await this.bridge.invoke('transfer.write', {
                            id: transfer.id,
                            data: Array.from(chunk),
                        })
                    }
                    await this.bridge.invoke('transfer.close', { id: transfer.id })
                    this.notifications.info(this.translate.instant('Saved to {path}', { path }))
                } catch (error) {
                    await this.bridge.invoke('transfer.cancel', { id: transfer.id }).catch(() => undefined)
                    throw error
                }
            },
        }]
    }
}
