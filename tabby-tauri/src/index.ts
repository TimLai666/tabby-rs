import { CommonModule } from '@angular/common'
import { NgModule } from '@angular/core'
import {
    ConfigProvider,
    HostAppService,
    HostWindowService,
    LogService,
    PlatformService,
    UpdaterService,
    VaultService,
} from 'tabby-core'
import { PasswordStorageService } from 'tabby-ssh'
import { SettingsTabProvider } from 'tabby-settings'

import { HostBridge } from './api/hostBridge'
import './api/keychain'
import { SecretImporter } from './api/secretImporter'
import {
    IdentitySettingsTabComponent,
    IdentitySettingsTabProvider,
} from './components/identitySettingsTab.component'
import { TauriConfigProvider } from './config'
import { TauriHostAppService } from './services/hostApp.service'
import { TauriHostWindowService } from './services/hostWindow.service'
import { TauriLogService } from './services/log.service'
import { TauriPasswordStorageService } from './services/passwordStorage.service'
import { TauriPlatformService } from './services/platform.service'
import { TauriSecretImporter } from './services/secretImporter.service'
import { TauriHostBridge } from './services/tauriHostBridge.service'
import { TauriUpdaterService } from './services/updater.service'
import { TauriVaultService } from './services/vault.service'

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
        TauriVaultService,
        { provide: VaultService, useExisting: TauriVaultService },
        TauriSecretImporter,
        { provide: SecretImporter, useExisting: TauriSecretImporter },
        { provide: PasswordStorageService, useClass: TauriPasswordStorageService },
        { provide: ConfigProvider, useClass: TauriConfigProvider, multi: true },
        { provide: SettingsTabProvider, useClass: IdentitySettingsTabProvider, multi: true },
    ],
})
// Angular discovers providers and declarations through the NgModule metadata.
// eslint-disable-next-line @typescript-eslint/no-extraneous-class
export default class TauriModule { }

export * from './api/hostBridge'
export * from './api/keychain'
export * from './api/secretImporter'
export { TauriHostBridge }
export { TauriPasswordStorageService }
export { TauriSecretImporter }
export { TauriVaultService }
