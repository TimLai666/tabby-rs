import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const productionPath = path.join(root, 'src-tauri/src/window_persistence_linux.rs')
const modelPath = path.join(root, 'src-tauri/src/window_geometry_persistence.rs')
const lockPath = path.join(root, 'src-tauri/Cargo.lock')
const guardedFiles = [productionPath, modelPath, lockPath]
const fingerprints = new Map(guardedFiles.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
let temporary
let exitCode = 0
try {
    if (process.argv.length === 3 && process.argv[2] === '--help') {
        console.log('Usage: node scripts/test-linux-window-capture.mjs [--help]\nCompiles the actual Linux window persistence module and WindowStates model in an owned temporary Rust crate. Native window I/O and AppState are faked; this does not verify Wry or filesystem persistence. Runs cargo test --offline and removes its temporary crate.')
    } else {
        assert.equal(process.argv.length, 2, 'unknown arguments are rejected; use --help')
        const lock = fs.readFileSync(lockPath, 'utf8')
        const serde = lock.match(/\[\[package\]\]\r?\nname = "serde"\r?\nversion = "([^"]+)"/)
        const serdeJson = lock.match(/\[\[package\]\]\r?\nname = "serde_json"\r?\nversion = "([^"]+)"/)
        assert.ok(serde && serdeJson, 'Cargo.lock contains serde and serde_json')
        temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-linux-window-capture-'))
        fs.mkdirSync(path.join(temporary, 'src'))
        fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname = "tabby-linux-window-capture-fixture"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\nserde = { version = "=${serde[1]}", features = ["derive"] }\nserde_json = "=${serdeJson[1]}"\n`)
        fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Waker}};
mod error {
    #[derive(Debug, Clone, PartialEq, Eq)] pub(crate) enum AppError { InvalidArgument(String), Unsupported(String), Native(String) }
}
mod window_geometry {
    #[path = ${JSON.stringify(modelPath)}] pub(crate) mod persistence;
}
mod state {
    use crate::{error::AppError, window_geometry::persistence::{LogicalBounds, SavedWindowState, WindowStates}};
    use std::sync::Mutex;
    pub(crate) struct AppState { states: Mutex<WindowStates>, saved: SavedWindowState, pub calls: Mutex<Vec<&'static str>>, pub save_calls: Mutex<usize> }
    impl AppState {
        pub(crate) fn new(saved: SavedWindowState) -> Self { Self { states: Mutex::new(WindowStates::default()), saved, calls: Mutex::new(Vec::new()), save_calls: Mutex::new(0) } }
        pub(crate) fn begin_window_state(&self, label: &str) -> Result<SavedWindowState, AppError> { self.calls.lock().unwrap().push("begin"); if self.states.lock().unwrap().begin(label, self.saved) { Ok(self.saved) } else { Err(AppError::InvalidArgument(format!("window state already registered for {label}"))) } }
        pub(crate) fn ready_window_state_with_sample(&self, label: &str, sample: LogicalBounds) { self.calls.lock().unwrap().push("ready_sample"); self.states.lock().unwrap().ready_with_sample(label, sample); }
        pub(crate) fn ready_window_state_with_origin(&self, label: &str, sample: LogicalBounds) { self.calls.lock().unwrap().push("ready_origin"); self.states.lock().unwrap().ready_with_origin(label, sample); }
        pub(crate) fn window_sampling_bounds(&self, label: &str) -> Option<LogicalBounds> { self.states.lock().unwrap().sampling_bounds(label) }
        pub(crate) fn record_window_state(&self, label: &str, sample: SavedWindowState) { self.calls.lock().unwrap().push("record"); if let Some(bounds) = sample.bounds { self.states.lock().unwrap().record(label, bounds, sample.maximized); } }
        pub(crate) fn remove_window_state(&self, label: &str) { self.calls.lock().unwrap().push("remove"); self.states.lock().unwrap().remove(label); }
        pub(crate) fn current(&self, label: &str) -> Option<SavedWindowState> { self.states.lock().unwrap().for_close(label, false) }
        pub(crate) fn startup_window_maximized(&self, label: &str) -> Option<bool> { self.states.lock().unwrap().startup_maximized(label) }
        pub(crate) fn begin_direct(&self, label: &str, saved: SavedWindowState) { assert!(self.states.lock().unwrap().begin(label, saved)); }
        pub(crate) fn ready_direct(&self, label: &str, sample: LogicalBounds) { self.states.lock().unwrap().ready_with_sample(label, sample); }
    }
}
use error::AppError;
use window_geometry::persistence::{LogicalBounds, SavedWindowState};
fn fixture_bounds(x:i32,y:i32,width:u32,height:u32)->LogicalBounds { LogicalBounds{x,y,width,height} }
fn fixture_saved(bounds:Option<LogicalBounds>,maximized:bool)->SavedWindowState { SavedWindowState{bounds,maximized} }
#[derive(Clone)] pub struct WebviewWindow { label: String, native: NativeInput }
impl WebviewWindow { fn new(label: &str, native: NativeInput) -> Self { Self { label: label.into(), native } } fn label(&self) -> &str { &self.label } }
#[derive(Clone)] struct NativeInput { restore_result: Result<LogicalBounds, String>, read_result: Result<SavedWindowState, String>, absolute_positioning: Result<bool, String>, calls: Arc<Mutex<Vec<String>>>, barrier: ManualBarrier }
impl NativeInput { fn new() -> Self { Self { restore_result: Ok(fixture_bounds(10, 20, 800, 600)), read_result: Ok(fixture_saved(Some(fixture_bounds(30, 40, 800, 600)), false)), absolute_positioning: Ok(false), calls: Arc::new(Mutex::new(Vec::new())), barrier: ManualBarrier::default() } } }
#[derive(Default, Clone)] struct ManualBarrier(Arc<Mutex<BarrierState>>);
#[derive(Default)] struct BarrierState { result: Option<Result<(), AppError>>, waiters: Vec<Waker> }
impl ManualBarrier { fn complete(&self, result: Result<(), AppError>) { let waiters = { let mut state = self.0.lock().unwrap(); state.result = Some(result); std::mem::take(&mut state.waiters) }; for waiter in waiters { waiter.wake(); } } }
struct BarrierWait(ManualBarrier);
impl Future for BarrierWait { type Output = Result<(), AppError>; fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> { let mut state = self.0.0.lock().unwrap(); if let Some(result) = state.result.clone() { Poll::Ready(result) } else { if !state.waiters.iter().any(|w| w.will_wake(cx.waker())) { state.waiters.push(cx.waker().clone()); } Poll::Pending } } }
mod platform {
    pub(crate) fn wait_for_window_operations(window: &crate::WebviewWindow) -> crate::BarrierWait { crate::BarrierWait(window.native.barrier.clone()) }
    pub(crate) fn supports_absolute_positioning(window: &crate::WebviewWindow) -> Result<bool, crate::AppError> { window.native.calls.lock().unwrap().push("supports_absolute_positioning".into()); window.native.absolute_positioning.clone().map_err(crate::AppError::Native) }
    pub(crate) mod window_state_linux {
        use crate::{error::AppError, window_geometry::persistence::{LogicalBounds, SavedWindowState}, WebviewWindow};
        pub(crate) fn restore_startup(window: &WebviewWindow, saved: SavedWindowState, origin: Option<(i32,i32)>) -> Result<LogicalBounds,AppError> { window.native.calls.lock().unwrap().push(format!("restore:{saved:?}:{origin:?}")); window.native.restore_result.clone().map_err(AppError::Native) }
        pub(crate) fn read(window: &WebviewWindow, origin: LogicalBounds) -> Result<SavedWindowState,AppError> { window.native.calls.lock().unwrap().push(format!("read:{origin:?}")); window.native.read_result.clone().map_err(AppError::Native) }
        pub(crate) fn read_for_capture(window: &WebviewWindow, origin: LogicalBounds) -> Result<Option<SavedWindowState>,AppError> { read(window, origin).map(Some) }
    }
}
mod window_persistence_linux { #[path = ${JSON.stringify(productionPath)}] pub(crate) mod actual; }
#[cfg(test)] mod tests {
    use super::*;
    use crate::state::AppState;
    struct Noop; impl std::task::Wake for Noop { fn wake(self: Arc<Self>) {} }
    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> { future.poll(&mut Context::from_waker(&Waker::from(Arc::new(Noop)))) }
    fn bounds(x: i32, y: i32, width: u32, height: u32) -> LogicalBounds { LogicalBounds { x, y, width, height } }
    fn saved(bounds: Option<LogicalBounds>, maximized: bool) -> SavedWindowState { SavedWindowState { bounds, maximized } }
    fn state(seed: SavedWindowState) -> AppState { AppState::new(seed) }
    fn window(label: &str) -> WebviewWindow { WebviewWindow::new(label, NativeInput::new()) }
    #[test] fn restore_returns_raw_constructor_and_keeps_seed_unready_without_saving() {
        let seed = saved(Some(bounds(-2400, 10, 1200, 800)), false); let app = state(seed); let mut native = NativeInput::new(); native.restore_result = Ok(bounds(7, 9, 200, 100)); let window = WebviewWindow::new("main", native);
        assert_eq!(window_persistence_linux::actual::restore_window(&window, &app, Some((7,9))), Ok(bounds(7,9,200,100)));
        assert_eq!(app.current("main").unwrap(), seed); assert_eq!(app.window_sampling_bounds("main"), Some(bounds(-2400,10,1200,800))); assert_eq!(*app.save_calls.lock().unwrap(), 0);
        assert_eq!(*window.native.calls.lock().unwrap(), vec![format!("restore:{seed:?}:Some((7, 9))")]);
        let mut tiny = NativeInput::new(); tiny.restore_result = Ok(bounds(0,0,1,1)); let tiny = WebviewWindow::new("tiny", tiny); let fresh = state(SavedWindowState::default());
        assert_eq!(window_persistence_linux::actual::restore_window(&tiny, &fresh, None), Ok(bounds(0,0,1,1)));
    }
    #[test] fn restore_error_removes_only_own_label_and_duplicate_does_not_touch_record() {
        let seed = saved(Some(bounds(20,30,700,500)), false); let state = state(seed); state.begin_direct("peer", saved(Some(bounds(2,3,4,5)), false));
        let mut bad = NativeInput::new(); bad.restore_result = Err("native restore failed".into()); let bad = WebviewWindow::new("main", bad);
        assert_eq!(window_persistence_linux::actual::restore_window(&bad, &state, None), Err(AppError::Native("native restore failed".into()))); assert_eq!(state.current("main"), None); assert!(state.current("peer").is_some());
        state.begin_direct("main", seed); state.ready_direct("main", bounds(50,60,900,700));
        let duplicate = window("main"); assert!(matches!(window_persistence_linux::actual::restore_window(&duplicate, &state, None), Err(AppError::InvalidArgument(_))));
        assert!(duplicate.native.calls.lock().unwrap().is_empty()); assert_eq!(state.window_sampling_bounds("main"), Some(bounds(50,60,900,700))); assert_eq!(state.current("peer"), Some(saved(Some(bounds(2,3,4,5)), false)));
    }
    #[test] fn restore_passes_maximized_seed_and_preserves_its_flag() {
        let seed=saved(Some(bounds(-1900,20,1000,700)),true); let state=state(seed); let mut native=NativeInput::new(); native.restore_result=Ok(bounds(12,14,900,650)); let window=WebviewWindow::new("main",native);
        assert_eq!(window_persistence_linux::actual::restore_window(&window,&state,None),Ok(bounds(12,14,900,650)));
        assert_eq!(*window.native.calls.lock().unwrap(),vec![format!("restore:{seed:?}:None")]); assert_eq!(state.current("main"),Some(saved(seed.bounds,false))); assert_eq!(state.startup_window_maximized("main"),Some(true));
    }
    #[test] fn barrier_controls_readiness_and_error_or_cancellation_preserves_seed_and_peer() {
        let seed = saved(Some(bounds(-1800,70,1000,700)), false); let state = state(seed); let mut main_window = window("main"); let peer_seed = saved(Some(bounds(90,80,700,500)), false); state.begin_direct("peer", peer_seed);
        assert!(window_persistence_linux::actual::restore_window(&main_window, &state, None).is_ok()); let mut pending = Box::pin(window_persistence_linux::actual::enable_capture(&main_window,&state,bounds(100,110,900,650)));
        assert!(matches!(poll(pending.as_mut()), Poll::Pending)); window_persistence_linux::actual::capture_window(&main_window,&state).unwrap(); assert_eq!(state.current("main"),Some(seed)); assert_eq!(state.current("peer"),Some(peer_seed)); assert_eq!(state.window_sampling_bounds("main"),Some(bounds(-1800,70,1000,700)));
        main_window.native.barrier.complete(Ok(())); assert_eq!(poll(pending.as_mut()),Poll::Ready(Ok(()))); assert_eq!(state.current("main"),Some(seed)); assert_eq!(state.window_sampling_bounds("main"),Some(bounds(100,110,900,650))); drop(pending);
        main_window.native.read_result=Ok(saved(Some(bounds(100,110,900,650)),false)); window_persistence_linux::actual::capture_window(&main_window,&state).unwrap(); assert_eq!(state.current("main"),Some(seed));
        main_window.native.read_result=Ok(saved(Some(bounds(100,110,901,650)),false)); window_persistence_linux::actual::capture_window(&main_window,&state).unwrap(); assert_eq!(state.current("main"),Some(saved(Some(bounds(100,110,901,650)),false)));
        let failed_window = window("failed"); failed_window.native.barrier.complete(Err(AppError::Native("barrier failed".into()))); assert!(window_persistence_linux::actual::restore_window(&failed_window,&state,None).is_ok());
        let mut future = Box::pin(window_persistence_linux::actual::enable_capture(&failed_window,&state,bounds(1,2,3,4))); assert_eq!(poll(future.as_mut()),Poll::Ready(Err(AppError::Native("barrier failed".into())))); drop(future); assert_eq!(state.current("failed"),Some(seed)); assert_eq!(state.window_sampling_bounds("failed"),Some(bounds(-1800,70,1000,700)));
        let cancelled = window("cancelled"); assert!(window_persistence_linux::actual::restore_window(&cancelled,&state,None).is_ok()); let mut future=Box::pin(window_persistence_linux::actual::enable_capture(&cancelled,&state,bounds(1,2,3,4))); assert!(matches!(poll(future.as_mut()),Poll::Pending)); drop(future); assert_eq!(state.current("cancelled"),Some(seed)); assert_eq!(state.window_sampling_bounds("cancelled"),Some(bounds(-1800,70,1000,700))); assert_eq!(*state.save_calls.lock().unwrap(),0);
    }
    #[test] fn pending_or_failed_barrier_does_not_query_window_backend() {
        let seed=saved(Some(bounds(-1800,70,1000,700)),false); let state=state(seed); let pending=window("pending"); state.begin_direct("pending",seed);
        let mut waiting=Box::pin(window_persistence_linux::actual::enable_capture(&pending,&state,bounds(100,110,900,650))); assert!(matches!(poll(waiting.as_mut()),Poll::Pending)); assert!(pending.native.calls.lock().unwrap().is_empty()); drop(waiting);
        let failed=window("failed"); failed.native.barrier.complete(Err(AppError::Native("barrier failed".into()))); state.begin_direct("failed",seed);
        let mut rejected=Box::pin(window_persistence_linux::actual::enable_capture(&failed,&state,bounds(100,110,900,650))); assert_eq!(poll(rejected.as_mut()),Poll::Ready(Err(AppError::Native("barrier failed".into())))); assert!(failed.native.calls.lock().unwrap().is_empty()); assert_eq!(state.current("failed"),Some(saved(seed.bounds,false)));
    }
    #[test] fn x11_first_normal_sample_updates_absent_and_offscreen_cache_after_barrier() {
        for (label,seed) in [("absent",SavedWindowState::default()),("offscreen",saved(Some(bounds(-2600,40,1200,800)),false))] {
            let state=state(seed); state.begin_direct(label,seed); let constructor=bounds(60,50,900,650); let mut native=NativeInput::new(); native.absolute_positioning=Ok(true); native.read_result=Ok(saved(Some(constructor),false)); let window=WebviewWindow::new(label,native);
            let mut enabling=Box::pin(window_persistence_linux::actual::enable_capture(&window,&state,constructor)); assert!(matches!(poll(enabling.as_mut()),Poll::Pending)); assert!(window.native.calls.lock().unwrap().is_empty()); window.native.barrier.complete(Ok(())); assert_eq!(poll(enabling.as_mut()),Poll::Ready(Ok(()))); drop(enabling);
            assert_eq!(state.current(label),Some(saved(seed.bounds,false)),"readiness does not persist constructor state: {label}"); assert_eq!(state.window_sampling_bounds(label),Some(constructor)); assert_eq!(window.native.calls.lock().unwrap().as_slice(),["supports_absolute_positioning"]);
            window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current(label),Some(saved(Some(constructor),false)),"first unchanged X11 sample becomes normal bounds: {label}"); assert_eq!(*state.save_calls.lock().unwrap(),0);
        }
    }
    #[test] fn x11_maximized_sample_preserves_normal_bounds_and_duplicate_readiness_isolated_by_label() {
        let normal=bounds(-900,20,1000,700); let constructor=bounds(80,70,950,650); let peer_seed=saved(Some(bounds(10,20,600,400)),false); let state=state(saved(Some(normal),false)); state.begin_direct("max",saved(Some(normal),true)); state.begin_direct("peer",peer_seed);
        let mut native=NativeInput::new(); native.absolute_positioning=Ok(true); native.read_result=Ok(saved(Some(bounds(0,0,1920,1080)),true)); let mut window=WebviewWindow::new("max",native);
        let mut ready=Box::pin(window_persistence_linux::actual::enable_capture(&window,&state,constructor)); window.native.barrier.complete(Ok(())); assert_eq!(poll(ready.as_mut()),Poll::Ready(Ok(()))); drop(ready); assert_eq!(state.current("max"),Some(saved(Some(normal),false))); assert_eq!(state.startup_window_maximized("max"),Some(true));
        window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current("max"),Some(saved(Some(normal),false)),"maximized event does not replace normal bounds");
        let moved=bounds(80,70,951,650); window.native.read_result=Ok(saved(Some(moved),false)); window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current("max"),Some(saved(Some(moved),false)));
        let new_origin=bounds(700,600,500,350); let mut duplicate=Box::pin(window_persistence_linux::actual::enable_capture(&window,&state,new_origin)); assert_eq!(poll(duplicate.as_mut()),Poll::Ready(Ok(()))); drop(duplicate); assert_eq!(state.window_sampling_bounds("max"),Some(moved)); assert_eq!(state.current("peer"),Some(saved(peer_seed.bounds,false))); assert_eq!(*state.calls.lock().unwrap(),vec!["ready_origin","record","record","ready_origin"]); assert_eq!(*state.save_calls.lock().unwrap(),0);
    }
    #[test] fn backend_query_error_does_not_arm_capture_or_change_cached_bounds() {
        let seed=saved(Some(bounds(-1800,70,1000,700)),false); let state=state(seed); state.begin_direct("main",seed); let constructor=bounds(100,110,900,650); let mut native=NativeInput::new(); native.absolute_positioning=Err("backend query failed".into()); let window=WebviewWindow::new("main",native);
        let mut enabling=Box::pin(window_persistence_linux::actual::enable_capture(&window,&state,constructor)); window.native.barrier.complete(Ok(())); assert_eq!(poll(enabling.as_mut()),Poll::Ready(Err(AppError::Native("backend query failed".into())))); drop(enabling);
        assert_eq!(state.current("main"),Some(saved(seed.bounds,false))); assert_eq!(state.window_sampling_bounds("main"),seed.bounds); assert!(state.calls.lock().unwrap().is_empty()); assert_eq!(window.native.calls.lock().unwrap().as_slice(),["supports_absolute_positioning"]);
    }
    #[test] fn absent_seed_and_unchanged_constructor_are_suppressed_until_changed_sample() {
        let state=state(SavedWindowState::default()); let mut window=window("main"); let constructor=bounds(120,80,200,100); window_persistence_linux::actual::restore_window(&window,&state,Some((120,80))).unwrap(); let mut ready=Box::pin(window_persistence_linux::actual::enable_capture(&window,&state,constructor)); assert!(matches!(poll(ready.as_mut()),Poll::Pending)); window.native.barrier.complete(Ok(())); assert_eq!(poll(ready.as_mut()),Poll::Ready(Ok(()))); drop(ready);
        window.native.read_result=Ok(saved(Some(constructor),false)); window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current("main"),Some(SavedWindowState::default()));
        let changed=bounds(120,80,201,100); window.native.read_result=Ok(saved(Some(changed),false)); window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current("main"),Some(saved(Some(changed),false))); assert_eq!(*state.save_calls.lock().unwrap(),0);
    }
    #[test] fn sampling_uses_latest_origin_and_noops_unknown_removed_or_unready_labels() {
        let seed=saved(Some(bounds(1,2,300,200)),false); let state=state(seed); let mut main_window=window("main"); state.begin_direct("main",seed); state.ready_direct("main",bounds(10,20,400,300));
        main_window.native.read_result=Ok(saved(Some(bounds(50,60,410,310)),false)); window_persistence_linux::actual::capture_window(&main_window,&state).unwrap();
        main_window.native.read_result=Ok(saved(Some(bounds(50,60,420,320)),false)); window_persistence_linux::actual::capture_window(&main_window,&state).unwrap();
        let reads=main_window.native.calls.lock().unwrap().clone(); assert_eq!(reads,vec![format!("read:{:?}",bounds(10,20,400,300)),format!("read:{:?}",bounds(50,60,410,310))]); assert_eq!(state.current("main"),Some(saved(Some(bounds(50,60,420,320)),false)));
        let unknown=window("unknown"); window_persistence_linux::actual::capture_window(&unknown,&state).unwrap(); assert!(unknown.native.calls.lock().unwrap().is_empty());
        let unready=window("unready"); state.begin_direct("unready",SavedWindowState::default()); window_persistence_linux::actual::capture_window(&unready,&state).unwrap(); assert!(unready.native.calls.lock().unwrap().is_empty());
        state.remove_window_state("main"); window_persistence_linux::actual::capture_window(&main_window,&state).unwrap(); assert_eq!(main_window.native.calls.lock().unwrap().len(),2);
    }
    #[test] fn native_read_error_preserves_bounds_and_maximized_sample_ends_suppression() {
        let original=saved(Some(bounds(-100,30,800,600)),true); let state=state(original); let mut window=window("main"); let constructor=bounds(140,90,1000,700); state.begin_direct("main",original); state.ready_direct("main",constructor);
        window.native.read_result=Err("native read failed".into()); assert_eq!(window_persistence_linux::actual::capture_window(&window,&state),Err(AppError::Native("native read failed".into()))); assert_eq!(state.current("main"),Some(saved(original.bounds,false)));
        window.native.read_result=Ok(saved(Some(bounds(0,0,1920,1080)),true)); window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current("main"),Some(saved(original.bounds,false)));
        window.native.read_result=Ok(saved(Some(constructor),false)); window_persistence_linux::actual::capture_window(&window,&state).unwrap(); assert_eq!(state.current("main"),Some(saved(Some(constructor),false))); assert_eq!(*state.save_calls.lock().unwrap(),0);
    }
    #[test] fn duplicate_readiness_does_not_reset_an_existing_capture_or_peer() {
        let first_seed=saved(Some(bounds(5,6,700,500)),false); let second_seed=saved(Some(bounds(50,60,800,600)),false); let state=state(first_seed); let main_window=window("main"); state.begin_direct("peer",second_seed);
        let initial=bounds(100,110,900,650); state.begin_direct("main",first_seed); let mut first=Box::pin(window_persistence_linux::actual::enable_capture(&main_window,&state,initial)); main_window.native.barrier.complete(Ok(())); assert_eq!(poll(first.as_mut()),Poll::Ready(Ok(()))); drop(first);
        let updated=bounds(100,110,1100,750); let mut sampled=main_window.clone(); sampled.native.read_result=Ok(saved(Some(updated),false)); window_persistence_linux::actual::capture_window(&sampled,&state).unwrap(); assert_eq!(state.current("main"),Some(saved(Some(updated),false)));
        let replacement=bounds(900,800,400,300); let mut second=Box::pin(window_persistence_linux::actual::enable_capture(&main_window,&state,replacement)); assert_eq!(poll(second.as_mut()),Poll::Ready(Ok(()))); drop(second);
        assert_eq!(state.current("main"),Some(saved(Some(updated),false))); assert_eq!(state.window_sampling_bounds("main"),Some(updated)); assert_eq!(state.current("peer"),Some(saved(second_seed.bounds,false))); assert_eq!(*state.save_calls.lock().unwrap(),0);
    }
    #[test] fn missing_native_bounds_preserve_normal_cache_for_either_maximized_flag() {
        let normal=bounds(-500,25,950,675); let constructor=bounds(80,90,1000,700);
        for maximized in [false,true] {
            let state=state(saved(Some(normal),false)); let mut window=window(if maximized { "maximized" } else { "normal" }); state.begin_direct(window.label(),saved(Some(normal),false));
            let mut ready=Box::pin(window_persistence_linux::actual::enable_capture(&window,&state,constructor)); window.native.barrier.complete(Ok(())); assert_eq!(poll(ready.as_mut()),Poll::Ready(Ok(()))); drop(ready);
            window.native.read_result=Ok(saved(None,maximized)); window_persistence_linux::actual::capture_window(&window,&state).unwrap();
            assert_eq!(state.current(window.label()),Some(saved(Some(normal),false))); assert_eq!(*state.save_calls.lock().unwrap(),0);
        }
    }
}`)
        const run = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml')], { encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
        process.stdout.write(run.stdout ?? '')
        process.stderr.write(run.stderr ?? '')
        exitCode = run.status ?? 1
        if (run.error) throw run.error
    }
} catch (error) {
    console.error(error)
    exitCode = 1
} finally {
    if (temporary) fs.rmSync(temporary, { recursive: true, force: true })
    for (const file of guardedFiles) {
        const current = crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
        if (current !== fingerprints.get(file)) { console.error(`Guarded source changed during test: ${file}`); exitCode = 1 }
    }
}
process.exitCode = exitCode
