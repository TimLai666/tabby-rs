export type UpdateChannel = 'stable' | 'nightly'

export interface UpdateInfo {
    version: string
    currentVersion: string
    channel: UpdateChannel
    publishedAt: string
    notes: string
    downloadSize?: number
    requiresConfigMigration: boolean
}

export abstract class UpdaterService {
    abstract check (): Promise<UpdateInfo|null>
    abstract download (info: UpdateInfo): Promise<void>
    abstract install (info: UpdateInfo): Promise<void>
    abstract setChannel (channel: UpdateChannel): Promise<void>
    abstract getChannel (): Promise<UpdateChannel>

    canCancel (): boolean {
        return false
    }

    async cancel (): Promise<void> {
        // Hosts without an explicit cancellation path leave the update flow unchanged.
    }

    async update (info: UpdateInfo): Promise<void> {
        await this.download(info)
        await this.install(info)
    }
}
