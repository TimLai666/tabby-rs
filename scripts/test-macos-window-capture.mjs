import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const productionPath = path.join(root, 'src-tauri/src/window_persistence_macos.rs')
const modelPath = path.join(root, 'src-tauri/src/window_geometry_persistence.rs')
const lockPath = path.join(root, 'src-tauri/Cargo.lock')
const guardedFiles = [productionPath, modelPath, lockPath]
const fingerprints = new Map(guardedFiles.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
let temporary
let exitCode = 0
try {
    if (process.argv.length === 3 && process.argv[2] === '--help') {
        console.log('Usage: node scripts/test-macos-window-capture.mjs [--help]\nCompile the actual macOS window persistence module and window geometry model in a temporary Rust crate. Native window I/O and the AppState boundary are faked; this does not verify Wry or filesystem persistence. Runs cargo test --offline and removes its owned temporary crate.')
    } else {
        assert.equal(process.argv.length, 2, 'unknown arguments are rejected; use --help')
        const lock = fs.readFileSync(lockPath, 'utf8')
        const serde = lock.match(/\[\[package\]\]\r?\nname = "serde"\r?\nversion = "([^"]+)"/)
        const serdeJson = lock.match(/\[\[package\]\]\r?\nname = "serde_json"\r?\nversion = "([^"]+)"/)
        assert.ok(serde && serdeJson, 'Cargo.lock contains serde and serde_json')
        temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-macos-window-capture-'))
        fs.mkdirSync(path.join(temporary, 'src'))
        fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname = "tabby-macos-window-capture-fixture"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\nserde = { version = "=${serde[1]}", features = ["derive"] }\nserde_json = "=${serdeJson[1]}"\n`)
        fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{future::Future, pin::Pin, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}, task::{Context, Poll, Wake, Waker}};

mod error {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum AppError { InvalidArgument(String), Unsupported(String), Native(String) }
}
mod window_geometry {
    #[path = ${JSON.stringify(modelPath)}]
    pub(crate) mod persistence;
}
mod state {
    use crate::{error::AppError, window_geometry::persistence::{SavedWindowState, WindowStates}};
    use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};
    pub(crate) struct AppState {
        states: Mutex<WindowStates>,
        saved: SavedWindowState,
        pub calls: Mutex<Vec<&'static str>>,
        saves: AtomicUsize,
    }
    impl AppState {
        pub(crate) fn new(saved: SavedWindowState) -> Self { Self { states: Mutex::new(WindowStates::default()), saved, calls: Mutex::new(Vec::new()), saves: AtomicUsize::new(0) } }
        pub(crate) fn begin_window_state(&self, label: &str) -> Result<SavedWindowState, AppError> {
            self.calls.lock().unwrap().push("begin");
            if self.states.lock().unwrap().begin(label, self.saved) { Ok(self.saved) }
            else { Err(AppError::InvalidArgument(format!("window state already registered for {label}"))) }
        }
        pub(crate) fn ready_window_state(&self, label: &str) {
            self.calls.lock().unwrap().push("ready");
            self.states.lock().unwrap().ready(label);
        }
        pub(crate) fn record_window_state(&self, label: &str, sample: SavedWindowState) {
            self.calls.lock().unwrap().push("record");
            if let Some(bounds) = sample.bounds { self.states.lock().unwrap().record(label, bounds, sample.maximized); }
        }
        pub(crate) fn remove_window_state(&self, label: &str) {
            self.calls.lock().unwrap().push("remove");
            self.states.lock().unwrap().remove(label);
        }
        pub(crate) fn current(&self, label: &str) -> Option<SavedWindowState> { self.states.lock().unwrap().for_close(label, false) }
        pub(crate) fn save_calls(&self) -> usize { self.saves.load(Ordering::SeqCst) }
    }
}
use error::AppError;
use window_geometry::persistence::{LogicalBounds, SavedWindowState};

#[derive(Clone)]
pub struct WebviewWindow { label: String, native: NativeInput }
impl WebviewWindow {
    fn new(label: &str, native: NativeInput) -> Self { Self { label: label.into(), native } }
    fn label(&self) -> &str { &self.label }
}
#[derive(Clone)]
struct NativeInput {
    restore_error: Option<String>,
    restore_event: Option<SavedWindowState>,
    read_result: Result<SavedWindowState, String>,
    native_calls: Arc<AtomicUsize>,
    barrier: ManualBarrier,
}
impl NativeInput {
    fn new() -> Self { Self { restore_error: None, restore_event: None, read_result: Ok(SavedWindowState::default()), native_calls: Arc::new(AtomicUsize::new(0)), barrier: ManualBarrier::default() } }
}
#[derive(Default, Clone)]
struct ManualBarrier(Arc<Mutex<BarrierState>>);
#[derive(Default)]
struct BarrierState { result: Option<Result<(), AppError>>, waiters: Vec<Waker> }
impl ManualBarrier {
    fn complete(&self, result: Result<(), AppError>) {
        let waiters = {
            let mut state = self.0.lock().unwrap();
            state.result = Some(result);
            std::mem::take(&mut state.waiters)
        };
        for waiter in waiters { waiter.wake(); }
    }
}
struct BarrierWait(ManualBarrier);
impl Future for BarrierWait {
    type Output = Result<(), AppError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.0.lock().unwrap();
        if let Some(result) = state.result.clone() { Poll::Ready(result) }
        else {
            if !state.waiters.iter().any(|waker| waker.will_wake(cx.waker())) { state.waiters.push(cx.waker().clone()); }
            Poll::Pending
        }
    }
}
thread_local! { static RESTORE_STATE: std::cell::RefCell<Option<Arc<state::AppState>>> = const { std::cell::RefCell::new(None) }; }
mod platform {
    pub(crate) fn wait_for_window_operations(window: &crate::WebviewWindow) -> crate::BarrierWait { crate::BarrierWait(window.native.barrier.clone()) }
    pub(crate) mod window_state_macos {
        use crate::{error::AppError, state::AppState, SavedWindowState, WebviewWindow};
        pub(crate) fn restore_startup(window: &WebviewWindow, _saved: SavedWindowState) -> Result<(), AppError> {
            window.native.native_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(event) = window.native.restore_event {
                crate::RESTORE_STATE.with(|slot| if let Some(state) = slot.borrow().as_ref() { state.record_window_state(window.label(), event); });
            }
            match &window.native.restore_error { Some(error) => Err(AppError::Native(error.clone())), None => Ok(()) }
        }
        pub(crate) fn read(window: &WebviewWindow) -> Result<SavedWindowState, AppError> {
            window.native.native_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            window.native.read_result.clone().map_err(AppError::Native)
        }
        pub(crate) fn set_restore_state(state: Option<std::sync::Arc<AppState>>) {
            crate::RESTORE_STATE.with(|slot| *slot.borrow_mut() = state);
        }
    }
}
mod window_persistence_macos {
    #[path = ${JSON.stringify(productionPath)}]
    pub(crate) mod actual;
}
use state::AppState;

#[cfg(test)]
mod tests {
    use super::*;
    struct Noop;
    impl Wake for Noop { fn wake(self: Arc<Self>) {} }
    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        future.poll(&mut context)
    }
    fn bounds(x: i32) -> LogicalBounds { LogicalBounds { x, y: 20, width: 900, height: 600 } }
    fn saved(x: i32) -> SavedWindowState { SavedWindowState { bounds: Some(bounds(x)), maximized: false } }
    fn app(x: i32) -> Arc<AppState> { Arc::new(AppState::new(saved(x))) }
    fn app_with(saved: SavedWindowState) -> Arc<AppState> { Arc::new(AppState::new(saved)) }
    fn window(label: &str) -> WebviewWindow { WebviewWindow::new(label, NativeInput::new()) }
    fn restore(state: &Arc<AppState>, window: &WebviewWindow) -> Result<(), AppError> {
        platform::window_state_macos::set_restore_state(Some(state.clone()));
        let result = window_persistence_macos::actual::restore_window(window, state);
        platform::window_state_macos::set_restore_state(None);
        result
    }

    #[test]
    fn restore_and_pending_enable_keep_seed_until_barrier_then_capture_updates() {
        let state = app(11);
        let mut native = NativeInput::new();
        native.restore_event = Some(saved(99));
        let barrier = native.barrier.clone();
        let window = WebviewWindow::new("main", native);
        restore(&state, &window).unwrap();
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(11)), "restore-time event must not overwrite seed");
        assert_eq!(*state.calls.lock().unwrap(), vec!["begin", "record"], "restore returns without enabling capture");

        let mut enabling = Box::pin(window_persistence_macos::actual::enable_capture(&window, &state));
        assert!(matches!(poll(enabling.as_mut()), Poll::Pending), "enable_capture returned before the native operation barrier completed");
        state.record_window_state("main", saved(101));
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(11)), "events while operation barrier is pending must not overwrite seed");
        assert!(!state.calls.lock().unwrap().contains(&"ready"), "pending enable must not mark the window ready");

        barrier.complete(Ok(()));
        assert_eq!(poll(enabling.as_mut()), Poll::Ready(Ok(())));
        state.record_window_state("main", saved(12));
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(12)), "normal bounds after barrier completion must update cache");
    }

    #[test]
    fn enable_barrier_error_preserves_seed_and_returns_original_error() {
        let state = app(11);
        let window = window("main");
        restore(&state, &window).unwrap();
        let mut enabling = Box::pin(window_persistence_macos::actual::enable_capture(&window, &state));
        assert!(matches!(poll(enabling.as_mut()), Poll::Pending), "enable_capture returned before the native operation barrier completed");
        window.native.barrier.complete(Err(AppError::Native("operation wait failed".into())));
        assert_eq!(poll(enabling.as_mut()), Poll::Ready(Err(AppError::Native("operation wait failed".into()))));
        state.record_window_state("main", saved(33));
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(11)), "failed barrier must leave label unready");
        assert!(!state.calls.lock().unwrap().contains(&"ready"));
    }

    #[test]
    fn dropping_pending_enable_future_never_marks_ready() {
        let state = app(11);
        let window = window("main");
        restore(&state, &window).unwrap();
        let mut enabling = Box::pin(window_persistence_macos::actual::enable_capture(&window, &state));
        assert!(matches!(poll(enabling.as_mut()), Poll::Pending), "enable_capture returned before the native operation barrier completed");
        drop(enabling);
        state.record_window_state("main", saved(33));
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(11)));
        assert!(!state.calls.lock().unwrap().contains(&"ready"));
    }

    #[test]
    fn restore_failure_removes_only_its_label_and_returns_original_error() {
        let state = app(11);
        assert!(state.begin_window_state("peer").is_ok());
        let mut native = NativeInput::new();
        native.restore_error = Some("native restore failed".into());
        let window = WebviewWindow::new("main", native);
        let result = restore(&state, &window);
        assert_eq!(result, Err(AppError::Native("native restore failed".into())));
        assert_eq!(state.current("main"), None);
        assert_eq!(state.current("peer").unwrap().bounds, Some(bounds(11)));
        assert_eq!(window.native.native_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicate_begin_does_not_call_native_or_damage_existing_record() {
        let state = app(11);
        assert!(state.begin_window_state("main").is_ok());
        state.ready_window_state("main");
        state.record_window_state("main", saved(25));
        let window = window("main");
        assert!(matches!(restore(&state, &window), Err(AppError::InvalidArgument(_))));
        assert_eq!(window.native.native_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(25)));
    }

    #[test]
    fn capture_records_only_normal_bounds_in_memory_without_saving() {
        let state = app(11);
        state.begin_window_state("main").unwrap();
        state.ready_window_state("main");
        let mut native = NativeInput::new();
        native.read_result = Ok(saved(40));
        let window = WebviewWindow::new("main", native);
        window_persistence_macos::actual::capture_window(&window, &state).unwrap();
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(40)));
        assert_eq!(state.save_calls(), 0);
        assert_eq!(window.native.native_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn maximized_and_missing_samples_do_not_replace_normal_bounds() {
        let state = app(11);
        state.begin_window_state("main").unwrap();
        state.ready_window_state("main");
        let mut maximized = NativeInput::new();
        maximized.read_result = Ok(SavedWindowState { bounds: Some(bounds(80)), maximized: true });
        assert!(window_persistence_macos::actual::capture_window(&WebviewWindow::new("main", maximized), &state).is_ok());
        let mut missing = NativeInput::new();
        missing.read_result = Ok(SavedWindowState { bounds: None, maximized: false });
        assert!(window_persistence_macos::actual::capture_window(&WebviewWindow::new("main", missing), &state).is_ok());
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(11)));
    }

    #[test]
    fn unknown_and_removed_labels_cannot_be_resurrected_or_affect_peers() {
        let state = app(11);
        assert!(state.begin_window_state("first").is_ok());
        assert!(state.begin_window_state("second").is_ok());
        state.ready_window_state("first");
        state.ready_window_state("second");
        let mut native = NativeInput::new();
        native.read_result = Ok(saved(70));
        let window = WebviewWindow::new("unknown", native.clone());
        assert!(window_persistence_macos::actual::capture_window(&window, &state).is_ok());
        assert_eq!(state.current("unknown"), None, "recording cannot resurrect an unknown label");
        assert_eq!(state.current("first").unwrap().bounds, Some(bounds(11)));
        assert_eq!(state.current("second").unwrap().bounds, Some(bounds(11)));
        state.remove_window_state("first");
        assert!(window_persistence_macos::actual::capture_window(&WebviewWindow::new("first", native), &state).is_ok());
        assert_eq!(state.current("first"), None, "recording after removal cannot resurrect a label");
        assert_eq!(state.current("second").unwrap().bounds, Some(bounds(11)));
    }

    #[test]
    fn default_saved_state_survives_restore_and_enable_then_accepts_capture() {
        let state = app_with(SavedWindowState::default());
        let window = window("empty-seed");
        restore(&state, &window).unwrap();
        assert_eq!(state.current("empty-seed").unwrap().bounds, None);
        let mut enabling = Box::pin(window_persistence_macos::actual::enable_capture(&window, &state));
        assert!(matches!(poll(enabling.as_mut()), Poll::Pending), "enable_capture returned before the native operation barrier completed");
        assert_eq!(state.current("empty-seed").unwrap().bounds, None);
        window.native.barrier.complete(Ok(()));
        assert_eq!(poll(enabling.as_mut()), Poll::Ready(Ok(())));
        let mut native = NativeInput::new();
        native.read_result = Ok(saved(48));
        window_persistence_macos::actual::capture_window(&WebviewWindow::new("empty-seed", native), &state).unwrap();
        assert_eq!(state.current("empty-seed").unwrap().bounds, Some(bounds(48)));
        assert_eq!(state.save_calls(), 0);
    }

    #[test]
    fn native_read_error_is_returned_and_retains_old_cache() {
        let state = app(11);
        state.begin_window_state("main").unwrap();
        state.ready_window_state("main");
        let mut native = NativeInput::new();
        native.read_result = Err("native read failed".into());
        let window = WebviewWindow::new("main", native);
        assert_eq!(window_persistence_macos::actual::capture_window(&window, &state), Err(AppError::Native("native read failed".into())));
        assert_eq!(state.current("main").unwrap().bounds, Some(bounds(11)));
        assert_eq!(state.save_calls(), 0);
    }
}
`)
        const result = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml'), '--', '--nocapture'], { cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
        if (result.error) throw result.error
        process.stdout.write(result.stdout)
        process.stderr.write(result.stderr)
        assert.equal(result.signal, null, `cargo terminated by ${result.signal}`)
        assert.equal(result.status, 0, 'macOS capture and readiness behavior must pass')
        assert.match(result.stdout, /test result: ok\. 33 passed; 0 failed/, 'all model and capture checks executed')
        console.log('All macOS window persistence behavior checks passed; native and AppState boundaries are simulated')
    }
} catch (error) {
    console.error(error)
    exitCode = 1
} finally {
    try {
        for (const file of guardedFiles) {
            const actual = crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
            assert.equal(actual, fingerprints.get(file), `source guard changed: ${file}`)
        }
        console.log(`SHA-256 source guard passed for ${guardedFiles.length} inputs`)
    } catch (error) {
        console.error(error)
        exitCode = 1
    }
    if (temporary) {
        try { fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 }) }
        catch (error) { console.error(`could not remove owned temporary crate ${temporary}: ${error}`); exitCode = 1 }
    }
}
process.exitCode = exitCode
