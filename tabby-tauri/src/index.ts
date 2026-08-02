import { CommonModule } from '@angular/common'
import { APP_INITIALIZER, NgModule } from '@angular/core'
import {
    ConfigProvider,
    HostAppService,
    HostWindowService,
    LogService,
    PlatformService,
    UpdaterService,
    VaultService,
} from 'tabby-core'
import { SettingsTabProvider } from 'tabby-settings'

import {
    PTYInterface,
    ShellProvider,
    UACService,
} from '../../tabby-local/src/api'
import { PasswordStorageService } from '../../tabby-ssh/src/services/passwordStorage.service'
import { HostBridge } from './api/hostBridge'
import './api/keychain'
import './api/ptyBridge'
import { SecretImporter } from './api/secretImporter'
import './api/shell'
import './api/windowsIntegration'
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
import { TauriPTYInterface } from './services/ptyInterface.service'
import { TauriSecretImporter } from './services/secretImporter.service'
import {
    TauriDetectedShellProvider,
    TauriSpawnRequestService,
} from './services/shellProvider.service'
import { TauriHostBridge } from './services/tauriHostBridge.service'
import { TauriUACService } from './services/uac.service'
import { TauriUpdaterService } from './services/updater.service'
import { TauriVaultService } from './services/vault.service'

function initializeUac (service: TauriUACService): () => Promise<void> {
    return async () => {
        try {
            await service.initialize()
        } catch (error) {
            console.info('Windows administrator integration is unavailable:', error)
        }
    }
}

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
        { provide: ShellProvider, useClass: TauriDetectedShellProvider, multi: true },
        TauriSpawnRequestService,
        { provide: PTYInterface, useClass: TauriPTYInterface },
        TauriUACService,
        { provide: UACService, useExisting: TauriUACService },
        {
            provide: APP_INITIALIZER,
            useFactory: initializeUac,
            deps: [TauriUACService],
            multi: true,
        },
        { provide: ConfigProvider, useClass: TauriConfigProvider, multi: true },
        { provide: SettingsTabProvider, useClass: IdentitySettingsTabProvider, multi: true },
    ],
})
// Angular discovers providers and declarations through the NgModule metadata.
// eslint-disable-next-line @typescript-eslint/no-extraneous-class
export default class TauriModule { }

export * from './api/hostBridge'
export * from './api/keychain'
export * from './api/pty'
export * from './api/secretImporter'
export * from './api/shell'
export * from './api/windowsIntegration'
export { TauriDetectedShellProvider }
export { TauriHostBridge }
export { TauriPasswordStorageService }
export { TauriPTYInterface }
export { TauriSecretImporter }
export { TauriSpawnRequestService }
export { TauriUACService }
export { TauriVaultService }
