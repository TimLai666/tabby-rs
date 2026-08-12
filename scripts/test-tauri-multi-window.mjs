import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const read = relativePath => fs.readFileSync(path.join(root, relativePath), 'utf8')

const desktop = read('src-tauri/src/commands/desktop.rs')
const app = read('src-tauri/src/commands/app.rs')
const lib = read('src-tauri/src/lib.rs')
const hostApp = read('tabby-tauri/src/services/hostApp.service.ts')
const bridge = read('tabby-tauri/src/api/hostBridge.ts')
const capability = JSON.parse(read('src-tauri/capabilities/default.json'))

assert.match(desktop, /pub fn window_new\([\s\S]*WebviewWindowBuilder::new\(/)
assert.match(desktop, /WebviewUrl::App\("index\.html"\.into\(\)\)/)
assert.match(desktop, /let label = format!\("window-\{\}", state\.next_window_id\(\)\)/)
assert.match(desktop, /on_page_load\(move \|window, payload\|/)
assert.match(desktop, /window\.emit\("app\.launch", context\)/)
assert.doesNotMatch(desktop, /fn main_window\(/)
assert.match(desktop, /pub fn window_get_state\(\s*window: tauri::WebviewWindow/)
assert.match(desktop, /pub fn window_apply_state\(\s*window: tauri::WebviewWindow/)
assert.match(lib, /pub\(crate\) fn register_desktop_window_events\(window: &tauri::WebviewWindow\)/)
assert.match(lib, /let _ = emitter\.emit\("desktop\.windowFocused"/)
assert.match(app, /is_main_window: window\.label\(\) == "main"/)
assert.match(hostApp, /this\.bridge\.invoke\('window\.new', \{\}\)/)
assert.match(hostApp, /context\.secondInstance \|\| context\.request\.newWindow/)
assert.match(hostApp, /this\.bridge\.invoke\('window\.new', \{ launch: context \}\)/)
assert.match(bridge, /'window\.new': \{\s*request: \{ launch\?: LaunchContext \}\s*response: null\s*\}/)
assert.deepEqual(capability.windows, ['main', 'window-*'])

console.log('Tauri multi-window contract passed')
