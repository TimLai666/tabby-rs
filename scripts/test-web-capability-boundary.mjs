import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-web/src/platform.ts'), 'utf8')
const pluginSettings = fs.readFileSync(path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.pug'), 'utf8')

assert.match(source, /supportsPluginManagement = false/)
assert.match(source, /readClipboard \(\): string \{[\s\S]*?new UnsupportedCapabilityError\('clipboard'\)/)
assert.match(source, /async readClipboardText \(\): Promise<string> \{[\s\S]*navigator\.clipboard[\s\S]*readText\(\)[\s\S]*?new UnsupportedCapabilityError\('clipboard'\)/)
assert.match(source, /openPath \(_path: string\): void \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)
assert.match(source, /showItemInFolder \(_path: string\): void \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)
assert.match(source, /installPlugin \(_name: string, _version: string\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('pluginInstall'\)/)
assert.match(source, /uninstallPlugin \(_name: string\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('pluginInstall'\)/)
assert.match(source, /cancelPluginOperation \(_id: string\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('pluginInstall'\)/)
assert.match(source, /getWinSCPPath \(\): string\|null \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)
assert.match(source, /async exec \(_app: string, _argv: string\[\]\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('localPty'\)/)
assert.match(source, /const onCancel = \(\) => finish\(\[\]\)/)
assert.match(source, /this\.fileSelector\.multiple = options\?\.multiple \?\? false/)
assert.match(source, /this\.fileSelector\.oncancel = onCancel/)
assert.match(source, /this\.fileSelector\.oncancel = null/)
assert.match(source, /this\.fileSelector\.value = ''/)

const hostWindow = fs.readFileSync(path.join(root, 'tabby-web/src/services/hostWindow.service.ts'), 'utf8')
assert.match(hostWindow, /minimize \(\): void \{[\s\S]*?new UnsupportedCapabilityError\('windowControls'\)/)
assert.match(hostWindow, /toggleMaximize \(\): void \{[\s\S]*?new UnsupportedCapabilityError\('windowControls'\)/)
assert.match(pluginSettings, /button\.btn\.btn-secondary\.btn-sm\.ms-auto\(\*ngIf='canManagePlugins\(\)'/)

const transpile = source => ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
    },
}).outputText
const runtimeRequire = createRequire(path.join(root, 'tabby-web/webpack.config.mjs'))
class RuntimePlatformService {
    constructor () {
        this.fileTransferStarted = { next () {} }
    }
}
class RuntimeUnsupportedCapabilityError extends Error {
    code = 'UNSUPPORTED_CAPABILITY'

    constructor (capability) {
        super(`Capability "${capability}" is not available in this host`)
        this.name = 'UnsupportedCapabilityError'
        this.capability = capability
    }
}
const createElement = () => ({
    addEventListener () {},
    click () {},
    style: {},
})
const document = {
    body: { appendChild () {} },
    createElement,
}
const window = {
    addEventListener () {},
    close () {},
    document,
    open () {},
}
const webPlatformModule = { exports: {} }
vm.runInNewContext(transpile(source), {
    console,
    document,
    exports: webPlatformModule.exports,
    module: webPlatformModule,
    navigator: { clipboard: { readText: async () => { throw new Error('clipboard denied') } } },
    require: name => {
        if (name === '@angular/core') return { Inject: () => target => target, Injectable: () => target => target }
        if (name === '@ng-bootstrap/ng-bootstrap') return { NgbModal: class {} }
        if (name === '@vaadin/vaadin-context-menu') return {}
        if (name === 'copy-text-to-clipboard') return () => {}
        if (name === 'tabby-core') return {
            FileDownload: class {},
            FileUpload: class {},
            PlatformService: RuntimePlatformService,
            UnsupportedCapabilityError: RuntimeUnsupportedCapabilityError,
        }
        if (name === './components/messageBoxModal.component') return { MessageBoxModalComponent: class {} }
        if (name === './styles.scss') return {}
        return runtimeRequire(name)
    },
    window,
}, { filename: 'platform.ts' })

const { WebPlatformService } = webPlatformModule.exports
const platform = new WebPlatformService({ loadConfig: async () => '', saveConfig: async () => {}, getAppVersion: () => 'test' }, {})
const expectUnsupported = async (capability, operation) => {
    await assert.rejects(Promise.resolve().then(operation), error => {
        assert.equal(error.name, 'UnsupportedCapabilityError')
        assert.equal(error.code, 'UNSUPPORTED_CAPABILITY')
        assert.equal(error.capability, capability)
        return true
    })
}

for (const [capability, operation] of [
    ['clipboard', () => platform.readClipboard()],
    ['clipboard', () => platform.readClipboardText()],
    ['filesystem', () => platform.startDownloadDirectory('downloads')],
    ['filesystem', () => platform.startUploadDirectory()],
    ['filesystem', () => platform.showItemInFolder('file')],
    ['filesystem', () => platform.getWinSCPPath()],
    ['filesystem', () => platform.pickDirectory()],
    ['filesystem', () => platform.openPath('file')],
    ['pluginInstall', () => platform.installPlugin('plugin', '1.0.0')],
    ['pluginInstall', () => platform.updatePlugin('plugin')],
    ['pluginInstall', () => platform.uninstallPlugin('plugin')],
    ['pluginInstall', () => platform.cancelPluginOperation('operation')],
    ['localPty', () => platform.exec('shell', [])],
]) {
    await expectUnsupported(capability, operation)
}

const updaterSource = fs.readFileSync(path.join(root, 'tabby-web/src/services/updater.service.ts'), 'utf8')
const updaterModule = { exports: {} }
vm.runInNewContext(transpile(updaterSource), {
    exports: updaterModule.exports,
    module: updaterModule,
    require: name => name === 'tabby-core'
        ? { UnsupportedCapabilityError: RuntimeUnsupportedCapabilityError, UpdaterService: class {} }
        : runtimeRequire(name),
}, { filename: 'updater.service.ts' })
const updater = new updaterModule.exports.NullUpdaterService()
assert.equal(await updater.check(), null)
await expectUnsupported('updater', () => updater.download({}))
await expectUnsupported('updater', () => updater.install({}))
await expectUnsupported('updater', () => updater.setChannel('stable'))
await expectUnsupported('updater', () => updater.getChannel())

console.log('Web capability boundary contract passed')
