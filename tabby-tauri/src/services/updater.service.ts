import { Injectable } from '@angular/core'
import { UpdaterService } from 'tabby-core'

@Injectable()
export class TauriUpdaterService extends UpdaterService {
    async check (): Promise<boolean> {
        return false
    }

    async update (): Promise<void> {
        throw new Error('The Tauri updater is implemented in issue #23')
    }
}
