import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

// The Tauri docking service must fall back to 'off' when the native window
// reports no docking support (Wayland), and the settings rows must follow that
// capability instead of always showing the dock options.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const rxjs = require('rxjs')
const read = name => fs.readFileSync(path.join(root, name), 'utf8')

const angular = {
    Inject: () => () => undefined,
    Injectable: () => target => target,
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
            if (name === 'rxjs') return rxjs
            if (name in dependencies) return dependencies[name]
            throw new Error(`Unprovided dependency: ${name}`)
        },
        console,
    })
    return module.exports
}

class DockingService {
    constructor () { this.screensChanged = new rxjs.Subject() }
    get screensChanged$ () { return this.screensChanged }
}

const { TauriDockingService } = load('tabby-tauri/src/services/docking.service.ts', {
    'tabby-core': { BOOTSTRAP_DATA: 'BOOTSTRAP_DATA', DockingService, ConfigService: class {}, PlatformService: class {}, Screen: class {} },
    '../api/hostBridge': {},
})

const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(resolve => setImmediate(resolve))
}

function makeService ({ docking, isMainWindow = true, appearance }) {
    const calls = []
    const infos = []
    const originalInfo = console.info
    console.info = (...args) => infos.push(args)
    const bridge = {
        calls,
        invoke: async (command, request) => {
            calls.push({ command, request })
            if (command === 'window.getState') {
                if (docking === 'fail') throw new Error('fixture')
                return { capabilities: { docking } }
            }
            if (command === 'window.listScreens') return []
            return null
        },
    }
    const config = { store: { appearance } }
    const platform = { displayMetricsChanged$: new rxjs.Subject() }
    const service = new TauriDockingService(bridge, config, platform, { isMainWindow })
    return {
        service,
        calls,
        infos,
        restore: () => { console.info = originalInfo },
        setDockingCalls: () => calls.filter(call => call.command === 'window.setDocking'),
    }
}

const cases = []
async function test (name, body) {
    try {
        await body()
        cases.push({ name, ok: true })
        console.log(`PASS ${name}`)
    } catch (error) {
        cases.push({ name, ok: false })
        console.error(`FAIL ${name}: ${error?.stack ?? error}`)
    }
}

const leftDock = { dock: 'left', dockScreen: null, dockFill: 1, dockSpace: 1, dockAlwaysOnTop: true }

await test('unsupported docking sends off and logs once', async () => {
    const s = makeService({ docking: false, appearance: leftDock })
    try {
        await settle()
        s.service.dock()
        s.service.dock()
        await settle()
        const requests = s.setDockingCalls().map(call => call.request)
        assert.ok(requests.length >= 1)
        for (const request of requests) assert.equal(request.side, 'off')
        assert.equal(s.infos.length, 1)
        assert.equal(s.service.dockingSupported, false)
    } finally {
        s.restore()
    }
})

await test('supported docking keeps the configured side and always-on-top', async () => {
    const s = makeService({ docking: true, appearance: leftDock })
    try {
        await settle()
        s.service.dock()
        await settle()
        const last = s.setDockingCalls().at(-1).request
        assert.equal(last.side, 'left')
        assert.equal(last.alwaysOnTop, true)
        assert.equal(s.infos.length, 0)
        assert.equal(s.service.dockingSupported, true)
    } finally {
        s.restore()
    }
})

await test('secondary windows never dock, even when supported', async () => {
    const s = makeService({ docking: true, isMainWindow: false, appearance: leftDock })
    try {
        await settle()
        s.service.dock()
        await settle()
        assert.equal(s.setDockingCalls().at(-1).request.side, 'off')
    } finally {
        s.restore()
    }
})

await test('dock before the capability is known sends nothing and applies after it resolves', async () => {
    const s = makeService({ docking: true, appearance: leftDock })
    try {
        s.service.dock()
        assert.equal(s.setDockingCalls().length, 0)
        assert.equal(s.service.dockingSupported, null)
        await settle()
        assert.equal(s.setDockingCalls().at(-1).request.side, 'left')
    } finally {
        s.restore()
    }
})

await test('a failed capability read treats docking as unsupported', async () => {
    const s = makeService({ docking: 'fail', appearance: leftDock })
    try {
        await settle()
        assert.equal(s.service.dockingSupported, false)
        assert.equal(s.setDockingCalls().at(-1).request.side, 'off')
    } finally {
        s.restore()
    }
})

await test('settings rows are gated by the docking capability, not by service presence', () => {
    const pug = read('tabby-settings/src/components/windowSettingsTab.component.pug')
    const guards = [...pug.matchAll(/\*ngIf='([^']*docking[^']*)'/g)].map(match => match[1])
    assert.ok(guards.length >= 6, 'expected the docking rows in the template')
    for (const guard of guards) {
        assert.ok(guard.startsWith('dockingAvailable'), 'docking row must use dockingAvailable: ' + guard)
    }
    assert.match(read('tabby-settings/src/components/windowSettingsTab.component.ts'), /get dockingAvailable \(\): boolean/)
})

const failures = cases.filter(result => !result.ok).length
console.log(failures ? `Tauri docking: ${failures} failures` : `Tauri docking: ${cases.length}/${cases.length} cases passed`)
if (failures) process.exitCode = 1
