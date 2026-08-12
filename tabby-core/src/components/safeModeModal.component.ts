import { Component, Input } from '@angular/core'
import { NgbActiveModal } from '@ng-bootstrap/ng-bootstrap'
import { ConfigService } from '../services/config.service'
import { PlatformService } from '../api/platform'

interface PluginLoadFailure {
    plugin?: { name?: string, packageName?: string }
    phase?: string
    code?: string
    message?: string
}

/** @hidden */
@Component({
    templateUrl: './safeModeModal.component.pug',
})
export class SafeModeModalComponent {
    @Input() error: Error

    constructor (
        public modalInstance: NgbActiveModal,
        private config: ConfigService,
        private platform: PlatformService,
    ) {
        this.error = window['safeModeReason']
    }

    get failures (): PluginLoadFailure[] {
        return Array.isArray(window['pluginLoadFailures']) ? window['pluginLoadFailures'] : []
    }

    get suspectedPlugins (): string[] {
        const persisted = Array.isArray(window['safeModeSuspectedPlugins'])
            ? window['safeModeSuspectedPlugins']
            : []
        const failed = this.failures
            .map(failure => failure.plugin?.name)
            .filter((name): name is string => !!name && name !== '<plugin discovery>')
        return [...new Set([...persisted, ...failed])]
    }

    disablePlugin (name: string): void {
        const blacklist = this.config.store.pluginBlacklist ?? []
        if (!blacklist.includes(name)) {
            this.config.store.pluginBlacklist = [...blacklist, name]
        }
        void this.config.save().then(() => this.retry()).catch(error => {
            console.error('Could not disable plugin from safe mode:', error)
        })
    }

    async removePlugin (name: string): Promise<void> {
        const packageName = this.packageNameFor(name)
        if (!packageName) {
            console.error('Could not remove plugin from safe mode: package name is unavailable')
            return
        }
        try {
            await this.platform.uninstallPlugin(packageName)
            const blacklist = this.config.store.pluginBlacklist ?? []
            this.config.store.pluginBlacklist = blacklist.filter(value => value !== name && value !== packageName)
            await this.config.save()
            this.retry()
        } catch (error) {
            console.error('Could not remove plugin from safe mode:', error)
        }
    }

    private packageNameFor (name: string): string|null {
        const failure = this.failures.find(candidate => candidate.plugin?.name === name || candidate.plugin?.packageName === name)
        if (failure?.plugin?.packageName) {
            return failure.plugin.packageName
        }
        if (name.startsWith('tabby-') || name.startsWith('terminus-')) {
            return name
        }
        return null
    }

    retry (): void {
        const retryPluginBootstrap = window['retryPluginBootstrap']
        if (typeof retryPluginBootstrap === 'function') {
            void retryPluginBootstrap()
            return
        }
        this.config.requestRestart()
        this.close()
    }

    close (): void {
        this.modalInstance.dismiss()
    }
}
