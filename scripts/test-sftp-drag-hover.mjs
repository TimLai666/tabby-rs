import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(root + '/package.json'), ts = require('typescript'), rx = require('rxjs')
const read = name => fs.readFileSync(path.join(root, name), 'utf8')
const failures = [], checks = []
async function check (name, run) {
    try { await run(); checks.push(name); console.log(`PASS ${name}`) }
    catch (error) { failures.push(name); console.error(`FAIL ${name}: ${error?.stack ?? error}`) }
}

const angular = { Injectable: () => x => x, Directive: () => x => x, Inject: () => () => {}, Self: () => () => {}, Component: () => x => x, Input: () => () => {}, Output: () => () => {}, Optional: () => () => {}, EventEmitter: class extends rx.Subject { emit (value) { this.next(value) } } }
function load (text, dependencies = {}, globals = {}) {
    const module = { exports: {} }
    vm.runInNewContext(ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText,
        { exports: module.exports, module, require: name => { if (name === 'rxjs') return rx; if (name === '@angular/core') return angular; if (name in dependencies) return dependencies[name]; throw new Error(`Unprovided ${name}`) }, console, Error, Uint8Array, ...globals })
    return module.exports
}
function extractFunction (source, signature) {
    const match = signature.exec(source)
    assert.ok(match, `function signature not found: ${signature}`)
    const start = match.index + match[0].length - 1
    let depth = 0, end = start
    for (; end < source.length; end++) { if (source[end] === '{') depth++; else if (source[end] === '}' && --depth === 0) break }
    return source.slice(match.index, end + 1)
}

await check('production native window callback sends ordered hover and one unchanged drop to its own window', async () => {
    const body = extractFunction(read('src-tauri/src/lib.rs'), /pub\(crate\)\s+fn\s+register_desktop_window_events\s*\(window:\s*&tauri::WebviewWindow\)\s*\{/)
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'sftp-drag-native-'))
    try {
        const rust = String.raw`
use std::{path::PathBuf, sync::{Arc, Mutex}};
#[derive(Clone, Debug, PartialEq)] pub enum Value { Null, Bool(bool), Number(f64), Text(String), FileDrop(Vec<String>, f64, f64) }
trait IntoValue { fn into_value(self) -> Value; }
impl IntoValue for Value { fn into_value(self)->Value {self} }
impl IntoValue for bool { fn into_value(self)->Value {Value::Bool(self)} }
impl IntoValue for f64 { fn into_value(self)->Value {Value::Number(self)} }
impl IntoValue for &str { fn into_value(self)->Value {Value::Text(self.into())} }
impl IntoValue for () { fn into_value(self)->Value {Value::Null} }
#[macro_export] macro_rules! json { (null) => { Value::Null }; ({ "paths": $paths:expr, "x": $x:expr, "y": $y:expr $(,)? }) => { Value::FileDrop($paths, $x as f64, $y as f64) }; ({ "x": $x:expr, "y": $y:expr $(,)? }) => { Value::Text(format!("point:{:?}:{:?}",$x,$y)) }; ({ "width": $w:expr, "height": $h:expr $(,)? }) => { Value::Text(format!("size:{:?}:{:?}",$w,$h)) }; }
mod serde_json { pub use crate::json; pub use crate::Value; }
#[derive(Clone)] struct AppState;
struct Launches; impl Launches { fn remove(&self,_:&str){} }
impl AppState { fn launches(&self)->Launches {Launches} }
mod tauri {
 use super::{Arc,Mutex,Value,IntoValue,AppState}; use std::path::PathBuf;
 #[derive(Clone)] pub struct WebviewWindow { label:String, events:Arc<Mutex<Vec<(String,String,Value)>>> }
 impl WebviewWindow { pub fn new(label:&str, events:Arc<Mutex<Vec<(String,String,Value)>>>) -> Self {Self{label:label.into(),events}} pub fn label(&self)->&str{&self.label} pub fn emit_to<T:IntoValue>(&self,label:&str,event:&str,payload:T)->Result<(),()> { self.events.lock().unwrap().push((label.into(),event.into(),payload.into_value())); Ok(()) } pub fn on_window_event<F:Fn(&WindowEvent)+'static>(&self,f:F){ f(&WindowEvent::DragDrop(DragDropEvent::Enter{paths:vec![PathBuf::from("/tmp/a.bin")],position:Position{x:4.5,y:9.25}})); f(&WindowEvent::DragDrop(DragDropEvent::Over{position:Position{x:7.0,y:11.5}})); f(&WindowEvent::DragDrop(DragDropEvent::Leave)); f(&WindowEvent::DragDrop(DragDropEvent::Drop{paths:vec![PathBuf::from("/tmp/a.bin")],position:Position{x:10.0,y:20.0}})); } pub fn state<T>(&self)->AppState {AppState} }
 #[derive(Clone)] pub struct Position{pub x:f64,pub y:f64} pub struct Size{pub width:u32,pub height:u32}
 pub enum DragDropEvent{Enter{paths:Vec<PathBuf>,position:Position},Over{position:Position},Leave,Drop{paths:Vec<PathBuf>,position:Position}}
 pub enum Theme{Dark,Light,Other} pub enum WindowEvent{Focused(bool),Moved(Position),Resized(Size),CloseRequested{api:CloseApi},Destroyed,DragDrop(DragDropEvent),ThemeChanged(Theme),ScaleFactorChanged{scale_factor:f64}}
 pub struct CloseApi; impl CloseApi{pub fn prevent_close(&self){}}
}
${body}
#[cfg(test)] mod tests { use super::*; #[test] fn drag_payloads_are_owned_and_drop_is_emitted_once(){ let calls=Arc::new(Mutex::new(Vec::new())); let window=tauri::WebviewWindow::new("sftp-window",calls.clone()); register_desktop_window_events(&window); let got=calls.lock().unwrap().clone(); assert_eq!(got.iter().map(|e|e.1.as_str()).collect::<Vec<_>>(),vec!["desktop:fileDrag","desktop:fileDrag","desktop:fileDrag","desktop:fileDrag","desktop:fileDrop"]); assert!(got.iter().all(|e|e.0=="sftp-window")); assert_eq!(got[0].2,Value::Text("point:4.5:9.25".into())); assert_eq!(got[1].2,Value::Text("point:7.0:11.5".into())); assert_eq!(got[2].2,Value::Null); assert_eq!(got[3].2,Value::Null); assert_eq!(got[4].2,Value::FileDrop(vec!["/tmp/a.bin".into()],10.0,20.0)); } }
`
        const file = path.join(tmp, 'harness.rs'), binary = path.join(tmp, 'harness')
        fs.writeFileSync(file, rust)
        const built = spawnSync(process.env.RUSTC || 'rustc', ['--test', file, '-o', binary], { encoding: 'utf8', timeout: 30000 })
        assert.equal(built.status, 0, `extracted production callback must compile against framework boundary fakes:\n${built.stderr}`)
        const result = spawnSync(binary, [], { encoding: 'utf8', timeout: 30000 })
        assert.equal(result.status, 0, result.stdout + result.stderr)
        assert.match(result.stdout, /1 passed/)
    } finally { fs.rmSync(tmp, { recursive: true, force: true }) }
})

const bridgeWindow = {}
const bridgeModule = load(read('tabby-tauri/src/services/tauriHostBridge.service.ts'), { '../api/hostBridge': { HostBridge: class {} } }, { window: bridgeWindow })
await check('actual host bridge scopes drag hover to the current webview and unsubscribes only it', async () => {
    const routed = [], globalEvents = [], localEvents = []
    const source = (events, label) => ({ listen: async (name, handler) => { events.push({ name, handler, label }); return () => { events.splice(events.findIndex(x => x.name === name), 1) } } })
    const global = source(globalEvents, 'global'), local = source(localEvents, 'window')
    const api = { core: { invoke: async () => null }, event: global, webviewWindow: { getCurrentWebviewWindow: () => local } }
    bridgeWindow.__TAURI__ = api
    const host = new bridgeModule.TauriHostBridge()
    const hover = host.listen('desktop:fileDrag', payload => routed.push(payload)), globalEvent = host.listen('ssh:connecting', payload => routed.push(payload))
    await Promise.all([hover, globalEvent])
    assert.deepEqual(localEvents.map(x => x.name), ['desktop:fileDrag'])
    assert.deepEqual(globalEvents.map(x => x.name), ['ssh:connecting'])
    localEvents[0].handler({ payload: { x: 12, y: 34 } }); globalEvents[0].handler({ payload: { connectionId: 'ssh-1' } })
    assert.deepEqual(routed, [{ x: 12, y: 34 }, { connectionId: 'ssh-1' }])
    ;(await hover)(); assert.equal(localEvents.length, 0); assert.equal(globalEvents.length, 1)
})

const platformSource = read('tabby-tauri/src/services/platform.service.ts')
const coreModule = load(read('tabby-core/src/api/platform.ts'))
const platformModule = load(platformSource, { 'tabby-core': coreModule, '../api/hostBridge': {} }, { window: { devicePixelRatio: 2, matchMedia: () => ({ matches: false }) } })
await check('platform waits for hover and drop listeners, delivers both, and combines cleanup', async () => {
    const registrations = [], stopped = []
    const tracked = new Set(['desktop:fileDrop', 'desktop:fileDrag'])
    const bridge = { listen: (name, handler) => tracked.has(name) ? new Promise(resolve => registrations.push({ name, handler, resolve })) : Promise.resolve(() => {}), invoke: async () => '' }
    const instance = new platformModule.TauriPlatformService(bridge, { platform: 'windows' }, {})
    assert.ok(registrations.some(x => x.name === 'desktop:fileDrop'), 'drop listener registration must start')
    let ready = false; const readiness = instance.fileDropReady.then(unlisten => { ready = true; return unlisten })
    const first = registrations[0]
    first.resolve(() => stopped.push(first.name))
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(ready, false, 'readiness must wait for the second file listener')
    assert.equal(registrations.length, 2, 'both file listeners must register, with either registration timing')
    assert.notEqual(registrations[0].name, registrations[1].name)
    const second = registrations[1]
    second.resolve(() => stopped.push(second.name))
    const unlisten = await readiness; assert.equal(ready, true)
    const drops = [], drags = []; instance.fileDropped$.subscribe(x => drops.push(x)); instance.fileDragged$.subscribe(x => drags.push(x))
    registrations.find(x => x.name === 'desktop:fileDrop').handler({ paths: ['a'], x: 4, y: 8 })
    registrations.find(x => x.name === 'desktop:fileDrag').handler({ x: 4, y: 8 })
    assert.deepEqual(drops, [{ paths: ['a'], x: 4, y: 8 }]); assert.deepEqual(drags, [{ x: 4, y: 8 }])
    const position = instance.getFileDropPosition({ x: 100, y: 80 })
    assert.deepEqual([position.x, position.y], [50, 40])
    unlisten(); assert.deepEqual(stopped.sort(), ['desktop:fileDrag', 'desktop:fileDrop'])
})
await check('second listener registration failure releases the first registration', async () => {
    const calls = [], release = []
    const tracked = new Set(['desktop:fileDrop', 'desktop:fileDrag'])
    const bridge = { listen: name => {
        if (!tracked.has(name)) { return Promise.resolve(() => {}) }
        calls.push(name)
        return name === 'desktop:fileDrop' ? Promise.resolve(() => release.push(name)) : Promise.reject(new Error('drag listener denied'))
    }, invoke: async () => '' }
    const instance = new platformModule.TauriPlatformService(bridge, { platform: 'windows' }, {})
    await assert.rejects(instance.fileDropReady, /drag listener denied/)
    assert.deepEqual(calls.slice(0, 2), ['desktop:fileDrop', 'desktop:fileDrag']); assert.deepEqual(release, ['desktop:fileDrop'])
})

class FakeClassList {
    values = new Set()
    add (name) { this.values.add(name) }
    remove (name) { this.values.delete(name) }
    contains (name) { return this.values.has(name) }
}
class FakeElement {
    constructor (name, parent = null) { this.name = name; this.parentElement = parent; this.children = []; this.listeners = {}; this.classList = new FakeClassList(); this.isConnected = true; if (parent) parent.children.push(this) }
    addEventListener (name, callback) { (this.listeners[name] ??= []).push(callback) }
    fire (name, event = {}) { for (const callback of this.listeners[name] ?? []) callback({ target: this, ...event }) }
    appendChild (child) { child.parentElement = this; child.isConnected = true; this.children.push(child) }
    remove () { this.isConnected = false; if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(x => x !== this) }
    closest (selector) { let el = this; while (el) { if (selector === '[dropZone]' && el.dropZoneOwner) return el; el = el.parentElement } return null }
    set innerHTML (value) { this.html = value }
}
let timerId = 0
const timers = new Map()
const timerErrors = []
const fakeSetTimeout = (fn, ms = 0) => { const id = ++timerId; timers.set(id, { fn, ms }); return id }
const fakeClearTimeout = id => timers.delete(id)
function runTimers (ms = Infinity) { for (const [id, timer] of [...timers]) if (timer.ms <= ms) { timers.delete(id); try { timer.fn() } catch (error) { timerErrors.push(error) } } }
const rootElement = new FakeElement('root')
const documentFake = { createElement: name => new FakeElement(name), elementFromPoint: () => hitElement }
let hitElement = null
const sharedModule = load(read('tabby-core/src/directives/dropZone.directive.ts'), { '../api/platform': coreModule, './dropZone.directive.scss': '', './dropZone.directive.pug': read('tabby-core/src/directives/dropZone.directive.pug') }, { document: documentFake, setTimeout: fakeSetTimeout, clearTimeout: fakeClearTimeout })
const SharedDropZone = sharedModule.DropZoneDirective
const hintEvents = []
function sharedOwner (name) {
    const el = new FakeElement(name, rootElement); el.dropZoneOwner = true
    const d = new SharedDropZone({ nativeElement: el }, { startUploadFromDragEvent: async () => ({ name }) })
    d.transfer.subscribe(v => hintEvents.push(v)); d.ngAfterViewInit()
    return { el, d }
}
const nativeModule = load(read('tabby-tauri/src/fileDrop.directive.ts'), { 'tabby-core': {}, '../../tabby-core/src/directives/dropZone.directive': { DropZoneDirective: SharedDropZone }, './api/hostBridge': {}, './services/platform.service': {} }, { document: documentFake })
const NativeDrop = nativeModule.TauriFileDropDirective
function nativeOwner (owner, platform, uploads = [], errors = []) {
    const d = new NativeDrop({ nativeElement: owner.el }, owner.d, platform, { error: e => errors.push(e) }); d.ngOnInit()
    owner.d.transfer.subscribe(transfer => uploads.push(transfer))
    return d
}
function nativePlatform (osName = 'windows') {
    const p = { runtimeInfo: { platform: osName }, fileDragged$: new rx.Subject(), fileDropped$: new rx.Subject(), getFileDropPosition: platformModule.TauriPlatformService.prototype.getFileDropPosition }
    p.getFileDropPosition = p.getFileDropPosition.bind(p)
    return p
}

await check('shared hint supports idempotent public hover toggling and reuses its existing markup', async () => {
    const owner = sharedOwner('shared'), hint = () => owner.el.children.find(x => x.className === 'drop-zone-hint')
    assert.equal(typeof owner.d.setHovered, 'function'); owner.d.setHovered(true); owner.d.setHovered(true)
    assert.ok(hint()); assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint').length, 1)
    assert.equal(hint().html, read('tabby-core/src/directives/dropZone.directive.pug'))
    runTimers(0); assert.ok(hint().classList.contains('visible'))
    owner.d.setHovered(false); assert.equal(hint().classList.contains('visible'), false); runTimers(500); assert.equal(hint(), undefined)
})
await check('shared DOM drag events call the same hover API and stale show timer cannot resurrect a removed hint', async () => {
    const owner = sharedOwner('dom'), original = owner.d.setHovered
    const errorCount = timerErrors.length
    let calls = 0; owner.d.setHovered = function (value) { calls++; return original.call(this, value) }
    owner.el.fire('dragover'); owner.el.fire('dragleave'); owner.el.fire('dragover'); owner.el.fire('dragleave'); runTimers(0)
    assert.ok(calls >= 4)
    assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 0)
    runTimers(500); assert.equal(timerErrors.length, errorCount, 'stale show callbacks must not dereference a removed or replaced hint')
    assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected).length, 0)
})
await check('rapid leave and re-entry timers cannot show a hint after hover ownership is lost', async () => {
    const owner = sharedOwner('timer-identity')
    owner.d.setHovered(true); owner.d.setHovered(false); owner.d.setHovered(true); owner.d.setHovered(false)
    runTimers(0)
    assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 0)
    runTimers(500)
    assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected).length, 0)
})
await check('native hover hits only nearest owning zone, follows moves, clears on leave and never uploads', async () => {
    const a = sharedOwner('owner-a'), b = sharedOwner('owner-b'), platform = nativePlatform(), uploads = []
    const da = nativeOwner(a, platform, uploads), db = nativeOwner(b, platform, uploads)
    const childA = new FakeElement('child-a', a.el), childB = new FakeElement('child-b', b.el)
    hitElement = childA; platform.fileDragged$.next({ x: 100, y: 80 }); runTimers(0)
    assert.equal(a.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 1)
    assert.equal(b.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 0)
    hitElement = childB; platform.fileDragged$.next({ x: 120, y: 90 }); runTimers(0)
    assert.equal(a.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 0)
    assert.equal(b.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 1)
    hitElement = new FakeElement('overlay'); platform.fileDragged$.next({ x: 400, y: 300 }); runTimers(0)
    assert.equal(b.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length, 0)
    platform.fileDragged$.next(null); assert.equal(uploads.length, 0)
    da.ngOnDestroy(); db.ngOnDestroy()
})
await check('native hit testing uses macOS logical points and scaled Windows/Linux pixels', async () => {
    const owner = sharedOwner('coordinates'), platform = nativePlatform(), d = nativeOwner(owner, platform)
    const points = []
    for (const osName of ['macos', 'windows', 'linux']) {
        platform.runtimeInfo.platform = osName
        documentFake.elementFromPoint = (x, y) => { points.push([x, y]); return owner.el }
        platform.fileDragged$.next({ x: 100, y: 80 })
    }
    assert.deepEqual(points, [[100, 80], [50, 40], [50, 40]], 'hover events must reach elementFromPoint with platform-adjusted coordinates')
    d.ngOnDestroy()
    documentFake.elementFromPoint = () => hitElement
})
await check('native drop clears hint before one upload; leave, destruction and reopen reject stale events', async () => {
    const owner = sharedOwner('drop'), platform = nativePlatform(), uploads = [], calls = []
    const hiddenWhenPreparing = []
    const visibleCount = () => owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected && x.classList.contains('visible')).length
    platform.startUploadFromPaths = async paths => { calls.push(paths); hiddenWhenPreparing.push(visibleCount() === 0); return { getChildrens: () => [] } }
    const d = nativeOwner(owner, platform, uploads), child = new FakeElement('inside', owner.el)
    hitElement = child; platform.fileDragged$.next({ x: 4, y: 8 }); runTimers(0)
    assert.equal(visibleCount(), 1, 'native hover must show its owner hint before Drop')
    documentFake.elementFromPoint = (x, y) => { assert.deepEqual([x, y], [2, 4]); return hitElement }
    platform.fileDropped$.next({ paths: ['/a.bin'], x: 4, y: 8 })
    assert.equal(visibleCount(), 0, 'Drop must clear active hover synchronously')
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(calls.length, 1); assert.deepEqual(calls[0], ['/a.bin']); assert.equal(uploads.length, 1)
    assert.deepEqual(hiddenWhenPreparing, [true], 'Drop must clear the active hint before upload preparation starts')
    assert.equal(visibleCount(), 0)
    runTimers(500); assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected).length, 0)
    d.ngOnDestroy()
    const destroyedOwner = nativeOwner(owner, platform, uploads); platform.fileDragged$.next({ x: 4, y: 8 }); runTimers(0)
    assert.equal(visibleCount(), 1)
    destroyedOwner.ngOnDestroy(); assert.equal(visibleCount(), 0, 'destroy must clear active hover immediately')
    runTimers(500); assert.equal(owner.el.children.filter(x => x.className === 'drop-zone-hint' && x.isConnected).length, 0)
    platform.fileDragged$.next({ x: 4, y: 8 }); runTimers(0); assert.equal(visibleCount(), 0, 'destroyed owners must ignore later events')
    const reopened = nativeOwner(owner, platform, uploads); platform.fileDragged$.next({ x: 4, y: 8 }); runTimers(0)
    assert.equal(visibleCount(), 1)
    reopened.ngOnDestroy()
})

await check('terminal path decorator still receives native paths only outside the owning SFTP zone', async () => {
    const paths = []
    const ownerlessElement = new FakeElement('outside')
    const decoratorBase = load(read('tabby-terminal/src/api/decorator.ts')).TerminalDecorator
    const terminalModule = load(read('tabby-tauri/src/pathDrop.ts'), { 'tabby-terminal': { TerminalDecorator: decoratorBase, encodeTerminalPath: p => p }, '../../tabby-local/src/api': {}, '../../tabby-local/src/components/terminalTab.component': { TerminalTabComponent: class {} } }, { document: { elementFromPoint: () => hitElement }, window: { devicePixelRatio: 2 } })
    const platform = { runtimeInfo: { platform: 'windows' }, fileDragged$: new rx.Subject(), fileDropped$: new rx.Subject(), getFileDropPosition: ({ x, y }) => ({ x: x / 2, y: y / 2 }) }
    const decorator = new terminalModule.TauriPathDropDecorator(platform)
    const terminal = { content: { nativeElement: { getBoundingClientRect: () => ({ left: 0, right: 100, top: 0, bottom: 100 }) } }, config: { store: { terminal: { bracketedPaste: false } } }, sendInput: p => paths.push(p) }
    decorator.attach(terminal); hitElement = ownerlessElement
    platform.fileDragged$.next({ x: 20, y: 20 }); assert.deepEqual(paths, [])
    platform.fileDropped$.next({ paths: ['/outside'], x: 20, y: 20 }); assert.deepEqual(paths, ['/outside'])
    hitElement = { closest: () => ({}) }; platform.fileDropped$.next({ paths: ['/owned'], x: 20, y: 20 }); assert.deepEqual(paths, ['/outside'])
    decorator.detach(terminal)
})

console.log(`SFTP drag hover: ${checks.length} passed; ${failures.length} failed`)
assert.equal(failures.length, 0, failures.join('; '))
