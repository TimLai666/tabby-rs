export const MAX_PLUGIN_SOURCE_BYTES = 8 * 1024 * 1024

const BLOCKED_NODE_MODULES = new Set([
    'assert',
    'child_process',
    'cluster',
    'crypto',
    'dgram',
    'dns',
    'fs',
    'http',
    'https',
    'module',
    'net',
    'os',
    'path',
    'perf_hooks',
    'process',
    'readline',
    'stream',
    'tls',
    'tty',
    'url',
    'util',
    'vm',
    'worker_threads',
    'zlib',
])

export class NodeRuntimeRequiredError extends Error {
    readonly code = 'node-runtime-required'

    constructor (public readonly missingModule: string) {
        super(`Plugin requires unsupported Node.js runtime module: ${missingModule}`)
        this.name = 'NodeRuntimeRequiredError'
    }
}

export class MissingPluginModuleError extends Error {
    readonly code = 'missing-module'

    constructor (public readonly missingModule: string) {
        super(`Plugin module is not registered: ${missingModule}`)
        this.name = 'MissingPluginModuleError'
    }
}

export class DuplicatePluginModuleError extends Error {
    readonly code = 'duplicate-module'

    constructor (public readonly moduleName: string) {
        super(`Plugin module is already registered: ${moduleName}`)
        this.name = 'DuplicatePluginModuleError'
    }
}

function isObject (value: unknown): value is Record<string, unknown> {
    return value !== null && typeof value === 'object'
}

export class PluginModuleRegistry {
    private readonly modules = new Map<string, unknown>()

    register (name: string, value: unknown): void {
        if (this.modules.has(name)) {
            throw new DuplicatePluginModuleError(name)
        }
        this.modules.set(name, value)
    }

    require (name: string): unknown {
        const normalized = name.startsWith('node:') ? name.substring(5) : name
        const root = normalized.split('/')[0]
        if (
            normalized === 'electron'
            || normalized.startsWith('electron/')
            || normalized === '@electron/remote'
            || normalized.startsWith('@electron/remote/')
            || normalized.endsWith('.node')
            || BLOCKED_NODE_MODULES.has(root)
        ) {
            throw new NodeRuntimeRequiredError(name)
        }
        const value = this.modules.get(name)
        if (value === undefined) {
            throw new MissingPluginModuleError(name)
        }
        return value
    }
}

function sourceUrl (filename: string): string {
    return encodeURIComponent(filename)
}

export function evaluateCommonJs (
    code: string,
    filename: string,
    registry: PluginModuleRegistry,
): unknown {
    if (new TextEncoder().encode(code).byteLength > MAX_PLUGIN_SOURCE_BYTES) {
        throw new Error('Plugin entry exceeds the maximum supported size')
    }
    const module = { exports: {} as Record<string, unknown> }
    const require = (name: string): unknown => registry.require(name)
    // This is the deliberately high-trust, registry-only CommonJS evaluator for #22.
    // eslint-disable-next-line @typescript-eslint/no-implied-eval
    const factory = new Function(
        'module',
        'exports',
        'require',
        '__filename',
        '__dirname',
        `'use strict';\n//# sourceURL=${sourceUrl(filename)}\n${code}`,
    ) as (
        module: { exports: Record<string, unknown> },
        exports: Record<string, unknown>,
        require: (name: string) => unknown,
        filename: string,
        dirname: string,
    ) => void
    const separator = Math.max(filename.lastIndexOf('/'), filename.lastIndexOf('\\'))
    const dirname = separator >= 0 ? filename.substring(0, separator) : '.'
    factory(module, module.exports, require, filename, dirname)
    return module.exports
}

export interface RuntimePluginDescriptor {
    name: string
    packageName: string
    version: string
    path: string
    entry: string
    isBuiltin: boolean
    isLegacy: boolean
    manifest: Record<string, unknown>
}

export interface RuntimePluginSource {
    packageName: string
    entry: string
    code: string
}

export interface PluginRuntimeHost {
    discover: () => Promise<RuntimePluginDescriptor[]>
    readEntry: (packageName: string) => Promise<RuntimePluginSource>
}

export interface PluginRuntimeLifecycle {
    pluginStarted?: (plugin: RuntimePluginDescriptor) => Promise<void> | void
    pluginCompleted?: (plugin: RuntimePluginDescriptor) => Promise<void> | void
}

export interface PluginLoadFailure {
    plugin: RuntimePluginDescriptor
    phase: 'discover' | 'read' | 'evaluate' | 'angular-bootstrap'
    code: 'missing-module' | 'node-runtime-required' | 'invalid-export' | 'exception'
    message: string
    missingModule?: string
}

export interface PluginLoadResult {
    modules: LoadedPluginModule[]
    failures: PluginLoadFailure[]
}

export interface LoadedPluginModule {
    pluginName?: string
    bootstrap?: unknown
    [key: string]: unknown
}

function failureFor (plugin: RuntimePluginDescriptor, phase: PluginLoadFailure['phase'], error: unknown): PluginLoadFailure {
    const code = error instanceof NodeRuntimeRequiredError
        ? error.code
        : error instanceof MissingPluginModuleError
            ? error.code
            : phase === 'evaluate' && error instanceof Error && error.message.includes('default export')
                ? 'invalid-export'
                : 'exception'
    const failure: PluginLoadFailure = {
        plugin,
        phase,
        code,
        message: error instanceof Error ? error.message : String(error),
    }
    if (error instanceof NodeRuntimeRequiredError || error instanceof MissingPluginModuleError) {
        failure.missingModule = error.missingModule
    }
    return failure
}

export async function loadPluginModules (
    host: PluginRuntimeHost,
    registry: PluginModuleRegistry,
    blacklist: string[] = [],
    lifecycle: PluginRuntimeLifecycle = {},
): Promise<PluginLoadResult> {
    const modules: LoadedPluginModule[] = []
    const failures: PluginLoadFailure[] = []
    let discoveredPlugins: RuntimePluginDescriptor[] = []
    try {
        discoveredPlugins = await host.discover()
    } catch (error) {
        failures.push(failureFor({
            name: '<plugin discovery>',
            packageName: '<plugin-discovery>',
            version: '',
            path: '',
            entry: '',
            isBuiltin: false,
            isLegacy: false,
            manifest: {},
        }, 'discover', error))
        return { modules, failures }
    }
    for (const plugin of discoveredPlugins) {
        if (blacklist.includes(plugin.name) || blacklist.includes(plugin.packageName)) {
            continue
        }
        try {
            await lifecycle.pluginStarted?.(plugin)
        } catch (error) {
            console.warn(`Could not journal plugin start for ${plugin.name}:`, error)
        }
        // eslint-disable-next-line @typescript-eslint/init-declarations
        let source: RuntimePluginSource
        try {
            source = await host.readEntry(plugin.packageName)
        } catch (error) {
            failures.push(failureFor(plugin, 'read', error))
            continue
        }
        try {
            const evaluatedModule = evaluateCommonJs(source.code, source.entry, registry)
            if (!isObject(evaluatedModule) && typeof evaluatedModule !== 'function') {
                throw new Error('Plugin has no valid default export')
            }
            const packageModule = (typeof evaluatedModule === 'function'
                ? { default: evaluatedModule }
                : evaluatedModule) as {
                default?: unknown
                bootstrap?: unknown
            }
            const exported = packageModule.default ?? packageModule
            if (!isObject(exported) && typeof exported !== 'function') {
                throw new Error('Plugin has no valid default export')
            }
            const exportedModule = exported as { forRoot?: () => unknown }
            const pluginModule = typeof exportedModule.forRoot === 'function' ? exportedModule.forRoot() : exported
            if (!isObject(pluginModule) && typeof pluginModule !== 'function') {
                throw new Error('Plugin default export did not produce a module')
            }
            const loadedModule = pluginModule as LoadedPluginModule
            loadedModule.pluginName = plugin.name
            loadedModule.bootstrap = packageModule.bootstrap
            modules.push(loadedModule)
            try {
                await lifecycle.pluginCompleted?.(plugin)
            } catch (error) {
                console.warn(`Could not journal plugin completion for ${plugin.name}:`, error)
            }
        } catch (error) {
            failures.push(failureFor(plugin, 'evaluate', error))
        }
    }
    return { modules, failures }
}
