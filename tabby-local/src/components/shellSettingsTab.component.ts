import { Component, HostBinding } from '@angular/core'
import { WIN_BUILD_CONPTY_SUPPORTED, WIN_BUILD_CONPTY_STABLE, isWindowsBuild, ConfigService, HostAppService } from 'tabby-core'

/** @hidden */
@Component({
    templateUrl: './shellSettingsTab.component.pug',
})
export class ShellSettingsTabComponent {
    isConPTYAvailable: boolean
    isConPTYStable: boolean

    @HostBinding('class.content-box') true

    constructor (
        public config: ConfigService,
        public hostApp: HostAppService,
    ) {
        this.isConPTYAvailable = isWindowsBuild(WIN_BUILD_CONPTY_SUPPORTED, hostApp.platform, hostApp.windowsBuild)
        this.isConPTYStable = isWindowsBuild(WIN_BUILD_CONPTY_STABLE, hostApp.platform, hostApp.windowsBuild)
    }
}
