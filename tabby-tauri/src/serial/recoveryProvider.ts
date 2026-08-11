import { Injectable, Injector } from '@angular/core'
import { NewTabParameters, ProfilesService, RecoveryToken, TabRecoveryProvider } from 'tabby-core'

import { TauriSerialTabComponent } from './tab.component'

@Injectable()
export class TauriSerialTabRecoveryProvider extends TabRecoveryProvider<TauriSerialTabComponent> {
    constructor (private injector: Injector) { super() }

    async applicableTo (token: RecoveryToken): Promise<boolean> {
        return token.type === 'app:serial-tab'
    }

    async recover (token: RecoveryToken): Promise<NewTabParameters<TauriSerialTabComponent>> {
        return {
            type: TauriSerialTabComponent,
            inputs: {
                profile: this.injector.get(ProfilesService).getConfigProxyForProfile(token.profile),
                savedState: token.savedState,
            },
        }
    }
}
