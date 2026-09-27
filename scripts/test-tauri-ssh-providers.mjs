import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import ts from 'typescript'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const compilerOptions = { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true }
// Each vm realm has its own intrinsics, so assert on plain data, not cross-realm arrays.
const json = value => JSON.parse(JSON.stringify(value))

const ngModules = []
const modules = new Map()

/**
 * Stands in for a module the Tauri entry point imports. Named exports resolve to a
 * stable per-module class identity, so a provider pair can only match by real token.
 */
function moduleFor (specifier) {
    if (!modules.has(specifier)) {
        const exports = {}
        Object.defineProperty(exports, '__esModule', { value: true, enumerable: false })
        modules.set(specifier, new Proxy(exports, {
            get: (target, name) => {
                if (name in target) {
                    return target[name]
                }
                if (typeof name !== 'string') {
                    return undefined
                }
                target[name] = class {}
                Object.defineProperty(target[name], 'name', { value: `${specifier}#${name}` })
                return target[name]
            },
        }))
    }
    return modules.get(specifier)
}

moduleFor('@angular/core').NgModule = metadata => target => { ngModules.push({ metadata, target }) }
moduleFor('@angular/core').Injectable = () => target => target
const tabbyCore = moduleFor('tabby-core')

function load (relativePath) {
    const compiled = ts.transpileModule(fs.readFileSync(path.join(root, relativePath), 'utf8'), { compilerOptions }).outputText
    const loaded = { exports: {} }
    vm.runInNewContext(compiled, {
        exports: loaded.exports,
        require: specifier => {
            assert.doesNotMatch(
                specifier, /(^|\/)tabby-ssh(\/src)?$/,
                `Tauri must reach the shared SSH providers by their own module, not the tabby-ssh barrel: ${specifier}`,
            )
            return moduleFor(specifier)
        },
        console,
    })
    return loaded.exports
}

const { SSHConfigProvider } = load('tabby-ssh/src/config.ts')
moduleFor('../../tabby-ssh/src/config').SSHConfigProvider = SSHConfigProvider
const { SSHHotkeyProvider } = load('tabby-ssh/src/hotkeys.ts')
moduleFor('../../tabby-ssh/src/hotkeys').SSHHotkeyProvider = SSHHotkeyProvider

const tauriModule = load('tabby-tauri/src/index.ts').default
assert.ok(tauriModule, 'The Tauri entry point must export its NgModule class')
assert.equal(ngModules.length, 1, 'The Tauri entry point must declare exactly one NgModule')
assert.equal(ngModules[0].target, tauriModule, 'The captured metadata must belong to the exported TauriModule')

const providers = ngModules[0].metadata.providers
assert.ok(Array.isArray(providers) && providers.length > 0, 'TauriModule must declare a non-empty providers array')

/** Registrations that use `useClass` for the class, whatever token they sit under. */
const usesClass = cls => providers.filter(provider => provider && provider.useClass === cls)

for (const [label, cls, token, tokenName] of [
    ['SSHConfigProvider', SSHConfigProvider, tabbyCore.ConfigProvider, 'ConfigProvider'],
    ['SSHHotkeyProvider', SSHHotkeyProvider, tabbyCore.HotkeyProvider, 'HotkeyProvider'],
    ['TauriSftpContextMenu', moduleFor('./sftpContextMenu').TauriSftpContextMenu, tabbyCore.TabContextMenuItemProvider, 'TabContextMenuItemProvider'],
]) {
    const registrations = usesClass(cls)
    assert.equal(registrations.length, 1, `${label} must be registered exactly once, found ${registrations.length}`)
    assert.equal(registrations[0].provide, token, `${label} must register against ${tokenName}`)
    assert.equal(registrations[0].multi, true, `${label} must register as a multi provider`)
}

const configProvider = new SSHConfigProvider()
assert.deepEqual(json(configProvider.defaults), {
    ssh: {
        warnOnClose: false,
        winSCPPath: null,
        agentType: 'auto',
        agentPath: null,
        x11Display: null,
        knownHosts: [],
        verifyHostKeys: true,
    },
    hotkeys: { 'restart-ssh-session': [], 'launch-winscp': [], 'open-sftp': [] },
}, 'SSHConfigProvider must supply the ssh defaults and unbound hotkey defaults')

const hotkeyProvider = new SSHHotkeyProvider({ instant: key => `translated:${key}` })
const hotkeys = json(await hotkeyProvider.provide())
assert.deepEqual(hotkeys, [
    { id: 'restart-ssh-session', name: 'translated:Restart current SSH session' },
    { id: 'launch-winscp', name: 'translated:Launch WinSCP for current SSH session' },
    { id: 'open-sftp', name: 'translated:Open SFTP panel' },
], 'SSHHotkeyProvider must describe all three SSH hotkeys through the translate service')
for (const { id } of hotkeys) {
    assert.deepEqual(json(configProvider.defaults.hotkeys[id]), [], `${id} must ship an empty default binding`)
}

console.log('Tauri SSH settings and hotkey provider registrations passed')
