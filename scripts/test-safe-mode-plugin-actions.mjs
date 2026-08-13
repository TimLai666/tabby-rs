import assert from 'node:assert/strict'
import fs from 'node:fs'
import Module from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'tabby-core/src/components/safeModeModal.component.ts')
const source = fs.readFileSync(sourcePath, 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
        experimentalDecorators: true,
    },
    fileName: sourcePath,
}).outputText

const originalLoad = Module._load
const componentDecorator = () => target => target
const inputDecorator = () => () => undefined
Module._load = function (request, parent, isMain) {
    if (request === '@angular/core') {
        return { Component: componentDecorator, Input: inputDecorator }
    }
    if (request === '@ng-bootstrap/ng-bootstrap') {
        return { NgbActiveModal: class {} }
    }
    if (request.endsWith('/config.service') || request.endsWith('/platform')) {
        return { ConfigService: class {}, PlatformService: class {} }
    }
    return originalLoad.call(this, request, parent, isMain)
}

const componentModule = new Module(sourcePath)
componentModule.filename = sourcePath
componentModule.paths = Module._nodeModulePaths(root)
try {
    componentModule._compile(compiled, sourcePath)
} finally {
    Module._load = originalLoad
}

const retryCalls = []
globalThis.window = {
    safeModeReason: null,
    safeModeSuspectedPlugins: ['tabby-broken'],
    pluginLoadFailures: [],
    retryPluginBootstrap: () => retryCalls.push(true),
}

const config = {
    store: { pluginBlacklist: ['tabby-broken', 'tabby-keep'] },
    saveCalls: 0,
    async save () {
        this.saveCalls++
    },
}
const uninstallCalls = []
const platform = {
    async uninstallPlugin (name) {
        uninstallCalls.push(name)
    },
}
const component = new componentModule.exports.SafeModeModalComponent(
    { dismiss () {} },
    config,
    platform,
)

await component.removePlugin('tabby-broken')
assert.deepEqual(uninstallCalls, ['tabby-broken'])
assert.deepEqual(config.store.pluginBlacklist, ['tabby-keep'])
assert.equal(config.saveCalls, 1)
assert.deepEqual(retryCalls, [true])

const failingConfig = {
    store: { pluginBlacklist: ['tabby-broken'] },
    saveCalls: 0,
    async save () {
        this.saveCalls++
    },
}
const failingPlatform = {
    async uninstallPlugin () {
        throw new Error('uninstall failed')
    },
}
const failingComponent = new componentModule.exports.SafeModeModalComponent(
    { dismiss () {} },
    failingConfig,
    failingPlatform,
)
const originalConsoleError = console.error
console.error = () => {}
try {
    await failingComponent.removePlugin('tabby-broken')
} finally {
    console.error = originalConsoleError
}
assert.deepEqual(failingConfig.store.pluginBlacklist, ['tabby-broken'])
assert.equal(failingConfig.saveCalls, 0)

globalThis.window.safeModeSuspectedPlugins = ['@scope/plugin']
globalThis.window.pluginLoadFailures = []
const scopedUninstallCalls = []
const scopedConfig = {
    store: { pluginBlacklist: [] },
    saveCalls: 0,
    async save () {
        this.saveCalls++
    },
}
const scopedComponent = new componentModule.exports.SafeModeModalComponent(
    { dismiss () {} },
    scopedConfig,
    {
        async uninstallPlugin (name) {
            scopedUninstallCalls.push(name)
        },
    },
)
await scopedComponent.removePlugin('@scope/plugin')
assert.deepEqual(scopedUninstallCalls, ['@scope/plugin'])
assert.deepEqual(scopedConfig.store.pluginBlacklist, [])
assert.equal(scopedConfig.saveCalls, 1)

console.log('Safe mode plugin action contract passed')
