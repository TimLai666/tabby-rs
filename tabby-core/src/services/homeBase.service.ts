import { Injectable, Inject } from '@angular/core'
import { ConfigService } from './config.service'
import { PlatformService, BOOTSTRAP_DATA, BootstrapData, HostAppService } from '../api'

@Injectable({ providedIn: 'root' })
export class HomeBaseService {
    appVersion: string

    /** @hidden */
    private constructor (
        private config: ConfigService,
        private platform: PlatformService,
        private hostApp: HostAppService,
        @Inject(BOOTSTRAP_DATA) private bootstrapData: BootstrapData,
    ) {
        this.appVersion = platform.getAppVersion()
    }

    openGitHub (): void {
        this.platform.openExternal('https://github.com/Eugeny/tabby')
    }

    openDiscord (): void {
        this.platform.openExternal('https://discord.gg/Vn7BjmzhtF')
    }

    openTranslations (): void {
        this.platform.openExternal('https://translate.tabby.sh/project/tabby')
    }

    reportBug (): void {
        let body = `Version: ${this.appVersion}\n`
        body += `Platform: ${this.hostApp.platform} ${process.arch} ${this.platform.getOSRelease()}\n`
        const plugins = this.bootstrapData.installedPlugins.filter(x => !x.isBuiltin).map(x => x.name)
        body += `Plugins: ${plugins.join(', ') || 'none'}\n`
        body += `Frontend: ${this.config.store.terminal?.frontend}\n\n`
        this.platform.openExternal(`https://github.com/Eugeny/tabby/issues/new?body=${encodeURIComponent(body)}`)
    }

}
