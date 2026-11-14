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
    Injectable: () => target => target,
}

// Loads a source file as CommonJS with the given module dependencies.
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
            if (name in dependencies) return dependencies[name]
            throw new Error(`Unprovided dependency: ${name}`)
        },
    })
    return module.exports
}

// The abstract base class is the real tabby-core source, so default update() behavior is exercised.
const core = load('tabby-core/src/services/updater.service.ts')
const { TauriUpdaterService } = load('tabby-tauri/src/services/updater.service.ts', {
    'tabby-core': core,
    '../api/hostBridge': {},
})

const SIGNED_INFO = {
    version: '1.0.240-tabbyrs.2',
    currentVersion: '1.0.231-tabbyrs.1',
    channel: 'stable',
    publishedAt: '2026-11-01T00:00:00Z',
    notes: 'signed notes',
    downloadSize: 1024,
    requiresConfigMigration: false,
}

const RELEASE_PAGE = {
    version: '1.0.235',
    releaseUrl: 'https://github.com/eugeny/tabby/releases/tag/v1.0.235',
    releaseNotes: 'release page notes',
    releaseDate: '2026-11-10T12:00:00Z',
    isNewer: true,
}

function harness ({ checkResult, releasePageResult = RELEASE_PAGE, channel = 'stable' } = {}) {
    const bridge = {
        calls: [],
        listen: async () => () => undefined,
        invoke: async (command, request) => {
            bridge.calls.push({ command, request })
            if (command === 'update.check') {
                if (checkResult instanceof Error) throw checkResult
                return checkResult ?? null
            }
            if (command === 'update.checkReleasePage') {
                if (releasePageResult instanceof Error) throw releasePageResult
                return releasePageResult
            }
            if (command === 'update.getChannel') return channel
            return null
        },
    }
    const platform = {
        opened: [],
        messageBoxes: 0,
        getAppVersion: () => '1.0.231-tabbyrs.1',
        openExternal: async url => { platform.opened.push(url) },
        showMessageBox: async () => { platform.messageBoxes++; return { response: 0 } },
    }
    const translate = { instant: text => text }
    const service = new TauriUpdaterService(bridge, platform, translate)
    return { bridge, platform, service, commands: () => bridge.calls.map(call => call.command) }
}

const unsupported = () => Object.assign(new Error('unsupported'), { code: 'unsupported', details: 'updater endpoint is not configured' })

await check('an unsupported fallback keeps the signed updater\'s original error', async () => {
    // A configured signed updater can also answer `unsupported` (e.g. "updater is
    // unavailable"); the fallback is then refused and must not hide that reason.
    const original = Object.assign(new Error('unsupported'), { code: 'unsupported', details: 'updater is unavailable' })
    const refused = Object.assign(new Error('unsupported'), { code: 'unsupported', details: 'release page check is only used when the signed updater is not configured' })
    const { service } = harness({ checkResult: original, releasePageResult: refused })
    await assert.rejects(service.check(), error => error.details === 'updater is unavailable')
})

await check('a non-unsupported fallback failure propagates as itself', async () => {
    const network = Object.assign(new Error('io'), { code: 'io', details: 'connection refused' })
    const { service } = harness({ checkResult: unsupported(), releasePageResult: network })
    await assert.rejects(service.check(), error => error.details === 'connection refused')
})

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

await check('signed pipeline results pass through without the release page fallback', async () => {
    const { service, commands } = harness({ checkResult: SIGNED_INFO })
    const info = await service.check()
    assert.deepEqual(JSON.parse(JSON.stringify(info)), { ...SIGNED_INFO, downloadSize: 1024 })
    assert.deepEqual(commands(), ['update.check'])
})

await check('signed pipeline returning null stays null without the fallback', async () => {
    const { service, commands } = harness({ checkResult: null })
    assert.equal(await service.check(), null)
    assert.deepEqual(commands(), ['update.check'])
})

await check('Unsupported from update.check falls back to the release page check', async () => {
    const { service, bridge, commands } = harness({ checkResult: unsupported() })
    const info = await service.check()
    assert.deepEqual(commands(), ['update.check', 'update.checkReleasePage', 'update.getChannel'])
    assert.deepEqual(JSON.parse(JSON.stringify(bridge.calls[1])), { command: 'update.checkReleasePage', request: {} })
    assert.deepEqual(JSON.parse(JSON.stringify(info)), {
        version: '1.0.235',
        currentVersion: '1.0.231-tabbyrs.1',
        channel: 'stable',
        publishedAt: '2026-11-10T12:00:00Z',
        notes: 'release page notes',
        requiresConfigMigration: false,
    })
})

await check('release page info carries the nightly channel from the host', async () => {
    const { service } = harness({ checkResult: unsupported(), channel: 'nightly' })
    assert.equal((await service.check()).channel, 'nightly')
})

await check('release page check returns null when the release is not newer', async () => {
    const { service, commands } = harness({
        checkResult: unsupported(),
        releasePageResult: { ...RELEASE_PAGE, isNewer: false },
    })
    assert.equal(await service.check(), null)
    assert.deepEqual(commands(), ['update.check', 'update.checkReleasePage'])
})

await check('non-Unsupported update.check errors propagate without the fallback', async () => {
    const { service, commands } = harness({ checkResult: Object.assign(new Error('x'), { code: 'io', details: 'update check failed' }) })
    await assert.rejects(service.check(), { code: 'io' })
    assert.deepEqual(commands(), ['update.check'])
})

await check('release page check errors propagate to the caller', async () => {
    const { service } = harness({
        checkResult: unsupported(),
        releasePageResult: Object.assign(new Error('x'), { code: 'io', details: 'release check failed' }),
    })
    await assert.rejects(service.check(), { code: 'io' })
})

await check('update() on release page info opens the release URL without download or install', async () => {
    const { service, platform, commands } = harness({ checkResult: unsupported() })
    const info = await service.check()
    await service.update(info)
    assert.deepEqual(platform.opened, [RELEASE_PAGE.releaseUrl])
    assert.equal(platform.messageBoxes, 0)
    assert.ok(!commands().includes('update.download'))
    assert.ok(!commands().includes('update.install'))
})

await check('install() and download() on release page info never invoke the signed pipeline', async () => {
    const { service, platform, commands } = harness({ checkResult: unsupported() })
    const info = await service.check()
    await service.download(info)
    await service.install(info)
    assert.deepEqual(platform.opened, [RELEASE_PAGE.releaseUrl])
    assert.ok(!commands().includes('update.download'))
    assert.ok(!commands().includes('update.install'))
})

await check('cancel() is a no-op for release page info', async () => {
    const { service, commands } = harness({ checkResult: unsupported() })
    await service.check()
    await service.cancel()
    assert.ok(!commands().includes('update.cancel'))
})

await check('signed info keeps the confirmation, download and install flow after a release page check', async () => {
    const { service, platform, commands } = harness({ checkResult: unsupported() })
    await service.check()
    await service.update(SIGNED_INFO)
    assert.deepEqual(platform.opened, [])
    assert.equal(platform.messageBoxes, 1)
    assert.deepEqual(commands().slice(-2), ['update.download', 'update.install'])
})

if (failures.length) {
    console.error(`${failures.length} updater fallback check(s) failed`)
    process.exitCode = 1
} else {
    console.log('Tauri updater release page fallback checks passed')
}
