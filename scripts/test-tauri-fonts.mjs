import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const read = name => fs.readFileSync(path.join(root, name), 'utf8')

const angular = {
    Inject: () => () => undefined,
    Injectable: () => target => target,
    NgZone: class {},
}

function load (file, dependencies = {}, globals = {}) {
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
        console: globals.console ?? console,
        window: { matchMedia: () => ({ matches: false }) },
    })
    return module.exports
}

const warnings = []
const recordingConsole = { warn: (...args) => warnings.push(args), log: () => undefined, error: () => undefined }

const { TauriPlatformService } = load('tabby-tauri/src/services/platform.service.ts', {
    'tabby-core': {
        ClipboardContent: class {}, DirectoryDownload: class {}, DirectoryUpload: class {}, FileDownload: class {}, FileUpload: class {},
        FileUploadOptions: class {}, MenuItemOptions: class {}, MessageBoxOptions: class {}, MessageBoxResult: class {},
        NodeToolchainStatus: class {}, PluginInfo: class {}, PlatformService: class {}, PlatformTheme: class {},
        sanitizeTransferName: value => value, sanitizeTransferRelativePath: value => value,
    },
    '../api/hostBridge': { ContextMenuItem: class {}, HostBridge: class {}, HostEventMap: class {}, PluginOperation: class {},
        RuntimeInfo: class {}, TAURI_RUNTIME_INFO: 'TAURI_RUNTIME_INFO', TransferDirectoryEntry: class {} },
}, { console: recordingConsole })

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

function platformWithFonts (fonts, platformName = 'linux') {
    const bridge = {
        calls: [],
        listen: async () => () => undefined,
        invoke: async (command, request) => {
            bridge.calls.push({ command, request })
            if (fonts instanceof Error) throw fonts
            return fonts
        },
    }
    return { bridge, platform: new TauriPlatformService(bridge, { platform: platformName }, {}) }
}

await check('listFonts keeps only monospace families when the native side reports flags', async () => {
    const { platform, bridge } = platformWithFonts([
        { family: 'Noto Sans', fullName: null, monospace: false, styles: ['Regular'] },
        { family: 'Fira Code', fullName: null, monospace: true, styles: ['Regular'] },
        { family: 'DejaVu Sans', fullName: null, monospace: false, styles: ['Regular'] },
        { family: 'Cascadia Mono', fullName: null, monospace: true, styles: ['Regular'] },
        { family: 'Fira Code', fullName: null, monospace: true, styles: ['Bold'] },
    ])
    const fonts = await platform.listFonts()
    assert.deepEqual([...fonts], ['Cascadia Mono', 'Fira Code'])
    assert.deepEqual(JSON.parse(JSON.stringify(bridge.calls.filter(call => call.command === 'font.list'))), [{ command: 'font.list', request: {} }])
})

await check('listFonts keeps every family when no family reports a monospace flag', async () => {
    const { platform } = platformWithFonts([
        { family: 'Noto Sans', fullName: null, monospace: null, styles: [] },
        { family: 'DejaVu Sans', fullName: null, monospace: null, styles: [] },
    ])
    assert.deepEqual([...await platform.listFonts()], ['DejaVu Sans', 'Noto Sans'])
})

await check('listFonts returns sorted, de-duplicated families with localeCompare order', async () => {
    const { platform } = platformWithFonts([
        { family: 'Zeta Mono', fullName: null, monospace: true, styles: [] },
        { family: 'alpha Mono', fullName: null, monospace: true, styles: [] },
        { family: 'Beta Mono', fullName: null, monospace: true, styles: [] },
        { family: 'Beta Mono', fullName: null, monospace: true, styles: [] },
    ])
    assert.deepEqual([...await platform.listFonts()], ['alpha Mono', 'Beta Mono', 'Zeta Mono'])
})

await check('listFonts ignores null flags when other families report monospace', async () => {
    const { platform } = platformWithFonts([
        { family: 'Unknown', fullName: null, monospace: null, styles: [] },
        { family: 'Hack', fullName: null, monospace: true, styles: [] },
    ])
    assert.deepEqual([...await platform.listFonts()], ['Hack'])
})

await check('listFonts keeps every family when the detector flags none as monospace', async () => {
    const { platform } = platformWithFonts([
        { family: 'Noto Sans', fullName: null, monospace: false, styles: [] },
        { family: 'DejaVu Sans', fullName: null, monospace: false, styles: [] },
    ])
    assert.deepEqual([...await platform.listFonts()], ['DejaVu Sans', 'Noto Sans'])
})

await check('listFonts keeps every family on Windows and macOS like upstream', async () => {
    for (const platformName of ['windows', 'macos']) {
        const { platform } = platformWithFonts([
            { family: 'Segoe UI', fullName: null, monospace: false, styles: [] },
            { family: 'Cascadia Mono', fullName: null, monospace: true, styles: [] },
        ], platformName)
        assert.deepEqual([...await platform.listFonts()], ['Cascadia Mono', 'Segoe UI'], platformName)
    }
})

await check('listFonts returns an empty list when the bridge fails', async () => {
    warnings.length = 0
    const { platform } = platformWithFonts(new Error('font enumeration failed'))
    assert.deepEqual([...await platform.listFonts()], [])
    assert.equal(warnings.length, 1)
})

if (failures.length) {
    console.error(`${failures.length} font check(s) failed`)
    process.exitCode = 1
} else {
    console.log('Tauri font list checks passed')
}
