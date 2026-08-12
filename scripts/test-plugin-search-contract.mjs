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
const basePackage = {
    name: 'tabby-example',
    version: '1.2.3',
    description: 'fixture',
    keywords: ['tabby-plugin'],
    maintainers: [],
}

assert.ok(service.toPluginInfo({ package: basePackage }, 'tabby-', 'tabby-plugin'))
assert.equal(
    service.toPluginInfo({ package: { ...basePackage, keywords: ['other-keyword'] } }, 'tabby-', 'tabby-plugin'),
    null,
)
assert.ok(service.toPluginInfo({ package: { ...basePackage, keywords: ['TABBY-PLUGIN'] } }, 'tabby-', 'tabby-plugin'))

assert.match(componentSource, /catchError\(error => \{[\s\S]*availablePluginsReady = true[\s\S]*availablePluginsError/)
assert.match(componentSource, /blacklist\.includes\(plugin\.name\) \|\| blacklist\.includes\(plugin\.packageName\)/)
assert.match(componentSource, /filter\(x => x !== plugin\.name && x !== plugin\.packageName\)/)
assert.match(templateSource, /availablePluginsError/)

console.log('Plugin search keyword contract fixtures passed')
