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
const scopedEvents = [...windowEvents, 'desktop:windowCaptionControls', 'app:launch']
const bridgeCode = ts.transpileModule(read('tabby-tauri/src/services/tauriHostBridge.service.ts'), {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
}).outputText
const listeners = []
const invocations = []
const captionResponses = new Map([
    ['main', { revision: 3, visible: true, scale: 1, layout: { clip: { x: 600, y: 0, width: 137, height: 32 }, buttons: [] },
        pointer: { hovered: null, pressed: null }, maximized: false, active: true }],
    ['window-1', null],
])
function makeBridge (label) {
    const subscribe = target => async (event, handler) => {
        const entry = { target, event, handler }
        listeners.push(entry)
        return () => listeners.splice(listeners.indexOf(entry), 1)
    }
    const exports = {}
    vm.runInNewContext(bridgeCode, { exports, window: { __TAURI__: {
        core: { invoke: async (command, args) => {
            invocations.push({ label, command, args: JSON.parse(JSON.stringify(args)) })
            const response = captionResponses.get(label)
            if (response instanceof Error) throw response
            return response
        } },
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
for (const [i, label] of ['main', 'window-1'].entries()) {
    assert.equal(await windows[i].invoke('window.getCaptionState', {}), captionResponses.get(label))
    assert.deepEqual(invocations.at(-1), { label, command: 'window_get_caption_state', args: { request: {} } },
        'Caption queries use the injected current window, with no caller-supplied label')
}
const captionError = new Error('caption window was destroyed')
captionResponses.set('window-1', captionError)
await assert.rejects(windows[1].invoke('window.getCaptionState', {}), error => error === captionError,
    'Native query errors must not turn into an unprepared null state')
for (const event of scopedEvents) {
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
for (const event of ['desktop:hotkey', 'serial:portsChanged', 'update:state']) {
    const stop = await windows[0].listen(event, () => {})
    assert.equal(listeners[0].target, null, `${event} retains application-wide delivery`)
    stop()
}

assert.match(desktop, /pub async fn window_new\([\s\S]*WebviewWindowBuilder::new\(/,
    'Window creation must run outside the synchronous Windows command handler')
const present = lib.match(/fn present_and_dispatch\([\s\S]*?\r?\n\}/)?.[0]
assert.match(present, /Sender<LaunchContext>>\(\)\s*\.send\(context\)/)
assert.doesNotMatch(present, /spawn_blocking|create_window/,
    'Callbacks only enqueue; independently spawned dispatch tasks can reorder launches')
assert.match(lib, /while let Ok\(context\) = launch_receiver\.recv\(\) \{\s*dispatch_launch\(&launch_app, context\);/)
assert.ok(lib.indexOf('.manage(launch_sender)') < lib.indexOf('.plugin('), 'Early callbacks must have a registered input channel')
assert.ok(lib.indexOf('app.manage(AppState::new(') < lib.indexOf('while let Ok(context) = launch_receiver.recv()'),
    'The initial main request must exist before the receiver can dispatch later requests')
assert.match(lib, /app\.run_on_main_thread\(move \|\| \{[\s\S]*?state\.launches\(\)\.push\(window\.label\(\), request\.clone\(\)\)/,
    'Target selection and delivery must share the event thread with window destruction')
assert.match(desktop, /WebviewUrl::App\("index\.html"\.into\(\)\)/)
assert.match(desktop, /let label = format!\("window-\{\}", state\.next_window_id\(\)\)/)
assert.doesNotMatch(desktop, /on_page_load\(/, 'Launches must remain queued until their renderer can receive them')
assert.match(desktop, /state\.launches\(\)\.push\(&label, context\.for_new_window\(\)\)/)
assert.match(read('src-tauri/src/commands/launch.rs'), /state\.launches\(\)\.take\(window\.label\(\)\)/,
    'The invoking window must only consume its own launch requests')
assert.match(lib, /state\.launches\(\)\.push\(window\.label\(\), request\.clone\(\)\)/)
assert.match(lib, /window\.emit_to\(window\.label\(\), "app:launch", \(\)\)/)
assert.doesNotMatch(lib, /app\.emit\("app:launch"/)
assert.doesNotMatch(desktop, /fn main_window\(/)
assert.match(desktop, /pub fn window_get_state\(\s*window: tauri::WebviewWindow/)
const captionQuery = desktop.match(/pub async fn window_get_caption_state\([\s\S]*?\r?\n\}/)?.[0]
assert.ok(captionQuery, 'A caption query must be available without enabling frame controls')
assert.match(captionQuery, /window: tauri::WebviewWindow/)
assert.match(captionQuery, /caption_state\(&window\)/)
assert.doesNotMatch(captionQuery, /prepare_caption_controls|set_caption_frame/,
    'Reading state must not prepare controls or change the frame')
assert.match(lib, /\.invoke_handler\(tauri::generate_handler!\[[\s\S]*?\bwindow_get_caption_state,/)
assert.match(bridge, /'window\.getCaptionState': \{\s*request: Record<string, never>\s*response: WindowsCaptionSnapshot \| null\s*\}/)
assert.match(bridge, /'desktop:windowCaptionControls': WindowsCaptionSnapshot/)
assert.match(desktop, /pub fn window_apply_state\(\s*window: tauri::WebviewWindow/)
assert.match(lib, /pub\(crate\) fn register_desktop_window_events\(window: &tauri::WebviewWindow\)/)
for (const event of windowEvents) {
    assert.match(lib, new RegExp(`emitter\\.emit_to\\(\\s*emitter\\.label\\(\\),\\s*"${event}"`))
}
assert.match(lib, /tauri::WindowEvent::CloseRequested \{ api, \.\. \} => \{\s*api\.prevent_close\(\);[\s\S]*?emitter\.emit_to\(emitter\.label\(\), "desktop:windowCloseRequested", \(\)\)/,
    'Native close must be prevented before requesting confirmation in only the owning window')
const closeCommand = desktop.match(/pub fn window_close\([\s\S]*?\r?\n\}/)?.[0]
assert.ok(closeCommand)
assert.match(closeCommand, /set_closing\(window\.label\(\), true\)[\s\S]*window\.destroy\(\)\.map_err\(/,
    'An approved close must finish without requesting confirmation again')
assert.match(closeCommand, /set_closing\(window\.label\(\), false\)/, 'A failed destruction restores launch eligibility')
assert.match(lib, /windows\.retain\(\|window\| !state\.launches\(\)\.is_closing\(window\.label\(\)\)\)/,
    'Windows waiting for the native Destroyed event must not receive new launches')
assert.match(app, /is_main_window: window\.label\(\) == "main"/)
assert.match(hostApp, /this\.bridge\.invoke\('window\.new', \{\}\)/)
assert.match(hostApp, /if \(context\.request\.newWindow\)/)
assert.match(hostApp, /this\.bridge\.invoke\('window\.new', \{ launch: context \}\)/)
assert.match(bridge, /'window\.new': \{\s*request: \{ launch\?: LaunchContext \}\s*response: null\s*\}/)
assert.deepEqual(capability.windows, ['main', 'window-*'])

console.log('Tauri multi-window contract passed')
