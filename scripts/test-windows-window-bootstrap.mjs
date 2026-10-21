import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

if (process.argv.includes('--help')) {
    console.log('Usage: node scripts/test-windows-window-bootstrap.mjs\nCompile and test the real Windows/Linux async bootstrap command and initialization registry. Uses a temporary Rust crate with the locked Tokio version; Tauri event dispatch and state are simulated. Requires Cargo and network or cached dependencies. Removes owned temporary files.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'no arguments supported; use --help')

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'src-tauri/src/commands/app.rs')
const registryPath = path.join(root, 'src-tauri/src/window_geometry_initialization.rs')
const registryTestsPath = path.join(root, 'src-tauri/src/window_geometry_initialization_tests.rs')
const libPath = path.join(root, 'src-tauri/src/lib.rs')
const files = [sourcePath, registryPath, registryTestsPath, libPath]
const fingerprints = new Map(files.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
const source = fs.readFileSync(sourcePath, 'utf8')
const marker = 'pub async fn app_bootstrap('
const start = source.indexOf(marker)
assert.ok(start >= 0 && start === source.lastIndexOf(marker), 'one Windows bootstrap command')
const body = source.indexOf('{', start)
let depth = 0
let quoted = false
let escaped = false
let comment = false
let end = -1
for (let index = body; index < source.length; index++) {
    const ch = source[index]
    if (comment) {
        if (ch === '\n') comment = false
    } else if (quoted) {
        if (escaped) escaped = false
        else if (ch === '\\') escaped = true
        else if (ch === '"') quoted = false
    } else if (ch === '"') quoted = true
    else if (ch === '/' && source[index + 1] === '/') comment = true
    else if (ch === '{') depth++
    else if (ch === '}' && --depth === 0) { end = index + 1; break }
}
assert.ok(end > body, 'complete Windows bootstrap command')
const command = source.slice(start, end)
const lib = fs.readFileSync(libPath, 'utf8')
const presentationStart = lib.indexOf('            for window in &windows {')
const presentationEnd = lib.indexOf('            let mut delivered = false;', presentationStart)
assert.ok(presentationStart >= 0 && presentationEnd > presentationStart, 'real launch presentation loop')
// Compile the Windows/Linux arm even when this fixture runs on another host.
const presentation = lib.slice(presentationStart, presentationEnd).replace(/#\[cfg\((?:windows|any\(windows, target_os = "linux"\))\)\]\s*/g, '')
const lock = fs.readFileSync(path.join(root, 'src-tauri/Cargo.lock'), 'utf8')
const tokio = lock.match(/\[\[package\]\]\r?\nname = "tokio"\r?\nversion = "([^"]+)"/)
assert.ok(tokio, 'locked Tokio version')
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-windows-bootstrap-'))
try {
    fs.mkdirSync(path.join(temporary, 'src'))
    fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname = "tabby-windows-bootstrap-fixture"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\ntokio = { version = "=${tokio[1]}", features = ["macros", "rt-multi-thread", "sync", "time"] }\n`)
    fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{any::{Any, TypeId}, collections::{HashMap, VecDeque}, future::Future, ops::Deref, pin::Pin, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}}, task::{Context, Poll, Wake, Waker}, thread::ThreadId};

mod window_geometry {
    #[path = ${JSON.stringify(registryPath)}]
    pub(crate) mod initialization;
}
use window_geometry::initialization::Initializations;

#[derive(Debug)]
struct AppError { message: String }
impl AppError {
    #[allow(non_snake_case)]
    fn Io(message: String) -> Self { Self { message } }
}
#[derive(Debug)]
struct BootstrapData { label: String }
struct EmptyRequest;
struct AppState { calls: Mutex<Vec<(String, ThreadId)>> }
struct State<'a, T>(&'a T);
impl<T> Deref for State<'_, T> { type Target = T; fn deref(&self) -> &T { self.0 } }

type UiTask = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct UiQueue { tasks: Mutex<VecDeque<UiTask>>, rejected: AtomicBool }
impl UiQueue {
    fn run_one(&self) -> bool {
        let task = self.tasks.lock().unwrap().pop_front();
        if let Some(task) = task { task(); true } else { false }
    }
    fn len(&self) -> usize { self.tasks.lock().unwrap().len() }
}
struct Shared { states: HashMap<TypeId, Box<dyn Any + Send + Sync>>, ui: UiQueue }
#[derive(Clone)]
struct AppHandle(Arc<Shared>);
#[derive(Clone)]
struct WebviewWindow { app: AppHandle, label: String, closed: Arc<AtomicBool>, presentations: Arc<AtomicUsize> }
trait Manager {
    fn shared(&self) -> &Shared;
    fn try_state<T: Any + Send + Sync>(&self) -> Option<State<'_, T>> {
        self.shared().states.get(&TypeId::of::<T>()).and_then(|value| value.downcast_ref()).map(State)
    }
    fn state<T: Any + Send + Sync>(&self) -> State<'_, T> { self.try_state().expect("fixture state exists") }
}
impl Manager for AppHandle { fn shared(&self) -> &Shared { &self.0 } }
impl Manager for WebviewWindow { fn shared(&self) -> &Shared { &self.app.0 } }
impl WebviewWindow {
    fn label(&self) -> &str { &self.label }
    fn show(&self) -> Result<(), String> { self.presentations.fetch_add(1, Ordering::Relaxed); Ok(()) }
    fn unminimize(&self) -> Result<(), String> { self.presentations.fetch_add(1, Ordering::Relaxed); Ok(()) }
    fn set_focus(&self) -> Result<(), String> { self.presentations.fetch_add(1, Ordering::Relaxed); Ok(()) }
    fn app_handle(&self) -> &AppHandle { &self.app }
    fn is_visible(&self) -> Result<bool, String> {
        if self.closed.load(Ordering::Acquire) { Err("window destroyed".into()) } else { Ok(false) }
    }
    fn run_on_main_thread<F: FnOnce() + Send + 'static>(&self, task: F) -> Result<(), String> {
        if self.app.0.ui.rejected.load(Ordering::Acquire) { return Err("event loop closed".into()); }
        self.app.0.ui.tasks.lock().unwrap().push_back(Box::new(task));
        Ok(())
    }
}
fn bootstrap_data(window: WebviewWindow, _request: EmptyRequest, state: State<'_, AppState>) -> Result<BootstrapData, AppError> {
    state.calls.lock().unwrap().push((window.label.clone(), std::thread::current().id()));
    Ok(BootstrapData { label: window.label })
}

${command}

fn present_launch_windows(handle: &AppHandle, windows: Vec<WebviewWindow>) {
${presentation}
}

#[cfg(test)]
mod checks {
    use super::*;
    struct Noop;
    impl Wake for Noop { fn wake(self: Arc<Self>) {} }
    fn pending<F: Future>(future: Pin<&mut F>) {
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        assert!(matches!(future.poll(&mut context), Poll::Pending));
    }
    fn app(with_registry: bool) -> AppHandle {
        let mut states: HashMap<TypeId, Box<dyn Any + Send + Sync>> = HashMap::new();
        states.insert(TypeId::of::<AppState>(), Box::new(AppState { calls: Mutex::new(Vec::new()) }));
        if with_registry { states.insert(TypeId::of::<Initializations>(), Box::new(Initializations::default())); }
        AppHandle(Arc::new(Shared { states, ui: UiQueue::default() }))
    }
    fn window(app: &AppHandle, label: &str) -> WebviewWindow {
        WebviewWindow { app: app.clone(), label: label.into(), closed: Arc::new(AtomicBool::new(false)), presentations: Arc::new(AtomicUsize::new(0)) }
    }
    fn run_ui(app: &AppHandle) {
        let for_thread = app.clone();
        assert!(std::thread::spawn(move || for_thread.0.ui.run_one()).join().unwrap());
    }

    #[test]
    fn concurrent_launch_does_not_present_pending_or_failed_windows() {
        let app = app(true);
        let pending = window(&app, "window-1");
        let failed = window(&app, "window-2");
        let custom = window(&app, "tools");
        let ticket = app.state::<Initializations>().begin("window-1").unwrap();
        app.state::<Initializations>().begin("window-2").unwrap().complete(Err("native failure".into()));
        present_launch_windows(&app, vec![pending.clone(), failed.clone(), custom.clone()]);
        assert_eq!(pending.presentations.load(Ordering::Relaxed), 0, "pending window must remain hidden");
        assert_eq!(failed.presentations.load(Ordering::Relaxed), 0, "failed window must remain hidden");
        assert_eq!(custom.presentations.load(Ordering::Relaxed), 3, "custom windows preserve presentation");
        ticket.complete(Ok(()));
        present_launch_windows(&app, vec![pending.clone(), failed.clone()]);
        assert_eq!(pending.presentations.load(Ordering::Relaxed), 3, "completed window can be presented");
        assert_eq!(failed.presentations.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn bootstrap_waits_and_runs_its_transaction_on_the_event_thread() {
        let app = app(true);
        let window = window(&app, "main");
        let ticket = app.state::<Initializations>().begin("main").unwrap();
        let mut future = Box::pin(app_bootstrap(window, EmptyRequest, app.state::<AppState>()));
        pending(future.as_mut());
        assert_eq!(app.0.ui.len(), 0);
        assert!(app.state::<AppState>().calls.lock().unwrap().is_empty());
        ticket.complete(Ok(()));
        pending(future.as_mut());
        assert_eq!(app.0.ui.len(), 1);
        run_ui(&app);
        assert_eq!(future.await.unwrap().label, "main");
        let state = app.state::<AppState>();
        let calls = state.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_ne!(calls[0].1, std::thread::current().id());
    }

    #[tokio::test]
    async fn an_initialization_failure_prevents_the_bootstrap_transaction() {
        let app = app(true);
        let ticket = app.state::<Initializations>().begin("main").unwrap();
        ticket.complete(Err("native size initialization failed".into()));
        let result = app_bootstrap(window(&app, "main"), EmptyRequest, app.state::<AppState>()).await;
        assert_eq!(result.unwrap_err().message, "native size initialization failed");
        assert_eq!(app.0.ui.len(), 0);
        assert!(app.state::<AppState>().calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn destruction_after_readiness_prevents_a_queued_bootstrap_transaction() {
        let app = app(true);
        let window = window(&app, "main");
        app.state::<Initializations>().begin("main").unwrap().complete(Ok(()));
        let mut future = Box::pin(app_bootstrap(window.clone(), EmptyRequest, app.state::<AppState>()));
        pending(future.as_mut());
        window.closed.store(true, Ordering::Release);
        run_ui(&app);
        assert_eq!(future.await.unwrap_err().message, "window destroyed");
        assert!(app.state::<AppState>().calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatch_failure_is_returned_without_running_the_transaction() {
        let app = app(true);
        app.0.ui.rejected.store(true, Ordering::Release);
        let result = app_bootstrap(window(&app, "tools"), EmptyRequest, app.state::<AppState>()).await;
        assert_eq!(result.unwrap_err().message, "event loop closed");
        assert!(app.state::<AppState>().calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_custom_host_without_a_geometry_registry_keeps_its_original_bootstrap() {
        let app = app(false);
        let mut future = Box::pin(app_bootstrap(window(&app, "tools"), EmptyRequest, app.state::<AppState>()));
        pending(future.as_mut());
        run_ui(&app);
        assert_eq!(future.await.unwrap().label, "tools");
    }
}
`)
    const result = spawnSync('cargo', ['test', '--manifest-path', path.join(temporary, 'Cargo.toml'), '--', '--nocapture'], { cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
    if (result.error) throw result.error
    assert.equal(result.signal, null, result.stderr)
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, /test result: ok\. 12 passed; 0 failed/, 'all command and registry checks executed')
    for (const file of files) assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), fingerprints.get(file), 'fixture inputs remained unchanged')
    console.log('Windows bootstrap command: readiness, event-thread transaction, failure, destruction, and custom-host cases passed; Tauri boundaries simulated')
} finally {
    fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
}
