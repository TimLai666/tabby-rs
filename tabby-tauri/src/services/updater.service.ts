import { Injectable } from '@angular/core'
import { UpdateChannel, UpdateInfo, UpdaterService } from 'tabby-core'

import { HostBridge } from '../api/hostBridge'

@Injectable()
export class TauriUpdaterService extends UpdaterService {
    constructor (private bridge: HostBridge) {
        super()
    }

    async check (): Promise<UpdateInfo|null> {
        const info = await this.bridge.invoke('update.check', {})
        return info ? { ...info, downloadSize: info.downloadSize ?? undefined } : null
    }

    async download (info: UpdateInfo): Promise<void> {
        await this.bridge.invoke('update.download', { version: info.version })
    }

    async install (info: UpdateInfo): Promise<void> {
        await this.bridge.invoke('update.install', { version: info.version })
    }

    async setChannel (channel: UpdateChannel): Promise<void> {
        await this.bridge.invoke('update.setChannel', { channel })
    }

    async getChannel (): Promise<UpdateChannel> {
        return await this.bridge.invoke('update.getChannel', {})
    }
}
