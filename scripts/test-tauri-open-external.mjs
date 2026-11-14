import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

// Mirrors tabby-electron platform.service.ts openExternal: safe schemes open
// directly; other app-specific schemes ask first; file, javascript and data
// URLs never reach the native opener.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const read = name => fs.readFileSync(path.join(root, name), 'utf8')

const angular = {
    Inject: () => () => undefined,
    Injectable: () => target => target,
    NgZone: class {},
}

function load (file, dependencies = {}) {
    const module = { exports: {} }
    const output = ts.transpileModule(read(file), { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(output, {
        exports: module.exports,
        module,
        require: name => {
            if (name === '@angular/core') return angular
            if (name === 'rxjs') return require('rxjs')
            if (name in dependencies) return dependencies[name]
            throw new Error(`Unprovided dependency: ${name}`)
        },
        console,
        URL,
        window: { matchMedia: () => ({ matches: false }) },
    })
    return module.exports
}

const { TauriPlatformService } = load('tabby-tauri/src/services/platform.service.ts', {
    'tabby-core': {
        ClipboardContent: class {}, DirectoryDownload: class {}, DirectoryUpload: class {}, FileDownload: class {}, FileUpload: class {},
        FileUploadOptions: class {}, MenuItemOptions: class {}, MessageBoxOptions: class {}, MessageBoxResult: class {},
        NodeToolchainStatus: class {}, PluginInfo: class {}, PlatformService: class {}, PlatformTheme: class {},
        sanitizeTransferName: value => value, sanitizeTransferRelativePath: value => value,
    },
    '../api/hostBridge': { ContextMenuItem: class {}, HostBridge: class {}, HostEventMap: class {}, PluginOperation: class {},
        RuntimeInfo: class {}, TAURI_RUNTIME_INFO: 'TAURI_RUNTIME_INFO', TransferDirectoryEntry: class {} },
})

function makePlatform (dialogResponse) {
    const bridge = {
        calls: [],
        dialogs: [],
        listen: async () => () => undefined,
        invoke: async (command, request) => {
            bridge.calls.push({ command, request: JSON.parse(JSON.stringify(request)) })
            if (command === 'dialog.message') {
                bridge.dialogs.push(JSON.parse(JSON.stringify(request)))
                return { response: dialogResponse }
            }
            return null
        },
    }
    const translate = { instant: text => text }
    const platform = new TauriPlatformService(bridge, { platform: 'linux' }, {}, translate)
    return { bridge, platform }
}

const opened = bridge => bridge.calls.filter(call => call.command === 'desktop.openExternal')

const failures = []
async function check (name, run) {
    try {
        await run()
        console.log(`PASS ${name}`)
    } catch (error) {
        failures.push(name)
        console.error(`FAIL ${name}: ${error?.stack ?? error}`)
    }
}

for (const url of ['https://example.com/a?b=1', 'http://example.com', 'ftp://files.example.com/x', 'mailto:user@example.com']) {
    await check(`safe scheme opens directly without a prompt: ${url.split(':')[0]}`, async () => {
        const { bridge, platform } = makePlatform(1)
        await platform.openExternal(url)
        assert.deepEqual(bridge.dialogs, [])
        assert.deepEqual(opened(bridge).map(call => call.request), [{ url, confirmed: false }])
    })
}

await check('app-specific scheme asks with the upstream wording and opens on Open', async () => {
    const url = 'steam://run/440'
    const { bridge, platform } = makePlatform(0)
    await platform.openExternal(url)
    assert.equal(bridge.dialogs.length, 1)
    const dialog = bridge.dialogs[0]
    assert.equal(dialog.type, 'warning')
    assert.equal(dialog.message, 'Open this app-specific "steam" URI?')
    assert.equal(dialog.detail, url)
    assert.deepEqual(dialog.buttons, ['Open', 'Cancel'])
    assert.equal(dialog.defaultId, 0)
    assert.equal(dialog.cancelId, 1)
    assert.deepEqual(opened(bridge).map(call => call.request), [{ url, confirmed: true }])
})

await check('app-specific scheme does nothing when the user cancels', async () => {
    const { bridge, platform } = makePlatform(1)
    await platform.openExternal('zoommtg://join?confno=1')
    assert.equal(bridge.dialogs.length, 1)
    assert.deepEqual(opened(bridge), [])
})

for (const url of ['file:///etc/passwd', 'javascript:alert(1)', 'data:text/html,hi']) {
    await check(`forbidden scheme never prompts or opens: ${url.split(':')[0]}`, async () => {
        const { bridge, platform } = makePlatform(0)
        await platform.openExternal(url)
        assert.deepEqual(bridge.dialogs, [])
        assert.deepEqual(opened(bridge), [])
    })
}

console.log(failures.length ? `Tauri openExternal: ${failures.length} failures` : 'Tauri openExternal checks passed')
if (failures.length) process.exitCode = 1
