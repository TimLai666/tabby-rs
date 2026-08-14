import assert from 'node:assert/strict'
import fs from 'node:fs'
import Module from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const componentPath = path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.ts')
const componentSource = fs.readFileSync(componentPath, 'utf8')
const componentCompiled = ts.transpileModule(componentSource, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
        experimentalDecorators: true,
        esModuleInterop: true,
    },
    fileName: componentPath,
}).outputText

const decorator = () => target => target
const propertyDecorator = () => () => undefined
const originalLoad = Module._load
Module._load = function (request, parent, isMain) {
    if (request === '@angular/core') {
        return { Component: decorator, HostBinding: propertyDecorator, Input: propertyDecorator }
    }
    if (request === '@biesbjerg/ngx-translate-extract-marker') {
        return { marker: () => undefined }
    }
    if (request === 'tabby-core') {
        return {}
    }
    if (request.includes('../services/pluginManager.service')) {
        return { PluginManagerService: class {} }
    }
    return originalLoad.call(this, request, parent, isMain)
}

const componentModule = new Module(componentPath)
componentModule.filename = componentPath
componentModule.paths = Module._nodeModulePaths(root)
try {
    componentModule._compile(componentCompiled, componentPath)
} finally {
    Module._load = originalLoad
}

const { PluginsSettingsTabComponent } = componentModule.exports
const { of } = await import('rxjs')
const statusRequests = []
const pluginCalls = []
const restartRequests = []
const plugin = {
    name: 'fixture',
    packageName: 'tabby-fixture',
    version: '1.0.0',
    description: 'fixture',
    isBuiltin: false,
}
const platform = {
    supportsPluginManagement: false,
    getNodeToolchainStatus: async customNodePath => {
        statusRequests.push(customNodePath)
        if (!customNodePath) {
            return {
                nodePath: null,
                nodeVersion: null,
                npmPath: null,
                npmVersion: null,
                supported: false,
                reason: 'Node.js was not found on PATH',
            }
        }
        platform.supportsPluginManagement = true
        return {
            nodePath: customNodePath,
            nodeVersion: 'v22.0.0',
            npmPath: `${customNodePath}/npm`,
            npmVersion: '10.0.0',
            supported: true,
            reason: null,
        }
    },
    installPlugin: async value => pluginCalls.push(['install', value.name]),
    updatePlugin: async value => pluginCalls.push(['update', value.name]),
    uninstallPlugin: async value => pluginCalls.push(['uninstall', value.name]),
}
const pluginManager = {
    installedPlugins: [],
    listAvailable: () => of([]),
    listInstalled: () => of([]),
    installPlugin: async value => pluginCalls.push(['manager-install', value.name]),
    updatePlugin: async value => pluginCalls.push(['manager-update', value.name]),
    uninstallPlugin: async value => pluginCalls.push(['manager-uninstall', value.name]),
    getPluginOperationId: () => null,
}
const config = {
    store: {},
    requestRestart: () => restartRequests.push(true),
    save: () => undefined,
}
const component = new PluginsSettingsTabComponent(config, platform, pluginManager)
component.ngOnInit()
await new Promise(resolve => setImmediate(resolve))

assert.deepEqual(statusRequests, [undefined])
assert.equal(component.nodeStatus?.supported, false)
assert.equal(component.canManagePlugins(), false)
assert.equal(component.nodeStatus?.reason, 'Node.js was not found on PATH')

await component.installPlugin(plugin)
await component.updatePlugin(plugin)
await component.uninstallPlugin(plugin)
assert.deepEqual(pluginCalls, [])
assert.deepEqual(restartRequests, [])

component.customNodePath = '  /custom/node  '
await component.refreshNodeStatus()
assert.deepEqual(statusRequests, [undefined, '/custom/node'])
assert.equal(component.nodeStatus?.supported, true)
assert.equal(component.canManagePlugins(), true)

await component.installPlugin(plugin)
await component.updatePlugin(plugin)
await component.uninstallPlugin(plugin)
assert.deepEqual(pluginCalls, [
    ['manager-install', 'fixture'],
    ['manager-update', 'fixture'],
    ['manager-uninstall', 'fixture'],
])
assert.deepEqual(restartRequests, [true, true, true])

console.log('Plugin node status UI runtime fixture passed')
