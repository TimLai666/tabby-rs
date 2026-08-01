import { CommonModule } from '@angular/common'
import { NgModule } from '@angular/core'
import {
    ConfigProvider,
    HostAppService,
    HostWindowService,
    LogService,
    PlatformService,
    UpdaterService,
} from 'tabby-core'
import { SettingsTabProvider } from 'tabby-settings'

import { HostBridge } from './api/hostBridge'
import {
    IdentitySettingsTabComponent,
    IdentitySettingsTabProvider,
} from './components/identitySettingsTab.component'
import { TauriConfigProvider } from './config'
import { TauriHostAppService } from './services/hostApp.service'
import { TauriHostWindowService } from './services/hostWindow.service'
import { TauriLogService } from './services/log.service'
import { TauriPlatformService } from './services/platform.service'
import { TauriHostBridge } from './services/tauriHostBridge.service'
import { TauriUpdaterService } from './services/updater.service'

@NgModule({
    imports: [CommonModule],
    declarations: [IdentitySettingsTabComponent],
    providers: [
        TauriHostBridge,
        { provide: HostBridge, useExisting: TauriHostBridge },
        { provide: PlatformService, useClass: TauriPlatformService },
        { provide: HostAppService, useClass: TauriHostAppService },
        { provide: HostWindowService, useClass: TauriHostWindowService },
        { provide: LogService, useClass: TauriLogService },
        { provide: UpdaterService, useClass: TauriUpdaterService },
        { provide: ConfigProvider, useClass: TauriConfigProvider, multi: true },
        { provide: SettingsTabProvider, useClass: IdentitySettingsTabProvider, multi: true },
    ],
})
export default class TauriModule { }

export * from './api/hostBridge'
export { TauriHostBridge }
