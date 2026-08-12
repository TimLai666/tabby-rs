import assert from 'node:assert/strict'
import fs from 'node:fs'
import Module from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'app/src/plugin-runtime/runtime.ts')
const source = fs.readFileSync(sourcePath, 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
    },
    fileName: sourcePath,
}).outputText
const runtimeModule = new Module(sourcePath)
runtimeModule.filename = sourcePath
runtimeModule.paths = Module._nodeModulePaths(root)
runtimeModule._compile(compiled, sourcePath)

const {
    DuplicatePluginModuleError,
    MissingPluginModuleError,
    NodeRuntimeRequiredError,
    PluginModuleRegistry,
    evaluateCommonJs,
    loadPluginModules,
} = runtimeModule.exports

const registry = new PluginModuleRegistry()
const core = { marker: 'singleton' }
registry.register('tabby-core', core)
assert.equal(registry.require('tabby-core'), core)
assert.throws(() => registry.register('tabby-core', {}), DuplicatePluginModuleError)
assert.throws(() => registry.require('electron'), NodeRuntimeRequiredError)
assert.throws(() => registry.require('@electron/remote/main'), NodeRuntimeRequiredError)
assert.throws(() => registry.require('node:electron'), NodeRuntimeRequiredError)
assert.throws(() => registry.require('./native-addon.node'), NodeRuntimeRequiredError)
assert.throws(() => registry.require('node:fs'), NodeRuntimeRequiredError)
assert.throws(() => registry.require('missing-module'), MissingPluginModuleError)

const evaluated = evaluateCommonJs(
    "module.exports = { default: { forRoot: () => ({ marker: require('tabby-core').marker }) } }",
    '/plugins/tabby-good/dist/index.js',
    registry,
)
assert.equal(evaluated.default.forRoot().marker, 'singleton')

const result = await loadPluginModules({
    async discover () {
        return [
            {
                name: 'good', packageName: 'tabby-good', version: '1.0.0', path: '/plugins/tabby-good', entry: '/plugins/tabby-good/dist/index.js', isBuiltin: false, isLegacy: false, manifest: {},
            },
            {
                name: 'node-dependent', packageName: 'tabby-node-dependent', version: '1.0.0', path: '/plugins/tabby-node-dependent', entry: '/plugins/tabby-node-dependent/dist/index.js', isBuiltin: false, isLegacy: false, manifest: {},
            },
        ]
    },
    async readEntry (packageName) {
        return {
            packageName,
            entry: `/plugins/${packageName}/dist/index.js`,
            code: packageName === 'tabby-good'
                ? "module.exports = { default: { forRoot: () => ({}) } }"
                : "require('electron'); module.exports = { default: {} }",
        }
    },
}, registry)
assert.equal(result.modules.length, 1)
assert.equal(result.modules[0].pluginName, 'good')
assert.equal(result.failures.length, 1)
assert.equal(result.failures[0].code, 'node-runtime-required')

const discoveryFailure = await loadPluginModules({
    async discover () {
        throw new Error('plugin directory unavailable')
    },
    async readEntry () {
        throw new Error('unreachable')
    },
}, registry)
assert.equal(discoveryFailure.modules.length, 0)
assert.equal(discoveryFailure.failures[0].phase, 'discover')
assert.equal(discoveryFailure.failures[0].code, 'exception')

console.log('plugin runtime contract passed')
