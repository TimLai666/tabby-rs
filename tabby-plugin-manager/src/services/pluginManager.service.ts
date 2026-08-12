import { compare as semverCompare, valid as semverValid } from 'semver'
import { Observable, from, forkJoin, map, of } from 'rxjs'
import { Injectable, Inject } from '@angular/core'
import { Logger, LogService, PlatformService, BOOTSTRAP_DATA, BootstrapData, PluginInfo } from 'tabby-core'
import { PLUGIN_BLACKLIST } from '../../../app/src/pluginBlacklist'

const OFFICIAL_NPM_ACCOUNT = 'eugenepankov'
const REGISTRY_SEARCH_URL = 'https://registry.npmjs.com/-/v1/search'
const REGISTRY_PAGE_SIZE = 100
const REGISTRY_MAX_PAGES = 3
const REGISTRY_TIMEOUT_MS = 10_000
const REGISTRY_MAX_RESPONSE_BYTES = 2 * 1024 * 1024

interface RegistryPackage {
    name: string
    version: string
    description: string
    homepage?: string
    keywords: string[]
    maintainers: { username: string }[]
    publisher?: { username: string }
}

interface RegistrySearchObject {
    package: RegistryPackage
    searchScore?: number
}

interface RegistrySearchResponse {
    objects: RegistrySearchObject[]
    total: number
}

/* eslint-disable @typescript-eslint/no-use-before-define */
function parseRegistrySearchResponse (value: unknown): RegistrySearchResponse {
    const record = asRecord(value)
    if (!record || !Array.isArray(record.objects) || typeof record.total !== 'number' || !Number.isFinite(record.total)) {
        throw new Error('NPM registry response has an invalid shape')
    }
    return {
        total: Math.max(0, record.total),
        objects: record.objects
            .map(parseRegistrySearchObject)
            .filter((item): item is RegistrySearchObject => item !== null),
    }
}

function parseRegistrySearchObject (value: unknown): RegistrySearchObject|null {
    const record = asRecord(value)
    const packageRecord = record ? asRecord(record.package) : null
    if (
        !packageRecord
        || typeof packageRecord.name !== 'string'
        || typeof packageRecord.version !== 'string'
        || !semverValid(packageRecord.version)
    ) {
        return null
    }
    const links = asRecord(packageRecord.links)
    const publisher = asRecord(packageRecord.publisher)
    const maintainers = Array.isArray(packageRecord.maintainers)
        ? packageRecord.maintainers
            .map(item => {
                const maintainer = asRecord(item)
                return typeof maintainer?.username === 'string' ? { username: maintainer.username } : null
            })
            .filter((item): item is { username: string } => item !== null)
        : []
    const keywords = Array.isArray(packageRecord.keywords)
        ? packageRecord.keywords.filter((item): item is string => typeof item === 'string')
        : []
    return {
        'package': {
            name: packageRecord.name,
            version: packageRecord.version,
            description: typeof packageRecord.description === 'string' ? packageRecord.description : '',
            homepage: typeof links?.homepage === 'string' ? links.homepage : undefined,
            keywords,
            maintainers,
            publisher: typeof publisher?.username === 'string' ? { username: publisher.username } : undefined,
        },
        searchScore: typeof record?.searchScore === 'number' ? record.searchScore : undefined,
    }
}

function asRecord (value: unknown): Record<string, any>|null {
    return typeof value === 'object' && value !== null ? value as Record<string, any> : null
}
/* eslint-enable @typescript-eslint/no-use-before-define */

@Injectable({ providedIn: 'root' })
export class PluginManagerService {
    logger: Logger
    userPluginsPath: string
    installedPlugins: PluginInfo[]

    private constructor (
        log: LogService,
        private platform: PlatformService,
        @Inject(BOOTSTRAP_DATA) bootstrapData: BootstrapData,
    ) {
        this.logger = log.create('pluginManager')
        this.installedPlugins = [...bootstrapData.installedPlugins]
        this.installedPlugins.sort((a, b) => a.name.localeCompare(b.name))
        this.userPluginsPath = bootstrapData.userPluginsPath
    }

    listAvailable (query?: string): Observable<PluginInfo[]> {
        return forkJoin(
            this._listAvailableInternal('tabby-', 'tabby-plugin', query),
            this._listAvailableInternal('terminus-', 'terminus-plugin', query),
        ).pipe(
            map(x => x.reduce((a, b) => a.concat(b), [])),
            map(x => {
                const names = new Set<string>()
                return x.filter(item => {
                    if (names.has(item.name)) {
                        return false
                    }
                    names.add(item.name)
                    return true
                })
            }),
            map(x => x.sort((a, b) => b.searchScore! - a.searchScore!)),
        )
    }

    listInstalled (query: string): Observable<PluginInfo[]> {
        return of(this.installedPlugins.filter(x=>x.name.includes(query)))
    }

    private async refreshInstalledPlugins (): Promise<boolean> {
        const installed = await this.platform.listInstalledPlugins()
        if (!installed) {
            return false
        }
        const builtins = this.installedPlugins.filter(x => x.isBuiltin)
        this.installedPlugins = [...builtins, ...installed]
        this.installedPlugins.sort((a, b) => a.name.localeCompare(b.name))
        return true
    }

    private updateInstalledPlugin (plugin: PluginInfo): void {
        this.installedPlugins = this.installedPlugins.filter(x => x.packageName !== plugin.packageName)
        this.installedPlugins.push(plugin)
    }

    _listAvailableInternal (namePrefix: string, keyword: string, query?: string): Observable<PluginInfo[]> {
        return from(this.searchRegistry(keyword, query)).pipe(
            map(objects => objects
                .map(item => this.toPluginInfo(item, namePrefix, keyword))
                .filter((item): item is PluginInfo => item !== null),
            ),
            map(plugins => plugins.filter(x => x.packageName.startsWith(namePrefix))),
            map(plugins => plugins.filter(x => !PLUGIN_BLACKLIST.includes(x.packageName))),
            map(plugins => {
                const mapping: Record<string, PluginInfo[]> = {}
                for (const p of plugins) {
                    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
                    mapping[p.name] ??= []
                    mapping[p.name].push(p)
                }
                return Object.values(mapping).map(list => {
                    list.sort((a, b) => -semverCompare(a.version, b.version))
                    return list[0]
                })
            }),
            map(plugins => plugins.sort((a, b) => a.name.localeCompare(b.name))),
        )
    }

    private async searchRegistry (keyword: string, query?: string): Promise<RegistrySearchObject[]> {
        const objects: RegistrySearchObject[] = []
        const text = `keywords:${keyword}${query?.trim() ? ` ${query.trim()}` : ''}`
        for (let page = 0; page < REGISTRY_MAX_PAGES; page++) {
            const url = new URL(REGISTRY_SEARCH_URL)
            url.searchParams.set('text', text)
            url.searchParams.set('size', String(REGISTRY_PAGE_SIZE))
            url.searchParams.set('from', String(page * REGISTRY_PAGE_SIZE))
            const response = await this.fetchRegistryPage(url)
            objects.push(...response.objects)
            const received = (page + 1) * REGISTRY_PAGE_SIZE
            if (response.objects.length < REGISTRY_PAGE_SIZE || received >= response.total) {
                break
            }
        }
        return objects
    }

    private async fetchRegistryPage (url: URL): Promise<RegistrySearchResponse> {
        const controller = new AbortController()
        const timeout = window.setTimeout(() => controller.abort(), REGISTRY_TIMEOUT_MS)
        try {
            const response = await fetch(url, { signal: controller.signal })
            if (!response.ok) {
                throw new Error(`NPM registry returned HTTP ${response.status}`)
            }
            const contentLength = Number(response.headers.get('content-length'))
            if (Number.isFinite(contentLength) && contentLength > REGISTRY_MAX_RESPONSE_BYTES) {
                throw new Error('NPM registry response is too large')
            }
            const body = await this.readResponseBody(response)
            return parseRegistrySearchResponse(JSON.parse(body))
        } catch (error) {
            if (controller.signal.aborted) {
                throw new Error('NPM registry request timed out')
            }
            throw error
        } finally {
            window.clearTimeout(timeout)
        }
    }

    private async readResponseBody (response: Response): Promise<string> {
        if (!response.body) {
            const body = await response.text()
            if (new TextEncoder().encode(body).byteLength > REGISTRY_MAX_RESPONSE_BYTES) {
                throw new Error('NPM registry response is too large')
            }
            return body
        }
        const reader = response.body.getReader()
        const decoder = new TextDecoder()
        let bytes = 0
        let body = ''
        while (true) {
            const chunk = await reader.read()
            if (chunk.done) {
                break
            }
            bytes += chunk.value.byteLength
            if (bytes > REGISTRY_MAX_RESPONSE_BYTES) {
                await reader.cancel()
                throw new Error('NPM registry response is too large')
            }
            body += decoder.decode(chunk.value, { stream: true })
        }
        return body + decoder.decode()
    }

    private toPluginInfo (item: RegistrySearchObject, namePrefix: string, keyword: string): PluginInfo|null {
        if (
            !item.package.name.startsWith(namePrefix)
            || !item.package.keywords.some(value => value.toLowerCase() === keyword.toLowerCase())
            || item.package.keywords.includes('tabby-dummy-transition-plugin')
            || PLUGIN_BLACKLIST.includes(item.package.name)
        ) {
            return null
        }
        return {
            name: item.package.name.substring(namePrefix.length),
            packageName: item.package.name,
            isBuiltin: false,
            isLegacy: false,
            description: item.package.description,
            version: item.package.version,
            homepage: item.package.homepage,
            author: item.package.maintainers[0]?.username ?? '',
            isOfficial: item.package.publisher?.username === OFFICIAL_NPM_ACCOUNT,
            searchScore: item.searchScore,
        }
    }

    async installPlugin (plugin: PluginInfo): Promise<void> {
        try {
            await this.platform.installPlugin(plugin.packageName, plugin.version)
            if (!await this.refreshInstalledPlugins()) {
                this.updateInstalledPlugin(plugin)
            }
        } catch (err) {
            await this.refreshInstalledPlugins().catch(refreshError => this.logger.error(refreshError))
            this.logger.error(err)
            throw err
        }
    }

    async uninstallPlugin (plugin: PluginInfo): Promise<void> {
        try {
            await this.platform.uninstallPlugin(plugin.packageName)
            if (!await this.refreshInstalledPlugins()) {
                this.installedPlugins = this.installedPlugins.filter(x => x.packageName !== plugin.packageName)
            }
        } catch (err) {
            await this.refreshInstalledPlugins().catch(refreshError => this.logger.error(refreshError))
            this.logger.error(err)
            throw err
        }
    }

    getPluginOperationId (plugin: PluginInfo): string|null {
        return this.platform.getPluginOperationId(plugin.packageName)
    }
}
