import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const root = process.cwd()
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const rxjs = require('rxjs')

function loadClass (file, dependencies) {
    const module = { exports: {} }
    const source = fs.readFileSync(path.join(root, file), 'utf8')
    const output = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(output, {
        exports: module.exports,
        module,
        console,
        require: name => {
            if (name === '@angular/core') return { Injectable: () => value => value }
            if (name === 'rxjs') return rxjs
            if (name in dependencies) return dependencies[name]
            throw new Error('Unprovided dependency: ' + name)
        },
    })
    return module.exports
}

const { TauriDesktopIntegrationService } = loadClass('tabby-tauri/src/services/desktopIntegration.service.ts', {
    'tabby-core': { AppService: class {}, ConfigService: class {}, DockingService: class {},
        aggregateTabProgress: () => ({ state: 'none', value: null }), TabProgressState: class {} },
    '../api/hostBridge': {},
    './hostWindow.service': { TauriHostWindowService: class {} },
})

const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(resolve => setImmediate(resolve))
}

async function makeService (appearance, hideCheck) {
    const listeners = new Map()
    const calls = []
    const bridge = {
        listen: async (event, callback) => { listeners.set(event, callback); return () => listeners.delete(event) },
        invoke: async (command, request) => {
            calls.push({ command, request })
            if (command === 'window.getState') return { capabilities: { opacity: false, vibrancy: false } }
            if (command === 'window.canHideOnBlur') return hideCheck()
            return null
        },
    }
    const config = {
        ready$: { toPromise: async () => undefined },
        changed$: new rxjs.Subject(),
        store: { hotkeys: {}, appearance },
    }
    const app = { tabs: [], activeTab: null, tabOpened$: new rxjs.Subject(), tabRemoved$: new rxjs.Subject(),
        activeTabChange$: new rxjs.Subject() }
    const service = new TauriDesktopIntegrationService(app, bridge, config,
        { dock: () => undefined }, { windowShown$: new rxjs.Subject(), setProgressBar: () => undefined })
    await service.initialize()
    return {
        calls,
        blur: async () => { listeners.get('desktop:windowFocused')(false); await settle() },
        focus: async () => { listeners.get('desktop:windowFocused')(true); await settle() },
        hides: () => calls.filter(c => c.command === 'window.applyState' && c.request.visible === false).length,
        checks: () => calls.filter(c => c.command === 'window.canHideOnBlur').length,
    }
}

const cases = []
async function test (name, body) {
    try { await body(); cases.push({ name, ok: true }) }
    catch (error) { cases.push({ name, ok: false, error }) }
}
const docked = { dock: 'left', dockHideOnBlur: true }

await test('blur with another Tabby window focused does not hide', async () => {
    const s = await makeService(docked, () => ({ anyWindowFocused: true, enabled: true }))
    await s.blur()
    assert.equal(s.checks(), 1)
    assert.equal(s.hides(), 0)
})
await test('blur with no focused window and enabled hides', async () => {
    const s = await makeService(docked, () => ({ anyWindowFocused: false, enabled: true }))
    await s.blur()
    assert.equal(s.hides(), 1)
})
await test('blur while a native dialog disables the window does not hide', async () => {
    const s = await makeService(docked, () => ({ anyWindowFocused: false, enabled: false }))
    await s.blur()
    assert.equal(s.hides(), 0)
})
await test('focus never hides or checks', async () => {
    const s = await makeService(docked, () => ({ anyWindowFocused: false, enabled: true }))
    await s.focus()
    assert.equal(s.checks(), 0)
    assert.equal(s.hides(), 0)
})
await test('dock off or hide-on-blur off never calls the check', async () => {
    for (const appearance of [{ dock: 'off', dockHideOnBlur: true }, { dock: 'left', dockHideOnBlur: false }]) {
        const s = await makeService(appearance, () => ({ anyWindowFocused: false, enabled: true }))
        await s.blur()
        assert.equal(s.checks(), 0)
        assert.equal(s.hides(), 0)
    }
})
await test('failed check does not hide and is consumed', async () => {
    const s = await makeService(docked, () => { throw new Error('fixture') })
    await s.blur()
    assert.equal(s.hides(), 0)
})

for (const result of cases) {
    if (result.ok) console.log(`PASS ${result.name}`)
    else console.error(`FAIL ${result.name}: ${result.error?.stack ?? result.error}`)
}
const failures = cases.filter(result => !result.ok).length
console.log(`Tauri desktop integration: ${cases.length - failures}/${cases.length} cases passed`)
if (failures) process.exitCode = 1
