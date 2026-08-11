import { CommonModule } from '@angular/common'
import { APP_INITIALIZER, NgModule } from '@angular/core'
import { FormsModule } from '@angular/forms'
import { NgbModule } from '@ng-bootstrap/ng-bootstrap'
import {
    ConfigProvider,
    DockingService,
    FileProvider,
    HostAppService,
    HostWindowService,
    HotkeyProvider,
    LogService,
    PlatformService,
    NotificationsService,
    ProfileProvider,
    TabRecoveryProvider,
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
import { TauriHotkeyProvider } from './hotkeys'
import { TauriDesktopIntegrationService } from './services/desktopIntegration.service'
import { TauriDockingService } from './services/docking.service'
import { TauriFileProvider } from './services/fileProvider.service'
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
import { TauriNotificationsService } from './services/notifications.service'
import { TauriVaultService } from './services/vault.service'
import { TauriPathDropDecorator } from './pathDrop'
import { TauriExportTerminalContextMenu } from './terminalContextMenu'
import { TerminalContextMenuItemProvider, TerminalDecorator } from 'tabby-terminal'
import { TauriSshAuthPromptModalComponent } from './ssh/authPromptModal.component'
import { TauriSshHostKeyPromptModalComponent } from './ssh/hostKeyPromptModal.component'
import { TauriSshImportModalComponent } from './ssh/importModal.component'
import { TauriSshProfileSettingsComponent } from './ssh/profileSettings.component'
import { TauriSshProfilesService } from './ssh/profiles'
import { TauriSshTabRecoveryProvider } from './ssh/recoveryProvider'
import { TauriSshTabComponent } from './ssh/tab.component'
import { TauriSftpPanelComponent } from './ssh/sftpPanel.component'
import { TauriSftpContextMenu } from './sftpContextMenu'

function initializeUac (service: TauriUACService): () => Promise<void> {
    return async () => {
        try {
            await service.initialize()
        } catch (error) {
            console.info('Windows administrator integration is unavailable:', error)
        }
    }
}

function initializeDesktop (service: TauriDesktopIntegrationService): () => Promise<void> {
    return () => service.initialize()
}

@NgModule({
    imports: [CommonModule, FormsModule, NgbModule],
    declarations: [
        IdentitySettingsTabComponent,
        TauriSshAuthPromptModalComponent,
        TauriSshHostKeyPromptModalComponent,
        TauriSshImportModalComponent,
        TauriSshProfileSettingsComponent,
        TauriSshTabComponent,
        TauriSftpPanelComponent,
    ],
    providers: [
        TauriHostBridge,
        { provide: HostBridge, useExisting: TauriHostBridge },
        TauriPlatformService,
        { provide: PlatformService, useExisting: TauriPlatformService },
        TauriNotificationsService,
        { provide: NotificationsService, useExisting: TauriNotificationsService },
        TauriHostAppService,
        { provide: HostAppService, useExisting: TauriHostAppService },
        TauriHostWindowService,
        { provide: HostWindowService, useExisting: TauriHostWindowService },
        TauriDockingService,
        { provide: DockingService, useExisting: TauriDockingService },
        { provide: HotkeyProvider, useClass: TauriHotkeyProvider, multi: true },
        { provide: TerminalDecorator, useClass: TauriPathDropDecorator, multi: true },
        { provide: TerminalContextMenuItemProvider, useClass: TauriExportTerminalContextMenu, multi: true },
        { provide: TerminalContextMenuItemProvider, useClass: TauriSftpContextMenu, multi: true },
        { provide: FileProvider, useClass: TauriFileProvider, multi: true },
        TauriDesktopIntegrationService,
        { provide: LogService, useClass: TauriLogService },
        { provide: UpdaterService, useClass: TauriUpdaterService },
        TauriVaultService,
        { provide: VaultService, useExisting: TauriVaultService },
        TauriSecretImporter,
        { provide: SecretImporter, useExisting: TauriSecretImporter },
        { provide: PasswordStorageService, useClass: TauriPasswordStorageService },
        TauriSshProfilesService,
        { provide: ProfileProvider, useExisting: TauriSshProfilesService, multi: true },
        TauriSshTabRecoveryProvider,
        { provide: TabRecoveryProvider, useExisting: TauriSshTabRecoveryProvider, multi: true },
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
        {
            provide: APP_INITIALIZER,
            useFactory: initializeDesktop,
            deps: [TauriDesktopIntegrationService],
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
export { TauriPathDropDecorator }
export { TauriSshProfilesService, TauriSshTabComponent }
