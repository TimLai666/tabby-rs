import { UnsupportedCapabilityError, UpdaterService } from 'tabby-core'

export class NullUpdaterService extends UpdaterService {
    async check (): Promise<boolean> {
        return false
    }

    async update (): Promise<void> {
        throw new UnsupportedCapabilityError('updater')
    }
}
