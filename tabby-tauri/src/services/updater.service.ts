import { Injectable } from '@angular/core'
import { PlatformService, TranslateService, UpdateChannel, UpdateInfo, UpdaterService } from 'tabby-core'

import { HostBridge, UpdateStateDto } from '../api/hostBridge'

@Injectable()
export class TauriUpdaterService extends UpdaterService {
    private updateState: UpdateStateDto = { status: 'idle' }

    constructor (
        private bridge: HostBridge,
        private platform: PlatformService,
        private translate: TranslateService,
    ) {
        super()
        void this.bridge.listen('update:state', state => {
            this.updateState = state
        })
    }

    async check (): Promise<UpdateInfo|null> {
        const info = await this.bridge.invoke('update.check', {})
        return info ? { ...info, downloadSize: info.downloadSize ?? undefined } : null
    }

    async download (info: UpdateInfo): Promise<void> {
        await this.bridge.invoke('update.download', { version: info.version })
    }

    async install (info: UpdateInfo): Promise<void> {
        const confirmation = await this.platform.showMessageBox({
            type: 'warning',
            message: this.translate.instant('Installing the update will close all tabs and restart Tabby RS.'),
            buttons: [
                this.translate.instant('Update'),
                this.translate.instant('Cancel'),
            ],
            defaultId: 0,
            cancelId: 1,
        })
        if (confirmation.response !== 0) {
            return
        }
        await this.bridge.invoke('update.install', { version: info.version })
    }

    async setChannel (channel: UpdateChannel): Promise<void> {
        await this.bridge.invoke('update.setChannel', { channel })
    }

    async getChannel (): Promise<UpdateChannel> {
        return this.bridge.invoke('update.getChannel', {})
    }

    override canCancel (): boolean {
        return this.updateState.status === 'checking' || this.updateState.status === 'downloading'
    }

    override async cancel (): Promise<void> {
        if (this.canCancel()) {
            await this.bridge.invoke('update.cancel', {})
        }
    }
}
