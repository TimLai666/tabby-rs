import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Module from 'node:module'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-official-plugin-'))
const packageVersion = '1.0.0'

execFileSync('npm', ['pack', `tabby-clippy@${packageVersion}`, '--pack-destination', fixture], {
    cwd: root,
    stdio: 'ignore',
})
const archive = fs.readdirSync(fixture).find(name => name.endsWith('.tgz'))
assert.ok(archive, 'the official plugin package was not downloaded')
execFileSync('tar', ['-xzf', path.join(fixture, archive), '-C', fixture])

const packageRoot = path.join(fixture, 'package')
const packageJson = JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'))
assert.equal(packageJson.name, 'tabby-clippy')
assert.equal(packageJson.version, packageVersion)
assert.ok(packageJson.keywords.includes('tabby-plugin'))

const runtimePath = path.join(root, 'app/src/plugin-runtime/runtime.ts')
const runtimeModule = new Module(runtimePath)
runtimeModule.filename = runtimePath
runtimeModule.paths = Module._nodeModulePaths(root)
runtimeModule._compile(ts.transpileModule(fs.readFileSync(runtimePath, 'utf8'), {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
    },
    fileName: runtimePath,
}).outputText, runtimePath)

const { PluginModuleRegistry, loadPluginModules } = runtimeModule.exports
const registry = new PluginModuleRegistry()
const publicPluginApi = new Proxy({}, {
    get (_target, key) {
        if (key === '__esModule') {
            return true
        }
        return function PublicPluginApiStub () {}
    },
})
for (const name of ['tabby-core', 'tabby-terminal', 'tabby-settings']) {
    registry.register(name, publicPluginApi)
}

// Angular's partially compiled packages require the compiler facade during
// evaluation. These are the same singleton packages registered by entry.tauri.
await import('@angular/compiler')
for (const name of ['@angular/core', '@angular/common', '@angular/forms', 'rxjs']) {
    registry.register(name, await import(name))
}

const descriptor = {
    name: packageJson.name,
    packageName: packageJson.name,
    version: packageJson.version,
    path: packageRoot,
    entry: path.join(packageRoot, packageJson.main),
    isBuiltin: false,
    isLegacy: false,
    manifest: packageJson,
}
const result = await loadPluginModules({
    async discover () {
        return [descriptor]
    },
    async readEntry () {
        return {
            packageName: packageJson.name,
            entry: descriptor.entry,
            code: fs.readFileSync(descriptor.entry, 'utf8'),
        }
    },
}, registry)
assert.deepEqual(result.failures, [])
assert.equal(result.modules.length, 1)
assert.equal(typeof result.modules[0], 'function')

console.log(`Official plugin runtime fixture passed: tabby-clippy@${packageVersion}`)
