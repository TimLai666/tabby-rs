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

const infos = []
const recordingConsole = { info: (...args) => infos.push(args), warn: () => undefined, log: () => undefined, error: () => undefined }

// The real transfer base classes come from tabby-core, so close, cancel and completion
// state behave as they do in the application.
const core = load('tabby-core/src/api/platform.ts', {}, { console: recordingConsole })
const { TauriPlatformService } = load('tabby-tauri/src/services/platform.service.ts', {
    'tabby-core': {
        ClipboardContent: class {}, DirectoryUpload: class {}, MenuItemOptions: class {}, MessageBoxOptions: class {},
        MessageBoxResult: class {}, NodeToolchainStatus: class {}, PluginInfo: class {}, PlatformTheme: class {},
        sanitizeTransferName: value => value, sanitizeTransferRelativePath: value => value,
        ...core,
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

async function flush () {
    for (let i = 0; i < 20; i++) {
        await Promise.resolve()
    }
}

// Fake native bridge. Individual commands can be overridden per test.
function createBridge (overrides = {}) {
    let nextInhibitId = 0
    const bridge = {
        calls: [],
        listen: async () => () => undefined,
        invoke: async (command, request) => {
            bridge.calls.push({ command, request: JSON.parse(JSON.stringify(request ?? null)) })
            if (command in overrides) {
                return overrides[command](request)
            }
            switch (command) {
                case 'power.inhibitAcquire': return { id: ++nextInhibitId }
                case 'power.inhibitRelease': return null
                case 'transfer.openUpload': return [{ id: 'upload-1', name: 'a.bin', size: 3 }]
                case 'transfer.read': return []
                case 'transfer.write': return null
                case 'transfer.close': return null
                case 'transfer.cancel': return null
                case 'dialog.save': return '/downloads/a.bin'
                case 'dialog.open': return ['/downloads']
                case 'transfer.openDownload': return { id: 'download-1', name: 'a.bin' }
                case 'transfer.createDirectory': return null
                default: throw new Error(`Unexpected command ${command}`)
            }
        },
    }
    return bridge
}

function platformFor (bridge) {
    return new TauriPlatformService(bridge, { platform: 'linux' }, {})
}

const powerCalls = bridge => bridge.calls.filter(call => call.command.startsWith('power.'))
const releases = bridge => powerCalls(bridge).filter(call => call.command === 'power.inhibitRelease').map(call => call.request.id)
const acquires = bridge => powerCalls(bridge).filter(call => call.command === 'power.inhibitAcquire').length

await check('upload acquires on start and releases once after completion and close', async () => {
    const bridge = createBridge({
        'transfer.read': () => [],
    })
    const [upload] = await platformFor(bridge).startUpload({ multiple: false }, ['/src/a.bin'])
    assert.equal(acquires(bridge), 1, 'acquire must start with the transfer')
    assert.deepEqual(releases(bridge), [])

    const first = await upload.read()
    assert.equal(first.length, 0)
    await upload.closeAsync()
    await flush()
    upload.close()
    await flush()
    assert.deepEqual(releases(bridge), [1], 'one release for the single acquire')
})

await check('download releases after its last write is closed', async () => {
    const bridge = createBridge()
    const download = await platformFor(bridge).startDownload('a.bin', 0o644, 3)
    assert.equal(acquires(bridge), 1)
    await download.write(new Uint8Array([1, 2, 3]))
    assert.equal(download.isComplete(), true)
    assert.deepEqual(releases(bridge), [])
    await download.closeAsync()
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('directory download releases on close and ignores repeated close', async () => {
    const bridge = createBridge()
    const directory = await platformFor(bridge).startDownloadDirectory('photos', 10)
    assert.equal(acquires(bridge), 1)
    directory.close()
    await flush()
    directory.close()
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('cancelling an upload releases the inhibit exactly once', async () => {
    const bridge = createBridge()
    const [upload] = await platformFor(bridge).startUpload({ multiple: false }, ['/src/a.bin'])
    upload.cancel()
    await flush()
    upload.cancel()
    await upload.closeAsync()
    await flush()
    assert.deepEqual(releases(bridge), [1])
    assert.equal(upload.isCancelled(), true)
})

await check('cancelling a download releases the inhibit', async () => {
    const bridge = createBridge()
    const download = await platformFor(bridge).startDownload('a.bin', 0o644, 3)
    download.cancel()
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('cancelling a directory download releases the inhibit', async () => {
    const bridge = createBridge()
    const directory = await platformFor(bridge).startDownloadDirectory('photos', 10)
    directory.cancel()
    await flush()
    directory.close()
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('an upload read error releases the inhibit and still reports the error', async () => {
    const bridge = createBridge({
        'transfer.read': () => { throw { code: 'Io', details: 'read failed' } },
    })
    const [upload] = await platformFor(bridge).startUpload({ multiple: false }, ['/src/a.bin'])
    await assert.rejects(() => upload.read(), error => error.code === 'Io')
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('a download write error releases the inhibit and still reports the error', async () => {
    const bridge = createBridge({
        'transfer.write': () => { throw { code: 'Io', details: 'write failed' } },
    })
    const download = await platformFor(bridge).startDownload('a.bin', 0o644, 3)
    await assert.rejects(() => download.write(new Uint8Array([1])), error => error.code === 'Io')
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('a directory create error releases the inhibit', async () => {
    const bridge = createBridge({
        'transfer.createDirectory': () => { throw { code: 'Io', details: 'mkdir failed' } },
    })
    const directory = await platformFor(bridge).startDownloadDirectory('photos', 10)
    await assert.rejects(() => directory.createDirectory('sub'), error => error.code === 'Io')
    await flush()
    assert.deepEqual(releases(bridge), [1])
})

await check('a rejected acquire does not break the transfer and logs at info', async () => {
    infos.length = 0
    const bridge = createBridge({
        'power.inhibitAcquire': () => { throw { code: 'Unsupported', details: 'no bus' } },
    })
    const [upload] = await platformFor(bridge).startUpload({ multiple: false }, ['/src/a.bin'])
    const chunk = await upload.read()
    assert.equal(chunk.length, 0, 'the fake read returns no bytes after the first call')
    await upload.closeAsync()
    await flush()
    assert.deepEqual(releases(bridge), [], 'nothing to release without an id')
    assert.equal(infos.some(args => String(args[0]).includes('Power inhibit unavailable')), true)
})

await check('a download still writes and closes when the acquire is rejected', async () => {
    const bridge = createBridge({
        'power.inhibitAcquire': () => { throw new Error('bus unavailable') },
    })
    const download = await platformFor(bridge).startDownload('a.bin', 0o644, 3)
    await download.write(new Uint8Array([1, 2, 3]))
    assert.equal(download.isComplete(), true)
    await download.closeAsync()
    await flush()
    assert.equal(bridge.calls.filter(call => call.command === 'transfer.close').length, 1)
})

await check('a failed release is logged without failing close', async () => {
    infos.length = 0
    const bridge = createBridge({
        'power.inhibitRelease': () => { throw { code: 'Io', details: 'release failed' } },
    })
    const directory = await platformFor(bridge).startDownloadDirectory('photos', 10)
    directory.close()
    await flush()
    assert.equal(infos.some(args => String(args[0]).includes('Failed to release power inhibit')), true)
})

await check('each transfer holds its own inhibit id', async () => {
    const bridge = createBridge({
        'transfer.openUpload': () => [
            { id: 'upload-1', name: 'a.bin', size: 3 },
            { id: 'upload-2', name: 'b.bin', size: 3 },
        ],
    })
    const platform = platformFor(bridge)
    const [first, second] = await platform.startUpload({ multiple: true }, ['/src/a.bin', '/src/b.bin'])
    assert.equal(acquires(bridge), 2)
    second.cancel()
    await flush()
    assert.deepEqual(releases(bridge), [2])
    first.cancel()
    await flush()
    assert.deepEqual(releases(bridge), [2, 1])
})

if (failures.length) {
    console.error(`${failures.length} power inhibit check(s) failed`)
    process.exit(1)
}
console.log('tauri power inhibit: all checks passed')
