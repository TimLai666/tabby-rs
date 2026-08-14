import assert from 'node:assert/strict'
import fs from 'node:fs'
import Module from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'tabby-plugin-manager/src/services/pluginManager.service.ts')
const source = fs.readFileSync(sourcePath, 'utf8')
const componentSource = fs.readFileSync(
    path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.ts'),
    'utf8',
)
const templateSource = fs.readFileSync(
    path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.pug'),
    'utf8',
)
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
        experimentalDecorators: true,
    },
    fileName: sourcePath,
}).outputText

const originalLoad = Module._load
const decorator = () => target => target
Module._load = function (request, parent, isMain) {
    if (request === '@angular/core') {
        return { Injectable: decorator, Inject: () => () => undefined }
    }
    if (request === 'tabby-core') {
        return { BOOTSTRAP_DATA: Symbol('bootstrap-data') }
    }
    if (request === 'rxjs') {
        return { Observable: class {}, from: () => undefined, forkJoin: () => undefined, map: () => undefined, of: () => undefined }
    }
    if (request.includes('app/src/pluginBlacklist')) {
        return { PLUGIN_BLACKLIST: [] }
    }
    return originalLoad.call(this, request, parent, isMain)
}

const pluginManagerModule = new Module(sourcePath)
pluginManagerModule.filename = sourcePath
pluginManagerModule.paths = Module._nodeModulePaths(root)
try {
    pluginManagerModule._compile(compiled, sourcePath)
} finally {
    Module._load = originalLoad
}

const service = Object.create(pluginManagerModule.exports.PluginManagerService.prototype)
const registryObject = (version, name = 'tabby-fixture') => ({
    package: {
        name,
        version,
        description: 'fixture',
        keywords: ['tabby-plugin'],
        maintainers: [],
    },
})
const registryResponse = (value, headers = {}) => ({
    ok: true,
    headers: { get: name => headers[name] ?? null },
    body: null,
    text: async () => JSON.stringify(value),
})
const originalFetch = globalThis.fetch
const originalWindow = globalThis.window
const requests = []
globalThis.window = { setTimeout, clearTimeout }
globalThis.fetch = async url => {
    requests.push(String(url))
    const parsed = new URL(url)
    const from = Number(parsed.searchParams.get('from'))
    return registryResponse({
        total: 101,
        objects: from === 0 ? Array.from({ length: 100 }, (_, index) => registryObject('1.0.0', `tabby-fixture-${index}`)) : [registryObject('1.0.1')],
    })
}
try {
    const objects = await service.searchRegistry('tabby-plugin', ' terminal ')
    assert.equal(objects.length, 101)
    assert.equal(requests.length, 2)
    assert.equal(new URL(requests[0]).searchParams.get('from'), '0')
    assert.equal(new URL(requests[1]).searchParams.get('from'), '100')
    assert.equal(new URL(requests[0]).searchParams.get('text'), 'keywords:tabby-plugin terminal')

    globalThis.fetch = async () => ({
        ok: false,
        status: 503,
        headers: { get: () => null },
        body: null,
        text: async () => '',
    })
    await assert.rejects(() => service.searchRegistry('tabby-plugin'), /HTTP 503/)

    globalThis.fetch = async () => registryResponse({}, { 'content-length': String(2 * 1024 * 1024 + 1) })
    await assert.rejects(() => service.searchRegistry('tabby-plugin'), /response is too large/)

    globalThis.fetch = async () => registryResponse({
        total: 1,
        objects: [{ package: { name: 'tabby-invalid', version: 'not-semver' } }],
    })
    assert.deepEqual(await service.searchRegistry('tabby-plugin'), [])

    globalThis.window = { setTimeout: callback => setTimeout(callback, 0), clearTimeout }
    globalThis.fetch = async (_url, { signal }) => new Promise((resolve, reject) => {
        signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true })
    })
    await assert.rejects(() => service.searchRegistry('tabby-plugin'), /request timed out/)
} finally {
    globalThis.fetch = originalFetch
    if (originalWindow === undefined) {
        delete globalThis.window
    } else {
        globalThis.window = originalWindow
    }
}

const basePackage = {
    name: 'tabby-example',
    version: '1.2.3',
    description: 'fixture',
    keywords: ['tabby-plugin'],
    maintainers: [],
}

assert.ok(service.toPluginInfo({ package: basePackage }, 'tabby-', 'tabby-plugin'))
assert.equal(
    service.toPluginInfo({ package: { ...basePackage, name: '@scope/plugin' } }, 'tabby-', 'tabby-plugin')?.name,
    '@scope/plugin',
)
assert.equal(
    service.toPluginInfo({ package: { ...basePackage, keywords: ['other-keyword'] } }, 'tabby-', 'tabby-plugin'),
    null,
)
assert.ok(service.toPluginInfo({ package: { ...basePackage, keywords: ['TABBY-PLUGIN'] } }, 'tabby-', 'tabby-plugin'))
assert.doesNotMatch(source, /packageName\.startsWith\(namePrefix\)/)

assert.match(componentSource, /catchError\(error => \{[\s\S]*availablePluginsReady = true[\s\S]*availablePluginsError/)
assert.match(componentSource, /blacklist\.includes\(plugin\.name\) \|\| blacklist\.includes\(plugin\.packageName\)/)
assert.match(componentSource, /filter\(x => x !== plugin\.name && x !== plugin\.packageName\)/)
assert.match(componentSource, /tap\(plugins => this\.updateKnownUpgrades\(plugins\)\)/)
assert.match(componentSource, /shareReplay\(\{ bufferSize: 1, refCount: true \}\)/)
assert.doesNotMatch(componentSource, /availablePlugins\$\.pipe\(first\(\)/)
assert.match(templateSource, /availablePluginsError/)
assert.match(source, /await this\.platform\.updatePlugin\(plugin\.packageName\)/)
assert.match(componentSource, /await this\.updatePlugin\(this\.knownUpgrades\[plugin\.name\]!\)/)

const componentPath = path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.ts')
const componentCompiled = ts.transpileModule(componentSource, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
        experimentalDecorators: true,
        esModuleInterop: true,
    },
    fileName: componentPath,
}).outputText

const originalComponentLoad = Module._load
const propertyDecorator = () => () => undefined
Module._load = function (request, parent, isMain) {
    if (request === '@angular/core') {
        return { Component: decorator, HostBinding: propertyDecorator, Input: propertyDecorator }
    }
    if (request === 'tabby-core') {
        return {}
    }
    if (request.includes('../services/pluginManager.service')) {
        return { PluginManagerService: class {} }
    }
    return originalComponentLoad.call(this, request, parent, isMain)
}

const componentModule = new Module(componentPath)
componentModule.filename = componentPath
componentModule.paths = Module._nodeModulePaths(root)
try {
    componentModule._compile(componentCompiled, componentPath)
} finally {
    Module._load = originalComponentLoad
}

const { PluginsSettingsTabComponent } = componentModule.exports
const { of } = await import('rxjs')
const pluginManager = {
    installedPlugins: [{ name: 'demo', packageName: 'tabby-demo', version: '1.0.0' }],
    listAvailable: query => of([{
        name: 'demo',
        packageName: 'tabby-demo',
        version: query ? '1.1.0' : '1.0.0',
    }]),
    listInstalled: () => of([]),
}
const platform = { supportsPluginManagement: true }
const config = { store: {}, requestRestart: () => undefined, save: () => undefined }
const component = new PluginsSettingsTabComponent(config, platform, pluginManager)
component.ngOnInit()
const availableSubscription = component.availablePlugins$.subscribe()
await new Promise(resolve => setTimeout(resolve, 260))
assert.equal(component.knownUpgrades.demo, null)
component.searchAvailable('upgrade')
await new Promise(resolve => setTimeout(resolve, 260))
assert.equal(component.knownUpgrades.demo.version, '1.1.0')
availableSubscription.unsubscribe()

console.log('Plugin search keyword contract fixtures passed')
