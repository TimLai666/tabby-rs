import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Module from 'node:module'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const npmCommand = process.platform === 'win32' ? 'npm.cmd' : 'npm'
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-official-plugin-'))
const npmCache = path.join(fixture, 'npm-cache')
const publicFixtures = [
    { name: 'tabby-clippy', version: '1.0.0' },
    { name: 'tabby-jumper', version: '1.0.3' },
    { name: 'tabby-theme-hype', version: '1.0.0' },
]

const packages = publicFixtures.map(({ name, version }) => {
    execFileSync(npmCommand, [
        'pack',
        '--ignore-scripts',
        '--fetch-timeout=30000',
        '--fetch-retries=1',
        '--fetch-retry-mintimeout=1000',
        '--fetch-retry-maxtimeout=3000',
        `${name}@${version}`,
        '--pack-destination',
        fixture,
    ], {
        cwd: root,
        shell: process.platform === 'win32',
        stdio: 'pipe',
        env: {
            ...process.env,
            npm_config_cache: npmCache,
            npm_config_audit: 'false',
            npm_config_fund: 'false',
            npm_config_update_notifier: 'false',
            npm_config_loglevel: 'warn',
        },
    })
    const archive = fs.readdirSync(fixture).find(file => file === `${name}-${version}.tgz`)
    assert.ok(archive, `the public plugin package was not downloaded: ${name}@${version}`)
    const packageRoot = path.join(fixture, name)
    fs.mkdirSync(packageRoot)
    execFileSync('tar', ['-xzf', path.join(fixture, archive), '-C', packageRoot])
    const unpackedRoot = path.join(packageRoot, 'package')
    const packageJson = JSON.parse(fs.readFileSync(path.join(unpackedRoot, 'package.json'), 'utf8'))
    assert.equal(packageJson.name, name)
    assert.equal(packageJson.version, version)
    assert.ok(packageJson.keywords.includes('tabby-plugin'))
    return {
        name: name.replace(/^tabby-/, ''),
        packageName: name,
        version,
        path: unpackedRoot,
        entry: path.join(unpackedRoot, packageJson.main),
        isBuiltin: false,
        isLegacy: false,
        manifest: packageJson,
    }
})

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

const packageByName = new Map(packages.map(plugin => [plugin.packageName, plugin]))
const result = await loadPluginModules({
    async discover () {
        return packages
    },
    async readEntry (packageName) {
        const plugin = packageByName.get(packageName)
        assert.ok(plugin, `unknown public plugin package: ${packageName}`)
        return {
            packageName,
            entry: plugin.entry,
            code: fs.readFileSync(plugin.entry, 'utf8'),
        }
    },
}, registry)
assert.deepEqual(result.failures, [])
assert.equal(result.modules.length, publicFixtures.length)
assert.deepEqual(result.modules.map(module => typeof module), publicFixtures.map(() => 'function'))

console.log(`Official public plugin runtime fixtures passed: ${publicFixtures.map(({ name, version }) => `${name}@${version}`).join(', ')}`)
