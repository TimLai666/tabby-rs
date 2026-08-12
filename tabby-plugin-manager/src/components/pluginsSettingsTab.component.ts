/* eslint-disable @typescript-eslint/explicit-module-boundary-types */
import { marker as _ } from '@biesbjerg/ngx-translate-extract-marker'
import { BehaviorSubject, Observable, debounceTime, distinctUntilChanged, first, tap, switchMap, map } from 'rxjs'
import semverGt from 'semver/functions/gt'

import { Component, HostBinding, Input } from '@angular/core'
import { ConfigService, NodeToolchainStatus, PlatformService, PluginInfo } from 'tabby-core'
import { PluginManagerService } from '../services/pluginManager.service'

enum BusyState { Installing = 'Installing', Uninstalling = 'Uninstalling' }

const FORCE_ENABLE = ['tabby-core', 'tabby-settings', 'tabby-electron', 'tabby-web', 'tabby-plugin-manager', 'tabby-tauri']

_('Search plugins')

/** @hidden */
@Component({
    templateUrl: './pluginsSettingsTab.component.pug',
    styleUrls: ['./pluginsSettingsTab.component.scss'],
})
export class PluginsSettingsTabComponent {
    BusyState = BusyState
    @Input() availablePlugins$: Observable<PluginInfo[]>
    @Input() availablePluginsQuery$ = new BehaviorSubject<string>('')
    @Input() availablePluginsReady = false
    @Input() installedPluginsQuery$ = new BehaviorSubject<string>('')
    @Input() knownUpgrades: Record<string, PluginInfo|null> = {}
    @Input() busy = new Map<string, BusyState>()
    @Input() erroredPlugin: string
    @Input() errorMessage: string

    @HostBinding('class.content-box') true

    installedPlugins$: PluginInfo[] = []
    installedFilter = ''
    availableFilter = ''
    nodeStatus: NodeToolchainStatus | null = null
    customNodePath = ''
    nodeStatusLoading = false
    cancellingPlugins = new Set<string>()

    constructor (
        private config: ConfigService,
        private platform: PlatformService,
        public pluginManager: PluginManagerService,
    ) {
    }

    ngOnInit () {
        if (!this.canManagePlugins()) {
            void this.refreshNodeStatus()
        }
        this.availablePlugins$ = this.availablePluginsQuery$
            .asObservable()
            .pipe(
                debounceTime(200),
                distinctUntilChanged(),
                switchMap(query => {
                    this.availablePluginsReady = false
                    return this.pluginManager.listAvailable(query).pipe(tap(() => {
                        this.availablePluginsReady = true
                    }))
                }),
            )
        this.availablePlugins$.pipe(first(), map((plugins: PluginInfo[]) => {
            plugins.sort((a, b) => a.name > b.name ? 1 : -1)
            return plugins
        })).subscribe(available => {
            for (const plugin of this.pluginManager.installedPlugins) {
                this.knownUpgrades[plugin.name] = available.find(x => x.name === plugin.name && semverGt(x.version, plugin.version)) ?? null
            }
        })

        this.installedPluginsQuery$
            .asObservable()
            .pipe(
                debounceTime(200),
                distinctUntilChanged(),
                switchMap(query => {
                    return this.pluginManager.listInstalled(query)
                }),
            ).subscribe(plugin => {
                this.installedPlugins$ = plugin
            })
    }

    async refreshNodeStatus (): Promise<void> {
        this.nodeStatusLoading = true
        try {
            this.nodeStatus = await this.platform.getNodeToolchainStatus(this.customNodePath.trim() || undefined)
        } catch (error) {
            this.nodeStatus = {
                nodePath: null,
                nodeVersion: null,
                npmPath: null,
                npmVersion: null,
                supported: false,
                reason: String(error),
            }
        } finally {
            this.nodeStatusLoading = false
        }
    }

    canManagePlugins (): boolean {
        return this.platform.supportsPluginManagement
    }

    openPluginsFolder (): void {
        this.platform.openPath(this.pluginManager.userPluginsPath)
    }

    searchAvailable (query: string) {
        this.availablePluginsQuery$.next(query)
    }

    searchInstalled (query: string) {
        this.installedPluginsQuery$.next(query)
    }

    isAlreadyInstalled (plugin: PluginInfo): boolean {
        return this.pluginManager.installedPlugins.some(x => x.name === plugin.name)
    }

    async installPlugin (plugin: PluginInfo): Promise<void> {
        if (!this.canManagePlugins()) {
            return
        }
        this.busy.set(plugin.name, BusyState.Installing)
        try {
            await this.pluginManager.installPlugin(plugin)
            this.busy.delete(plugin.name)
            this.config.requestRestart()
        } catch (err) {
            console.error('Error installing plugin', plugin.name, err)
            this.erroredPlugin = plugin.name
            this.errorMessage = err
            this.busy.delete(plugin.name)
            throw err
        }
    }

    async uninstallPlugin (plugin: PluginInfo): Promise<void> {
        if (!this.canManagePlugins()) {
            return
        }
        this.busy.set(plugin.name, BusyState.Uninstalling)
        try {
            await this.pluginManager.uninstallPlugin(plugin)
            this.busy.delete(plugin.name)
            this.config.requestRestart()
        } catch (err) {
            console.error('Error uninstalling plugin', plugin.name, err)
            this.erroredPlugin = plugin.name
            this.errorMessage = err
            this.busy.delete(plugin.name)
            throw err
        }
    }

    canCancelPlugin (plugin: PluginInfo): boolean {
        return this.pluginManager.getPluginOperationId(plugin) !== null
    }

    isCancellingPlugin (plugin: PluginInfo): boolean {
        return this.cancellingPlugins.has(plugin.name)
    }

    async cancelPlugin (plugin: PluginInfo): Promise<void> {
        const operationId = this.pluginManager.getPluginOperationId(plugin)
        if (!operationId || this.cancellingPlugins.has(plugin.name)) {
            return
        }
        this.cancellingPlugins.add(plugin.name)
        try {
            await this.platform.cancelPluginOperation(operationId)
        } catch (err) {
            console.error('Error cancelling plugin operation', plugin.name, err)
            this.erroredPlugin = plugin.name
            this.errorMessage = err
        } finally {
            this.cancellingPlugins.delete(plugin.name)
        }
    }

    async upgradePlugin (plugin: PluginInfo): Promise<void> {
        await this.installPlugin(this.knownUpgrades[plugin.name]!)
        this.knownUpgrades[plugin.name] = null
    }

    showPluginInfo (plugin: PluginInfo) {
        this.platform.openExternal('https://www.npmjs.com/package/' + plugin.packageName)
    }

    showPluginHomepage (plugin: PluginInfo) {
        this.platform.openExternal(plugin.homepage ?? '')
    }

    isPluginEnabled (plugin: PluginInfo) {
        return !this.config.store.pluginBlacklist.includes(plugin.name)
    }

    canDisablePlugin (plugin: PluginInfo) {
        return !FORCE_ENABLE.includes(plugin.packageName)
    }

    togglePlugin (plugin: PluginInfo) {
        if (this.isPluginEnabled(plugin)) {
            this.disablePlugin(plugin)
        } else {
            this.enablePlugin(plugin)
        }
    }

    enablePlugin (plugin: PluginInfo) {
        this.config.store.pluginBlacklist = this.config.store.pluginBlacklist.filter(x => x !== plugin.name)
        this.config.save()
        this.config.requestRestart()
    }

    disablePlugin (plugin: PluginInfo) {
        this.config.store.pluginBlacklist = [...this.config.store.pluginBlacklist, plugin.name]
        this.config.save()
        this.config.requestRestart()
    }
}
