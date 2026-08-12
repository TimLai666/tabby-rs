import { UnsupportedCapabilityError, UpdateChannel, UpdateInfo, UpdaterService } from 'tabby-core'

export class NullUpdaterService extends UpdaterService {
    async check (): Promise<UpdateInfo|null> {
        return null
    }

    async download (_info: UpdateInfo): Promise<void> {
        throw new UnsupportedCapabilityError('updater')
    }

    async install (_info: UpdateInfo): Promise<void> {
        throw new UnsupportedCapabilityError('updater')
    }

    async setChannel (_channel: UpdateChannel): Promise<void> {
        throw new UnsupportedCapabilityError('updater')
    }

    async getChannel (): Promise<UpdateChannel> {
        throw new UnsupportedCapabilityError('updater')
    }
}
