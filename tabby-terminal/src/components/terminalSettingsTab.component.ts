import { Component, HostBinding } from '@angular/core'
import { ConfigService, getAltKeyName, getMetaKeyName, HostAppService, Platform, PlatformService } from 'tabby-core'

/** @hidden */
@Component({
    templateUrl: './terminalSettingsTab.component.pug',
})
export class TerminalSettingsTabComponent {
    Platform = Platform
    get altKeyName (): string { return getAltKeyName(this.hostApp.configPlatform) }
    get metaKeyName (): string { return getMetaKeyName(this.hostApp.configPlatform) }

    @HostBinding('class.content-box') true

    constructor (
        public config: ConfigService,
        public hostApp: HostAppService,
        private platform: PlatformService,
    ) { }

    openWSLVolumeMixer (): void {
        this.platform.openPath('sndvol.exe')
        this.platform.exec('wsl.exe', ['tput', 'bel'])
    }
}
