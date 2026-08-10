import { Injectable, Injector } from '@angular/core'
import { NewTabParameters, ProfilesService, RecoveryToken, TabRecoveryProvider } from 'tabby-core'

import { TauriSshTabComponent } from './tab.component'

@Injectable()
export class TauriSshTabRecoveryProvider extends TabRecoveryProvider<TauriSshTabComponent> {
    constructor (private injector: Injector) { super() }

    async applicableTo (recoveryToken: RecoveryToken): Promise<boolean> {
        return recoveryToken.type === 'app:ssh-tab'
    }

    async recover (recoveryToken: RecoveryToken): Promise<NewTabParameters<TauriSshTabComponent>> {
        return {
            type: TauriSshTabComponent,
            inputs: {
                profile: this.injector.get(ProfilesService).getConfigProxyForProfile(recoveryToken.profile),
                savedState: recoveryToken.savedState,
            },
        }
    }
}
