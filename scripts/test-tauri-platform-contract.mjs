import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import ts from 'typescript'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platform = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8')
const shellProvider = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/shellProvider.service.ts'), 'utf8')
const pluginSettings = fs.readFileSync(path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.ts'), 'utf8')
const bridge = fs.readFileSync(path.join(root, 'tabby-tauri/src/api/hostBridge.ts'), 'utf8')
const hostApp = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/hostApp.service.ts'), 'utf8')
const desktop = fs.readFileSync(path.join(root, 'src-tauri/src/commands/desktop.rs'), 'utf8')
const registration = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8')
const tauriConfig = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'))
const capabilities = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/capabilities/default.json'), 'utf8'))
const tauriEntry = fs.readFileSync(path.join(root, 'app/src/entry.tauri.ts'), 'utf8')
const tauriPolyfills = fs.readFileSync(path.join(root, 'app/src/tauri-polyfills.ts'), 'utf8')
const tauriWebpack = fs.readFileSync(path.join(root, 'app/webpack.config.tauri.mjs'), 'utf8')

assert.match(platform, /async exec \(app: string, argv: string\[\]\): Promise<void> \{[\s\S]*?desktop\.exec/)
assert.match(platform, /getWinSCPPath \(\): string \| null \{\s*return null\s*\}/)
assert.doesNotMatch(shellProvider, /tabby-electron\/src\/icons/)
assert.match(shellProvider, /require\('\.\.\/icons\/alpine\.svg'\)/)
assert.doesNotMatch(pluginSettings, /FORCE_ENABLE\s*=\s*\[[^\]]*tabby-electron/)
assert.ok(fs.existsSync(path.join(root, 'tabby-tauri/src/icons/alpine.svg')))
assert.match(bridge, /'desktop\.exec':[\s\S]*?request: \{ executable: string; args: string\[\] \}/)
assert.match(desktop, /pub async fn desktop_exec/)
assert.match(desktop, /Command::new\(&request\.executable\)\s*\.args\(&request\.args\)/)
assert.doesNotMatch(desktop, /Command::new\("(?:sh|bash|cmd|powershell)"\)/)
assert.match(registration, /desktop_exec/)
assert.match(registration, /use message_box::dialog_message/)
const messageBox = fs.readFileSync(path.join(root, 'src-tauri/src/message_box/mod.rs'), 'utf8')
assert.match(messageBox, /window: tauri::WebviewWindow/, 'Dialogs must use the invoking window as their owner')
for (const accessor of ['ns_window', 'gtk_window', 'hwnd']) {
    assert.match(messageBox, new RegExp(`window\\s*\\.\\s*${accessor}\\(\\)`), `Dialog owner must be forwarded via ${accessor}`)
}
assert.match(registration, /\.invoke_handler\(tauri::generate_handler!\[[\s\S]*?\bdialog_message,/)
assert.match(registration, /\.invoke_handler\(tauri::generate_handler!\[[\s\S]*?\bmenu_popup,/)
assert.equal(tauriConfig.bundle.resources['src/context_menu/empty_submenu_labels.NOTICE.md'], 'licenses/chromium-menu-translations.md')
assert.deepEqual(tauriConfig.app.security.dangerousDisableAssetCspModification, ['style-src'])
assert.equal(tauriConfig.app.windows[0].width, 1100)
assert.equal(tauriConfig.app.windows[0].height, 720)
assert.equal(tauriConfig.app.windows[0].titleBarStyle, 'Overlay')
assert.equal(tauriConfig.app.windows[0].hiddenTitle, true)
assert.equal(tauriConfig.app.windows[0].visible, false)
assert.match(desktop, /\.title_bar_style\(tauri::TitleBarStyle::Overlay\)[\s\S]*?\.hidden_title\(true\)[\s\S]*?\.visible\(false\)/)
assert.match(hostApp, /emitReady \(\): void \{[\s\S]*?window\.applyState', \{ visible: true \}/)
assert.ok(capabilities.permissions.includes('notification:default'))
assert.match(tauriEntry, /import ['"]\.\/tauri-polyfills['"]\r?\n/)
assert.match(tauriEntry, /import ['"]source-sans-pro\/source-sans-pro\.css['"]\r?\n/)
assert.match(tauriEntry, /import ['"]source-code-pro\/source-code-pro\.css['"]\r?\n/)
assert.match(tauriPolyfills, /setImmediate/)
assert.match(tauriWebpack, /test: \/logo\\\.svg\$\/[\s\S]*?type: 'asset\/resource'/)
assert.match(tauriWebpack, /test: \/\\\.svg\$\/[\s\S]*?svg-inline-loader[\s\S]*?exclude: \/logo\\\.svg\$\//)

const compiled = ts.transpileModule(platform, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
}).outputText
const platformModule = { exports: {} }
vm.runInNewContext(compiled, {
    exports: platformModule.exports,
    require: name => {
        if (name === '@angular/core') return { Injectable: () => target => target, Inject: () => () => {} }
        if (name === 'tabby-core') return { PlatformService: class {}, FileUpload: class {}, FileDownload: class {}, DirectoryDownload: class {} }
        if (name === '../api/hostBridge') return {}
        throw new Error(`Unexpected dependency: ${name}`)
    },
    console,
    window: {
        confirm: () => { throw new Error('Native choices must not collapse into window.confirm') },
        alert: () => { throw new Error('Native message boxes must preserve their button labels') },
    },
})
const provider = Object.create(platformModule.exports.TauriPlatformService.prototype)
const calls = []
provider.bridge = { invoke: async (command, request) => calls.push({ command, request }) }
for (const content of [
    { text: '紅色 台灣', html: '<span style="color:red">紅色 台灣</span>' },
    { text: 'plain text' },
    { text: '', html: '' },
]) {
    provider.setClipboard(content)
    const call = calls.pop()
    assert.equal(call.command, 'clipboard.writeText')
    assert.equal(call.request.text, content.text)
    assert.equal(call.request.html, content.html, 'Copy with formatting must preserve HTML across the native bridge')
    assert.equal(provider.clipboardText, content.text)
}

for (const options of [
    { type: 'warning', message: 'Delete?', buttons: ['Delete', 'Keep'], defaultId: 1, cancelId: 1 },
    { type: 'error', message: 'Unlock failed', detail: 'Test detail', buttons: ['Try again', 'Erase config', 'Quit'], defaultId: 0 },
    { type: 'warning', message: 'Notice', buttons: ['Understood'] },
]) {
    for (let response = 0; response < options.buttons.length; response++) {
        provider.bridge.invoke = async (command, request) => {
            assert.equal(command, 'dialog.message')
            assert.deepEqual(request, options)
            return { response }
        }
        assert.equal((await provider.showMessageBox(options)).response, response)
    }
}
provider.runtimeInfo = { platform: 'macos' }
provider.zone = { run: fn => fn() }
const selectedMenus = []
const menuRequests = []
const menuResolvers = []
provider.bridge.invoke = (command, request) => {
    assert.equal(command, 'menu.popup')
    menuRequests.push(JSON.parse(JSON.stringify(request)))
    return new Promise(resolve => menuResolvers.push(resolve))
}
const menu = [
    { label: 'Rename', commandLabel: 'Rename tab', click: () => selectedMenus.push('rename') },
    { type: 'separator' },
    { label: 'Disabled', enabled: false, submenu: [{ label: 'Blocked', click: () => selectedMenus.push('blocked') }] },
    { label: 'Color', sublabel: 'Blue', submenu: [
        { type: 'radio', label: 'Red', checked: false, click: () => selectedMenus.push('red') },
        { type: 'radio', label: 'Blue', checked: true, click: () => selectedMenus.push('blue') },
    ] },
]
provider.popupContextMenu(menu)
assert.equal(menuRequests[0].items[0].label, 'Rename')
assert.equal(menuRequests[0].items[3].sublabel, 'Blue')
assert.equal(menuRequests[0].items[3].submenu[1].checked, true)
assert.equal(menuRequests[0].items[2].enabled, false)
assert.ok(!JSON.stringify(menuRequests[0]).includes('commandLabel'))
assert.ok(!JSON.stringify(menuRequests[0]).includes('click'))
provider.popupContextMenu([{ label: 'Second menu', click: () => selectedMenus.push('second') }])
menuResolvers.shift()({ selectedId: menuRequests[0].items[3].submenu[1].id })
await Promise.resolve()
menuResolvers.shift()({ selectedId: menuRequests[1].items[0].id })
await Promise.resolve()
assert.deepEqual(selectedMenus, ['blue', 'second'], 'Each popup must retain its own callbacks')
for (const selectedId of [null, menuRequests[0].items[2].submenu[0].id, 99999]) {
    provider.popupContextMenu(menu)
    menuResolvers.shift()({ selectedId })
    await Promise.resolve()
}
assert.deepEqual(selectedMenus, ['blue', 'second'], 'Dismissal and disabled descendants must not invoke actions')

provider.bridge.invoke = async () => { throw new Error('Native dialog unavailable') }
await assert.rejects(provider.showMessageBox({ type: 'error', message: 'Test', buttons: ['OK'] }), /Native dialog unavailable/)

console.log('Tauri platform, clipboard, native dialog, and context menu contracts passed')
