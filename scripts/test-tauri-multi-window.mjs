import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import ts from 'typescript'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const read = relativePath => fs.readFileSync(path.join(root, relativePath), 'utf8')

const desktop = read('src-tauri/src/commands/desktop.rs')
const app = read('src-tauri/src/commands/app.rs')
const lib = read('src-tauri/src/lib.rs')
const hostApp = read('tabby-tauri/src/services/hostApp.service.ts')
const bridge = read('tabby-tauri/src/api/hostBridge.ts')
const capability = JSON.parse(read('src-tauri/capabilities/default.json'))
const windowEvents = ['desktop:windowFocused', 'desktop:windowMoved', 'desktop:windowResized',
    'desktop:windowCloseRequested', 'desktop:fileDrop', 'desktop:themeChanged', 'desktop:displayMetricsChanged']
const bridgeCode = ts.transpileModule(read('tabby-tauri/src/services/tauriHostBridge.service.ts'), {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
}).outputText
const listeners = []
function makeBridge (label) {
    const subscribe = target => async (event, handler) => {
        const entry = { target, event, handler }
        listeners.push(entry)
        return () => listeners.splice(listeners.indexOf(entry), 1)
    }
    const exports = {}
    vm.runInNewContext(bridgeCode, { exports, window: { __TAURI__: {
        event: { listen: subscribe(null) },
        webviewWindow: { getCurrentWebviewWindow: () => ({ listen: subscribe(label) }) },
    } }, require: name => {
        if (name === '@angular/core') return { Injectable: () => target => target }
        if (name === '../api/hostBridge') return { HostBridge: class {} }
        throw new Error(`Unexpected import: ${name}`)
    } })
    return new exports.TauriHostBridge()
}
const windows = [makeBridge('main'), makeBridge('window-1')]
for (const event of windowEvents) {
    const received = [[], []]
    const unsubscribe = await Promise.all(windows.map((value, i) => value.listen(event, data => received[i].push(data))))
    assert.deepEqual(listeners.map(value => value.target), ['main', 'window-1'], `${event} must not register an Any listener`)
    // Tauri includes Any listeners even when emitting to a specific label.
    for (const entry of listeners.filter(value => value.target === null || value.target === 'main')) {
        entry.handler({ payload: 'only-main' })
    }
    assert.deepEqual(received, [['only-main'], []])
    unsubscribe.forEach(stop => stop())
    assert.equal(listeners.length, 0, 'scoped listeners retain native unsubscription')
}
for (const event of ['desktop:hotkey', 'serial:portsChanged', 'update:state', 'app:launch']) {
    const stop = await windows[0].listen(event, () => {})
    assert.equal(listeners[0].target, null, `${event} retains application-wide delivery`)
    stop()
}

assert.match(desktop, /pub fn window_new\([\s\S]*WebviewWindowBuilder::new\(/)
assert.match(desktop, /WebviewUrl::App\("index\.html"\.into\(\)\)/)
assert.match(desktop, /let label = format!\("window-\{\}", state\.next_window_id\(\)\)/)
assert.match(desktop, /on_page_load\(move \|window, payload\|/)
assert.match(desktop, /window\.emit\("app:launch", context\)/)
assert.doesNotMatch(desktop, /fn main_window\(/)
assert.match(desktop, /pub fn window_get_state\(\s*window: tauri::WebviewWindow/)
assert.match(desktop, /pub fn window_apply_state\(\s*window: tauri::WebviewWindow/)
assert.match(lib, /pub\(crate\) fn register_desktop_window_events\(window: &tauri::WebviewWindow\)/)
for (const event of windowEvents) {
    assert.match(lib, new RegExp(`emitter\\.emit_to\\(\\s*emitter\\.label\\(\\),\\s*"${event}"`))
}
assert.match(lib, /tauri::WindowEvent::CloseRequested \{ api, \.\. \} => \{\s*api\.prevent_close\(\);[\s\S]*?emitter\.emit_to\(emitter\.label\(\), "desktop:windowCloseRequested", \(\)\)/,
    'Native close must be prevented before requesting confirmation in only the owning window')
const closeCommand = desktop.match(/pub fn window_close\([\s\S]*?\r?\n\}/)?.[0]
assert.ok(closeCommand)
assert.match(closeCommand, /window\.destroy\(\)\.map_err\(io_error\)/,
    'An approved close must finish without requesting confirmation again')
assert.match(app, /is_main_window: window\.label\(\) == "main"/)
assert.match(hostApp, /this\.bridge\.invoke\('window\.new', \{\}\)/)
assert.match(hostApp, /context\.secondInstance \|\| context\.request\.newWindow/)
assert.match(hostApp, /this\.bridge\.invoke\('window\.new', \{ launch: context \}\)/)
assert.match(bridge, /'window\.new': \{\s*request: \{ launch\?: LaunchContext \}\s*response: null\s*\}/)
assert.deepEqual(capability.windows, ['main', 'window-*'])

console.log('Tauri multi-window contract passed')
