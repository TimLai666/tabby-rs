import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const providers = {
    web: 'tabby-web/src/services/runtimeCapabilities.service.ts',
    tauri: 'tabby-tauri/src/services/runtimeCapabilities.service.ts',
    electron: 'tabby-electron/src/services/runtimeCapabilities.service.ts',
}

for (const [host, relativePath] of Object.entries(providers)) {
    const source = fs.readFileSync(path.resolve(relativePath), 'utf8')
    assert.match(source, new RegExp(`host: '${host}'`))
    assert.match(source, /readonly capabilities: RuntimeCapabilities/)
    for (const capability of ['localPty', 'filesystem', 'keychain', 'updater', 'pluginInstall', 'serial', 'desktopNotifications']) {
        assert.match(source, new RegExp(`${capability}: (true|false)`))
    }
}

const webSource = fs.readFileSync(path.resolve(providers.web), 'utf8')
for (const capability of ['localPty', 'filesystem', 'keychain', 'updater', 'pluginInstall', 'serial', 'desktopNotifications']) {
    assert.match(webSource, new RegExp(`${capability}: false`))
}

const commonSourceRoots = ['tabby-core/src', 'tabby-terminal/src', 'tabby-settings/src', 'tabby-web/src']
function sourceFiles (directory) {
    const files = []
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const filePath = path.join(directory, entry.name)
        if (entry.isDirectory()) {
            files.push(...sourceFiles(filePath))
        } else if (entry.isFile() && entry.name.endsWith('.ts')) {
            files.push(filePath)
        }
    }
    return files
}

for (const relativeRoot of commonSourceRoots) {
    for (const filePath of sourceFiles(path.resolve(relativeRoot))) {
        const source = fs.readFileSync(filePath, 'utf8')
        assert.doesNotMatch(source, /@tauri-apps[\\/]|(?:^|["'])\.\.?\/.*tabby-tauri|src-tauri/,
            `common package imports a Tauri runtime: ${path.relative(process.cwd(), filePath)}`)
    }
}

const appRootSource = fs.readFileSync(path.resolve('tabby-core/src/components/appRoot.component.ts'), 'utf8')
assert.doesNotMatch(appRootSource, /process\.platform/, 'common app root must use the injected host platform service')
assert.match(appRootSource, /hostApp\.platform === Platform\.Windows/)
assert.match(appRootSource, /hostApp\.platform === Platform\.macOS/)
assert.match(appRootSource, /hostApp\.platform === Platform\.Linux/)

const hotkeysUtilSource = fs.readFileSync(path.resolve('tabby-core/src/services/hotkeys.util.ts'), 'utf8')
const utilsSource = fs.readFileSync(path.resolve('tabby-core/src/utils.ts'), 'utf8')
const hotkeysServiceSource = fs.readFileSync(path.resolve('tabby-core/src/services/hotkeys.service.ts'), 'utf8')
const hotkeysApiSource = fs.readFileSync(path.resolve('tabby-core/src/api/index.ts'), 'utf8')
const terminalSettingsSource = fs.readFileSync(path.resolve('tabby-terminal/src/components/terminalSettingsTab.component.ts'), 'utf8')
const oscProcessingSource = fs.readFileSync(path.resolve('tabby-terminal/src/middleware/oscProcessing.ts'), 'utf8')
assert.doesNotMatch(hotkeysUtilSource, /process\.platform/, 'common hotkey utilities must not read Node platform state')
assert.doesNotMatch(utilsSource, /process\.platform|from ['"]os['"]/, 'common utilities must not read Node platform state')
assert.match(utilsSource, /getWindows10Build \(platform: Platform, windowsBuild\?: number\)/)
assert.doesNotMatch(oscProcessingSource, /from ['"]os['"]/, 'common terminal middleware must not import Node OS APIs')
assert.match(oscProcessingSource, /setHomeDirectory \(homeDirectory\?: string\)/)
assert.doesNotMatch(hotkeysServiceSource, /process\.platform/, 'common hotkey service must use the injected host platform service')
assert.match(hotkeysServiceSource, /this\.keyPlatform = hostApp\.configPlatform/)
assert.match(hotkeysServiceSource, /getKeyName\(eventData, this\.keyPlatform\)/)
assert.match(hotkeysServiceSource, /getKeystrokeName\(\[\.\.\.this\.pressedKeys\], this\.keyPlatform\)/)
assert.match(hotkeysApiSource, /getAltKeyName/)
assert.match(hotkeysApiSource, /getMetaKeyName/)
assert.match(terminalSettingsSource, /getAltKeyName\(this\.hostApp\.configPlatform\)/)
assert.match(terminalSettingsSource, /getMetaKeyName\(this\.hostApp\.configPlatform\)/)

console.log('Runtime capability contract fixtures passed')
