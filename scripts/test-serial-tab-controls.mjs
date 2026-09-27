import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { execFileSync } from 'node:child_process'
import ts from 'typescript'
import pug from 'pug'

const require = createRequire(import.meta.url)
const root = new URL('../', import.meta.url)
const upstream = file => execFileSync('git', ['show', `14e2d60:${file}`], { cwd: root, encoding: 'utf8' })
const api = upstream('tabby-serial/src/api.ts')
const rates = JSON.parse(api.match(/export const BAUD_RATES = (\[[\s\S]*?\])/)[1].replace(/,\s*]/, ']'))

function fixture (source, className) {
    const events = [], callbacks = [], immediate = [], errors = []
    let selection = 9600
    let rejectSelection = false
    const selector = { show: async (...args) => {
        events.push(['selector', ...args])
        if (rejectSelection) throw new Error('dismissed')
        return selection
    } }
    class Base {
        static template = ''
        static styles = []
        static animations = []
        hotkeys = { hotkey$: {} }
        translate = { instant: x => x }
        notifications = { error: e => errors.push(e) }
        profile = { name: 'Fixture serial', options: {} }
        hasFocus = false
        recentInputs = ''
        subscribeUntilDestroyed (_stream, cb) { callbacks.push(cb) }
        sendInput (data) { events.push(['input', data]) }
        reconnect () { events.push(['reconnect']) }
        setTitle (text) { events.push(['title', text]) }
        ngOnInit () {}
        isSessionExplicitlyTerminated () { return this.baseTerminated ?? false }
    }
    const module = { exports: {} }
    const loader = name => {
        if (name === 'ansi-colors') return {}
        if (name === '@angular/core') return { Component: () => target => target }
        if (name === '@biesbjerg/ngx-translate-extract-marker') return { marker: x => x }
        if (name === 'tabby-core') return { Platform: { Web: 3 }, SelectorService: class {} }
        if (name === 'tabby-terminal') return { BaseTerminalTabComponent: Base, ConnectableTerminalTabComponent: Base }
        if (name === '../api') return { BAUD_RATES: rates }
        if (name === './session') return {}
        if (name === './profile') {
            const profile = { exports: {} }
            vm.runInNewContext(ts.transpileModule(fs.readFileSync(new URL('tabby-tauri/src/serial/profile.ts', root), 'utf8'), {
                compilerOptions: { module: ts.ModuleKind.CommonJS },
            }).outputText, { module: profile, exports: profile.exports })
            return profile.exports
        }
        if (name.endsWith('.pug')) return ''
        return require(name)
    }
    const code = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
        esModuleInterop: true, experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(code, { module, exports: module.exports, require: loader,
        setImmediate: cb => immediate.push(cb), setTimeout, clearTimeout })
    const Tab = module.exports[className]
    const tab = className === 'SerialTabComponent' ? new Tab({}, selector) : new Tab({}, {}, selector)
    return { tab, events, errors, selector, callbacks, immediate,
        dismiss: () => { rejectSelection = true },
    }
}

const currentSource = fs.readFileSync(new URL('tabby-tauri/src/serial/tab.component.ts', root), 'utf8')
const original = fixture(upstream('tabby-serial/src/components/serialTab.component.ts'), 'SerialTabComponent')
const current = fixture(currentSource, 'TauriSerialTabComponent')
for (const f of [original, current]) {
    f.tab.ngOnInit()
    for (const cb of f.immediate) cb()
    for (const key of ['home', 'end', 'restart-serial-session']) f.callbacks[0](key)
    assert.deepEqual(f.events, [['title', 'Fixture serial']], 'unfocused tabs ignore keys')
    f.tab.hasFocus = true
    for (const key of ['home', 'end', 'restart-serial-session', 'unrelated']) f.callbacks[0](key)
}
assert.deepEqual(current.events, original.events)
for (const recentInputs of ['', 'close\r', 'quit\r', 'quit', 'CLOSE\r', 'x\rclose\r']) {
    original.tab.recentInputs = current.tab.recentInputs = recentInputs
    assert.equal(current.tab.isSessionExplicitlyTerminated(), original.tab.isSessionExplicitlyTerminated(), recentInputs)
}
current.tab.baseTerminated = true
assert.equal(current.tab.isSessionExplicitlyTerminated(), true)

const ratesSent = []
current.tab.session = { setBaudRate: async rate => ratesSent.push(rate) }
await current.tab.changeBaudRate()
assert.deepEqual(ratesSent, [9600])
assert.equal(current.tab.profile.options.baudRate, 9600)
const selectorCall = current.events.at(-1)
assert.equal(selectorCall[1], 'Baud rate')
assert.deepEqual(JSON.parse(JSON.stringify(selectorCall[2])), rates.map(x => ({ name: String(x), result: x, weight: x })))
current.tab.session.setBaudRate = async () => { throw { details: 'driver rejected rate' } }
let destroyed = false
current.tab.session.destroy = async () => { destroyed = true }
await current.tab.changeBaudRate()
assert.deepEqual(current.errors, ['driver rejected rate'])
assert.equal(destroyed, true, 'upstream serial errors close the failed session')
current.dismiss()
await current.tab.changeBaudRate()
assert.deepEqual(current.errors, ['driver rejected rate'], 'dismissal is silent')

// Compile both real Pug templates, preserving Angular bindings. Rendering and
// native-device interaction remain separate desktop acceptance requirements.
const actual = pug.render(fs.readFileSync(new URL('tabby-tauri/src/serial/tab.component.pug', root), 'utf8'))
const expected = pug.render(upstream('tabby-serial/src/components/serialTab.component.pug').replaceAll('baudrate', 'baudRate'))
assert.equal(actual, expected, 'toolbar controls, order, classes, bindings, and labels match upstream')
console.log('serial tab: upstream hotkeys, focus, title, explicit close, baud selector, errors, and toolbar template passed')
