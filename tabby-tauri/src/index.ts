import { NgModule } from '@angular/core'
import {
    ConfigProvider,
    HostAppService,
    HostWindowService,
    LogService,
    PlatformService,
    UpdaterService,
} from 'tabby-core'

import { TauriConfigProvider } from './config'
import { HostBridge } from './api/hostBridge'
import { TauriHostBridge } from './services/tauriHostBridge.service'
import { TauriHostAppService } from './services/hostApp.service'
import { TauriHostWindowService } from './services/hostWindow.service'
import { TauriLogService } from './services/log.service'
import { TauriPlatformService } from './services/platform.service'
import { TauriUpdaterService } from './services/updater.service'

@NgModule({
    providers: [
        TauriHostBridge,
        { provide: HostBridge, useExisting: TauriHostBridge },
        { provide: PlatformService, useClass: TauriPlatformService },
        { provide: HostAppService, useClass: TauriHostAppService },
        { provide: HostWindowService, useClass: TauriHostWindowService },
        { provide: LogService, useClass: TauriLogService },
        { provide: UpdaterService, useClass: TauriUpdaterService },
        { provide: ConfigProvider, useClass: TauriConfigProvider, multi: true },
    ],
})
export default class TauriModule { }

export * from './api/hostBridge'
export { TauriHostBridge }
