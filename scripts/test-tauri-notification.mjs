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

function load (file, globals = {}) {
    const module = { exports: {} }
    const output = ts.transpileModule(read(file), { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(output, { exports: module.exports, module, ...globals })
    return module.exports
}

const settle = async () => {
    for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve))
}

function fakeBridge (invoke) {
    const bridge = { calls: [], invoke: (command, request) => {
        bridge.calls.push(JSON.parse(JSON.stringify({ command, request })))
        return invoke(command, request)
    } }
    return bridge
}

const warnings = []
const recordingConsole = { warn: (...args) => warnings.push(args), log: () => undefined, error: () => undefined }

const adapterFile = 'tabby-tauri/src/services/notification.ts'
const { createTauriNotification, installTauriNotifications } = load(adapterFile, { console: recordingConsole })

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

await check('constructor invokes notification.show synchronously with title and body', async () => {
    const bridge = fakeBridge(async () => null)
    const Notification = createTauriNotification(bridge)
    new Notification('Process completed', { body: 'npm test' })
    assert.deepEqual(bridge.calls, [{ command: 'notification.show', request: { title: 'Process completed', body: 'npm test' } }])
})

await check('constructor sends a null body when the body option is omitted', async () => {
    const bridge = fakeBridge(async () => null)
    const Notification = createTauriNotification(bridge)
    new Notification('Tab activity')
    assert.deepEqual(bridge.calls, [{ command: 'notification.show', request: { title: 'Tab activity', body: null } }])
})

await check('click listeners and close are accepted without throwing or a synthetic click', async () => {
    const bridge = fakeBridge(async () => null)
    const Notification = createTauriNotification(bridge)
    const notification = new Notification('Process completed', { body: 'make' })
    let clicks = 0
    const onClick = () => { clicks++ }
    assert.doesNotThrow(() => {
        notification.addEventListener('click', onClick)
        notification.onclick = onClick
        notification.removeEventListener('click', onClick)
        notification.close()
    })
    await settle()
    assert.equal(clicks, 0)
})

await check('real tabContextMenu call shape works with the installed class', async () => {
    const source = read('tabby-core/src/tabContextMenu.ts')
    assert.match(source, /new Notification\(title, \{ body \}\)/, 'the shared caller must still use this call shape')
    assert.match(source, /notification\.addEventListener\('click', \(\) => this\.app\.selectTab\(tab\)\)/)
    const bridge = fakeBridge(async () => null)
    const host = {}
    installTauriNotifications(bridge, host)
    const title = 'Process completed'
    const body = 'make'
    const tab = { id: 'tab-1' }
    const selected = []
    assert.doesNotThrow(() => {
        const notification = new host.Notification(title, { body })
        notification.addEventListener('click', () => selected.push(tab))
    })
    await settle()
    assert.deepEqual(bridge.calls, [{ command: 'notification.show', request: { title, body } }])
    assert.deepEqual(selected, [])
})

await check('bridge rejection does not throw synchronously and is logged with console.warn', async () => {
    warnings.length = 0
    const bridge = fakeBridge(() => Promise.reject({ code: 'Io', details: 'no notification daemon' }))
    const Notification = createTauriNotification(bridge)
    assert.doesNotThrow(() => new Notification('Process completed', { body: 'make' }))
    await settle()
    assert.equal(warnings.length, 1)
    assert.match(String(warnings[0][0]), /native notification/)
})

await check('a rejected native call reaches onerror and error listeners; success reaches onshow', async () => {
    const failing = createTauriNotification(fakeBridge(() => Promise.reject({ code: 'Io', details: 'no daemon' })))
    const errors = []
    const notification = new failing('Process completed', { body: 'make' })
    notification.onerror = event => errors.push(['onerror', event.type, event.detail?.details])
    notification.addEventListener('error', event => errors.push(['listener', event.type]))
    notification.addEventListener('error', () => { throw new Error('listener failure must not propagate') })
    await settle()
    assert.deepEqual(JSON.parse(JSON.stringify(errors)), [['onerror', 'error', 'no daemon'], ['listener', 'error']])

    const ok = createTauriNotification(fakeBridge(async () => null))
    const shows = []
    const shown = new ok('Tab activity')
    shown.onshow = event => shows.push(event.type)
    await settle()
    assert.deepEqual([...shows], ['show'])
})

await check('synchronous bridge failure does not throw from the constructor', async () => {
    warnings.length = 0
    const bridge = fakeBridge(() => { throw new Error('Tauri global API is unavailable') })
    const Notification = createTauriNotification(bridge)
    assert.doesNotThrow(() => new Notification('Process completed'))
    assert.equal(warnings.length, 1)
})

await check('permission is granted and requestPermission resolves granted', async () => {
    const Notification = createTauriNotification(fakeBridge(async () => null))
    assert.equal(Notification.permission, 'granted')
    assert.equal(await Notification.requestPermission(), 'granted')
})

await check('installTauriNotifications replaces the global Notification with the adapter', async () => {
    const bridge = fakeBridge(async () => null)
    const host = {}
    installTauriNotifications(bridge, host)
    assert.equal(typeof host.Notification, 'function')
    new host.Notification('Tab activity', { body: 'vim' })
    assert.deepEqual(bridge.calls, [{ command: 'notification.show', request: { title: 'Tab activity', body: 'vim' } }])
})

await check('Tauri entry installs the adapter before Angular bootstrap', async () => {
    const entry = read('app/src/entry.tauri.ts')
    assert.match(entry, /import \{ installTauriNotifications \} from '\.\.\/\.\.\/tabby-tauri\/src\/services\/notification'/)
    const install = entry.indexOf('installTauriNotifications(new TauriHostBridge())')
    assert.ok(install > 0, 'entry must install the adapter with the Tauri host bridge')
    assert.ok(install < entry.indexOf('void main()'), 'adapter must be installed before main() bootstraps Angular')
})

if (failures.length) {
    console.error(`${failures.length} notification check(s) failed`)
    process.exitCode = 1
} else {
    console.log('Tauri notification adapter checks passed')
}
