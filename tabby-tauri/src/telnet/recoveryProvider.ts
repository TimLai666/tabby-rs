import { Injectable, Injector } from '@angular/core'
import { NewTabParameters, ProfilesService, RecoveryToken, TabRecoveryProvider } from 'tabby-core'

import { TauriTelnetTabComponent } from './tab.component'

@Injectable()
export class TauriTelnetTabRecoveryProvider extends TabRecoveryProvider<TauriTelnetTabComponent> {
    constructor (private injector: Injector) { super() }

    async applicableTo (token: RecoveryToken): Promise<boolean> {
        return token.type === 'app:telnet-tab'
    }

    async recover (token: RecoveryToken): Promise<NewTabParameters<TauriTelnetTabComponent>> {
        return {
            type: TauriTelnetTabComponent,
            inputs: {
                profile: this.injector.get(ProfilesService).getConfigProxyForProfile(token.profile),
                savedState: token.savedState,
            },
        }
    }
}
