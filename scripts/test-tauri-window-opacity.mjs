import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const root = process.cwd()
const cargo = fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8')
const platform = fs.readFileSync(path.join(root, 'src-tauri/src/platform/mod.rs'), 'utf8')
const desktop = fs.readFileSync(path.join(root, 'src-tauri/src/desktop.rs'), 'utf8')
const hostWindow = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/hostWindow.service.ts'), 'utf8')

assert.match(
    cargo,
    /windows-sys\s*=\s*\{[\s\S]*?Win32_Storage_FileSystem[\s\S]*?Win32_UI_WindowsAndMessaging[\s\S]*?\}/,
    'Windows opacity needs the user32 window-style APIs',
)
assert.match(
    cargo,
    /objc2-app-kit\s*=\s*\{[\s\S]*?NSWindow[\s\S]*?\}/,
    'macOS opacity needs the AppKit NSWindow binding',
)
assert.match(platform, /\.run_on_main_thread\(/, 'native opacity must run on the UI thread')
assert.match(platform, /setAlphaValue\(/, 'macOS opacity must update NSWindow alphaValue')
assert.match(platform, /SetLayeredWindowAttributes\(/, 'Windows opacity must update layered-window alpha')
assert.match(platform, /WS_EX_LAYERED/, 'Windows opacity must preserve the layered-window style')
assert.match(
    desktop,
    /opacity:\s*cfg!\(any\(target_os\s*=\s*"windows",\s*target_os\s*=\s*"macos"\)\)/,
    'opacity capability must match native platform support',
)
assert.match(
    platform,
    /target_os\s*=\s*"linux"[\s\S]*?window opacity is unavailable on Linux/,
    'Linux opacity must remain an explicit unsupported capability',
)
assert.match(
    hostWindow,
    /setOpacity \(opacity: number\): void \{\s*void this\.apply\(\{ opacity \}\)/,
    'opacity updates must not be dropped before capabilities are refreshed',
)

console.log('Tauri native window opacity contract passed')

const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript'), rx = require('rxjs')
function loadClass (source, dependencies = {}, globals = {}) {
    const module = { exports: {} }
    const output = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(output, {
        exports: module.exports, module, console, ...globals,
        require: name => {
            if (name === 'rxjs') { return rx }
            if (name === '@angular/core') { return { Injectable: () => x => x, Inject: () => () => {} } }
            if (name in dependencies) { return dependencies[name] }
            throw new Error('Unprovided dependency: ' + name)
        },
    })
    return module.exports
}
const core = loadClass(fs.readFileSync(path.join(root, 'tabby-core/src/api/platform.ts'), 'utf8'))
const native = loadClass(fs.readFileSync(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8'),
    { 'tabby-core': core, '../api/hostBridge': {} },
    { window: { matchMedia: () => ({ matches: false }) } })
const blocks = fs.readFileSync(path.join(root, 'tabby-settings/src/components/windowSettingsTab.component.pug'), 'utf8').split(/(?=^\.form-line)/m)
function visible (label, platform) {
    const block = blocks.find(block => block.split('\n').some(line => /\.title/.test(line) && line.trim().endsWith(label)))
    assert.ok(block, 'actual template row exists: ' + label)
    const condition = /^\.form-line\(\*ngIf='([^']+)'\)/.exec(block)?.[1]
    assert.ok(condition, 'row is capability gated: ' + label)
    return Boolean(Function('platform', 'return ' + condition)(platform))
}
for (const [platform, expected] of [['macos', true], ['windows', true], ['linux', false]]) {
    const instance = new native.TauriPlatformService({
        listen: async () => () => {},
        invoke: async () => '',
    }, { platform }, {})
    await instance.fileDropReady
    assert.equal(visible('Opacity', instance), expected, platform + ' opacity row')
    assert.equal(visible('Vibrancy', instance), expected, platform + ' vibrancy row')
    assert.equal(visible('Window frame', instance), false, 'unsupported native frame remains hidden')
}
const legacy = new core.PlatformService()
for (const supportsWindowControls of [false, true]) {
    legacy.supportsWindowControls = supportsWindowControls
    for (const label of ['Opacity', 'Vibrancy', 'Window frame']) {
        assert.equal(visible(label, legacy), supportsWindowControls, 'existing host visibility: ' + label)
    }
}
console.log('Window effect settings visibility passed for Native macOS/Windows/Linux and existing host capabilities')
