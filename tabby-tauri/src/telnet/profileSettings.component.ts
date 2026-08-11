import { Component } from '@angular/core'
import { FullyDefined, ProfileSettingsComponent } from 'tabby-core'

import { TauriTelnetProfile } from './profile'
import { TauriTelnetProfilesService } from './profiles'

@Component({
    templateUrl: './profileSettings.component.pug',
})
export class TauriTelnetProfileSettingsComponent implements ProfileSettingsComponent<TauriTelnetProfile, TauriTelnetProfilesService> {
    profile: FullyDefined<TauriTelnetProfile>
}
