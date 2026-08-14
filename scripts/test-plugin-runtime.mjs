import assert from 'node:assert/strict'
import fs from 'node:fs'
import Module from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'app/src/plugin-runtime/runtime.ts')
const fixturesRoot = path.join(root, 'test/fixtures/plugin-runtime')
const source = fs.readFileSync(sourcePath, 'utf8')
const entrySource = fs.readFileSync(path.join(root, 'app/src/entry.tauri.ts'), 'utf8')
assert.match(entrySource, /if \(!fallbackUsed\) \{[\s\S]*plugins\.bootstrapSucceeded/)
assert.doesNotMatch(entrySource, /tabby:terminal-ready/)
assert.match(entrySource, /updateProgress\(100\)[\s\S]*if \(runtimeInfo\.benchmarkReadyFile\)[\s\S]*app\.benchmarkReady/)
assert.match(entrySource, /import \* as AngularLocalize from '@angular\/localize'/)
assert.match(entrySource, /import \* as AngularLocalizeInit from '@angular\/localize\/init'/)
assert.match(entrySource, /registry\.register\('@angular\/localize', AngularLocalize\)/)
assert.match(entrySource, /registry\.register\('@angular\/localize\/init', AngularLocalizeInit\)/)
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
const angular = { marker: 'angular-singleton' }
registry.register('tabby-core', core)
registry.register('terminus-core', core)
registry.register('@angular/core', angular)
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

function fixtureDescriptor (directory) {
    const fixturePath = path.join(fixturesRoot, directory)
    const manifest = JSON.parse(fs.readFileSync(path.join(fixturePath, 'package.json'), 'utf8'))
    const packageName = manifest.name
    return {
        name: packageName.replace(/^(?:tabby|terminus)-/, ''),
        packageName,
        version: manifest.version,
        path: fixturePath,
        entry: path.join(fixturePath, manifest.main),
        isBuiltin: false,
        isLegacy: packageName.startsWith('terminus-'),
        manifest,
    }
}

const fixtureDirectories = [
    'tabby-fixture-electron',
    'tabby-fixture-for-root',
    'tabby-fixture-class',
    'tabby-fixture-invalid-export',
    'tabby-fixture-null-default',
    'tabby-fixture-missing-module',
    'tabby-fixture-native',
    'tabby-fixture-pure',
    'tabby-fixture-throw',
    'terminus-fixture-legacy',
]
const fixtureDescriptors = fixtureDirectories.map(fixtureDescriptor)
const fixtureByPackageName = new Map(fixtureDescriptors.map(plugin => [plugin.packageName, plugin]))
const lifecycleEvents = []
const result = await loadPluginModules({
    async discover () {
        return fixtureDescriptors
    },
    async readEntry (packageName) {
        const plugin = fixtureByPackageName.get(packageName)
        assert.ok(plugin, `unknown fixture package: ${packageName}`)
        return {
            packageName,
            entry: plugin.entry,
            code: fs.readFileSync(plugin.entry, 'utf8'),
        }
    },
}, registry, [], {
    pluginStarted (plugin) {
        lifecycleEvents.push(['started', plugin.packageName])
    },
    pluginCompleted (plugin) {
        lifecycleEvents.push(['completed', plugin.packageName])
    },
})
assert.equal(result.modules.length, 4)
assert.deepEqual(result.modules.map(module => module.pluginName), [
    'fixture-for-root',
    'fixture-class',
    'fixture-pure',
    'fixture-legacy',
])
const pureModule = result.modules.find(module => module.pluginName === 'fixture-pure')
assert.equal(pureModule.core, core)
assert.equal(pureModule.angular, angular)
const forRootModule = result.modules.find(module => module.pluginName === 'fixture-for-root')
assert.equal(forRootModule.fixture, 'for-root')
const classModule = result.modules.find(module => module.pluginName === 'fixture-class')
assert.equal(typeof classModule, 'function')
const legacyModule = result.modules.find(module => module.pluginName === 'fixture-legacy')
assert.equal(legacyModule.fixture, 'legacy')
assert.equal(legacyModule.core, core)
assert.deepEqual(result.failures.map(failure => [failure.plugin.name, failure.code]).sort(), [
    ['fixture-missing-module', 'missing-module'],
    ['fixture-electron', 'node-runtime-required'],
    ['fixture-invalid-export', 'invalid-export'],
    ['fixture-null-default', 'invalid-export'],
    ['fixture-native', 'node-runtime-required'],
    ['fixture-throw', 'exception'],
].sort())
assert.deepEqual(
    lifecycleEvents.filter(([event]) => event === 'started').map(([, packageName]) => packageName),
    fixtureDirectories.map(directory => fixtureDescriptor(directory).packageName),
)
assert.deepEqual(
    lifecycleEvents.filter(([event]) => event === 'completed').map(([, packageName]) => packageName),
    ['tabby-fixture-for-root', 'tabby-fixture-class', 'tabby-fixture-pure', 'terminus-fixture-legacy'],
)

const blacklistedReads = []
const blacklistedEvents = []
const blacklistedResult = await loadPluginModules({
    async discover () {
        return fixtureDescriptors
    },
    async readEntry (packageName) {
        blacklistedReads.push(packageName)
        const plugin = fixtureByPackageName.get(packageName)
        assert.ok(plugin, `unknown fixture package: ${packageName}`)
        return {
            packageName,
            entry: plugin.entry,
            code: fs.readFileSync(plugin.entry, 'utf8'),
        }
    },
}, registry, ['tabby-fixture-pure'], {
    pluginStarted (plugin) {
        blacklistedEvents.push(['started', plugin.packageName])
    },
    pluginCompleted (plugin) {
        blacklistedEvents.push(['completed', plugin.packageName])
    },
})
assert.equal(blacklistedResult.modules.some(module => module.pluginName === 'fixture-pure'), false)
assert.equal(blacklistedReads.includes('tabby-fixture-pure'), false)
assert.equal(blacklistedEvents.some(([, packageName]) => packageName === 'tabby-fixture-pure'), false)

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
