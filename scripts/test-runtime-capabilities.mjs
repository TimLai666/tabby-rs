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

console.log('Runtime capability contract fixtures passed')
