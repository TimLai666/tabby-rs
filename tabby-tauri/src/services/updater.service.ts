import { Injectable } from '@angular/core'
import { PlatformService, TranslateService, UpdateChannel, UpdateInfo, UpdaterService } from 'tabby-core'

import { HostBridge, UpdateReleasePageDto, UpdateStateDto } from '../api/hostBridge'

// Mirrors the Unsupported code of AppError, returned when the signed updater is not configured.
function isUnsupportedError (error: unknown): boolean {
    return typeof error === 'object' && error !== null && (error as { code?: unknown }).code === 'unsupported'
}

@Injectable()
export class TauriUpdaterService extends UpdaterService {
    private updateState: UpdateStateDto = { status: 'idle' }
    // Set when the last check came from the release page fallback (tabby-electron updater.service.ts check()).
    private releasePage: { version: string, url: string } | null = null

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
        try {
            const info = await this.bridge.invoke('update.check', {})
            return info ? { ...info, downloadSize: info.downloadSize ?? undefined } : null
        } catch (error) {
            if (!isUnsupportedError(error)) {
                throw error
            }
            try {
                return await this.checkReleasePage()
            } catch (fallbackError) {
                // `unsupported` also covers a configured but unavailable signed
                // updater; keep that original reason when the fallback is refused.
                if (isUnsupportedError(fallbackError)) {
                    throw error
                }
                throw fallbackError
            }
        }
    }

    // Used only when the signed updater is not configured. Mirrors the fallback in
    // tabby-electron updater.service.ts check(), which never downloads.
    private async checkReleasePage (): Promise<UpdateInfo|null> {
        const release: UpdateReleasePageDto = await this.bridge.invoke('update.checkReleasePage', {})
        if (!release.isNewer) {
            this.releasePage = null
            return null
        }
        this.releasePage = { version: release.version, url: release.releaseUrl }
        return {
            version: release.version,
            currentVersion: this.platform.getAppVersion(),
            channel: await this.getChannel(),
            publishedAt: release.releaseDate || new Date().toISOString(),
            notes: release.releaseNotes,
            requiresConfigMigration: false,
        }
    }

    async download (info: UpdateInfo): Promise<void> {
        if (this.isReleasePageInfo(info)) {
            return
        }
        await this.bridge.invoke('update.download', { version: info.version })
    }

    async install (info: UpdateInfo): Promise<void> {
        if (this.isReleasePageInfo(info)) {
            await this.openReleasePage()
            return
        }
        if (!await this.confirmInstall()) {
            return
        }
        await this.installConfirmed(info)
    }

    override async update (info: UpdateInfo): Promise<void> {
        if (this.isReleasePageInfo(info)) {
            await this.openReleasePage()
            return
        }
        if (!await this.confirmInstall()) {
            return
        }
        await this.download(info)
        await this.installConfirmed(info)
    }

    private isReleasePageInfo (info: UpdateInfo): boolean {
        return this.releasePage?.version === info.version
    }

    // Mirrors the fallback install() in tabby-electron updater.service.ts, which opens the release page.
    private async openReleasePage (): Promise<void> {
        const url = this.releasePage?.url
        if (url) {
            await this.platform.openExternal(url)
        }
    }

    private async confirmInstall (): Promise<boolean> {
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
        return confirmation.response === 0
    }

    private async installConfirmed (info: UpdateInfo): Promise<void> {
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
        if (this.releasePage) {
            return
        }
        if (this.canCancel()) {
            await this.bridge.invoke('update.cancel', {})
        }
    }
}
