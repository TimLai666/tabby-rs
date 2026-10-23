import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

if (process.argv.includes('--help')) {
    console.log('Usage: node scripts/test-macos-window-close.mjs\nCompiles the shared macOS/Linux close controller and LaunchQueue methods in an owned temporary Rust crate using the locked Tokio version. OS, Tauri framework, and AppState I/O boundaries are simulated. Requires Cargo and cached dependencies. Removes owned temporary files.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'unknown arguments; use --help')

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const controllerPath = path.join(root, 'src-tauri/src/window_close.rs')
const launchPath = path.join(root, 'src-tauri/src/launch.rs')
const lockPath = path.join(root, 'src-tauri/Cargo.lock')
const files = [controllerPath, launchPath, lockPath]
const fingerprints = new Map(files.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
const launch = fs.readFileSync(launchPath, 'utf8')
const queueStart = launch.indexOf('#[derive(Default)]\npub struct LaunchQueue')
const queueEnd = launch.indexOf('\npub fn window_creation_order', queueStart)
assert.ok(queueStart >= 0 && queueEnd > queueStart, 'complete production LaunchQueue declaration and implementation')
const queue = launch.slice(queueStart, queueEnd)
const lock = fs.readFileSync(lockPath, 'utf8')
const tokio = lock.match(/\[\[package\]\]\r?\nname = "tokio"\r?\nversion = "([^"]+)"/)
assert.ok(tokio, 'locked Tokio version')
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-macos-close-'))
try {
    fs.mkdirSync(path.join(temporary, 'src'))
    fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname = "tabby-macos-window-close-fixture"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\ntokio = { version = "=${tokio[1]}", features = ["macros", "rt-multi-thread", "sync", "time"] }\n`)
    fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{collections::{HashMap, HashSet, VecDeque}, future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Wake, Waker}, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError { Io(String), Unsupported(String) }
mod error { pub use crate::AppError; }
#[derive(Clone)]
pub struct WebviewWindow { label: String, events: Arc<Mutex<Vec<String>>>, destroy: Arc<Mutex<Result<(), String>>>, destroyed: Arc<Mutex<bool>>, state: Arc<AppState> }
impl WebviewWindow { pub fn new(label: &str, state: Arc<AppState>) -> Self { Self { label: label.into(), events: Arc::default(), destroy: Arc::new(Mutex::new(Ok(()))), destroyed: Arc::new(Mutex::new(false)), state } } pub fn label(&self) -> &str { &self.label } pub fn destroy(&self) -> Result<(), String> { let result = self.destroy.lock().unwrap().clone(); if result.is_ok() { *self.destroyed.lock().unwrap() = true; } result } }
trait Emitter { fn emit_to(&self, target: &str, event: &str, payload: ()) -> Result<(), String>; }
impl Emitter for WebviewWindow { fn emit_to(&self, target: &str, event: &str, _: ()) -> Result<(), String> { self.events.lock().unwrap().push(format!("{target}:{event}")); Ok(()) } }

mod launch {
    use std::{collections::{HashMap, HashSet, VecDeque}, sync::Mutex};
    #[derive(Clone, Debug)] pub struct LaunchContext(pub &'static str);
    ${queue}
}
use launch::{LaunchContext, LaunchQueue};

#[derive(Clone, Copy, Debug)] enum ReadMode { Value(bool), Error }
#[derive(Clone, Copy, Debug)] enum BarrierMode { Ready, Error, Pending }
pub struct AppState { queue: LaunchQueue, window_operations: tokio::sync::Mutex<()>, read: Mutex<ReadMode>, barrier: Mutex<BarrierMode>, saves: Mutex<Vec<(String, bool)>>, save_error: Mutex<bool>, reads: Mutex<usize>, saves_attempted: Mutex<usize>, missing_origin: Mutex<bool>, origins_seen: Mutex<Vec<window_geometry::persistence::LogicalBounds>> }
impl AppState {
 fn new() -> Self { Self { queue: LaunchQueue::default(), window_operations: tokio::sync::Mutex::new(()), read: Mutex::new(ReadMode::Value(false)), barrier: Mutex::new(BarrierMode::Ready), saves: Mutex::new(Vec::new()), save_error: Mutex::new(false), reads: Mutex::new(0), saves_attempted: Mutex::new(0), missing_origin: Mutex::new(false), origins_seen: Mutex::new(Vec::new()) } }
 fn window_sampling_bounds(&self, _: &str) -> Option<window_geometry::persistence::LogicalBounds> { if *self.missing_origin.lock().unwrap() { None } else { Some(origin()) } }
 fn launches(&self) -> &LaunchQueue { &self.queue }
 async fn lock_window_operations(&self) -> tokio::sync::MutexGuard<'_, ()> { self.window_operations.lock().await }
 fn save_window_state(&self, label: &str, maximized: bool) -> Result<(), AppError> { *self.saves_attempted.lock().unwrap() += 1; if *self.save_error.lock().unwrap() { return Err(AppError::Io("save".into())); } self.saves.lock().unwrap().push((label.into(), maximized)); Ok(()) }
}
mod state { pub use crate::AppState; }
mod window_geometry { pub mod persistence {
 #[derive(Clone,Copy,Debug,PartialEq,Eq)] pub struct LogicalBounds { pub x:i32,pub y:i32,pub width:u32,pub height:u32 }
} }
fn origin() -> window_geometry::persistence::LogicalBounds { window_geometry::persistence::LogicalBounds{x:31,y:-17,width:700,height:450} }
mod platform {
 use crate::{AppError, WebviewWindow};
 pub async fn wait_for_window_operations(window: &WebviewWindow) -> Result<(), AppError> {
   let mode = *window.state.barrier.lock().unwrap();
   match mode { super::BarrierMode::Ready => Ok(()), super::BarrierMode::Error => Err(AppError::Io("barrier".into())), super::BarrierMode::Pending => std::future::pending().await }
 }
 pub mod window_state_linux {
  pub fn read(window:&crate::WebviewWindow, origin:crate::window_geometry::persistence::LogicalBounds)->Result<super::window_state_macos::NativeSample,crate::AppError> {
   window.state.origins_seen.lock().unwrap().push(origin);
   super::window_state_macos::read(window)
  }
 }
 pub mod window_state_macos {
  use crate::{AppError, AppState, WebviewWindow};
  pub struct NativeSample { pub maximized: bool }
  pub fn read(window: &WebviewWindow) -> Result<NativeSample, AppError> { let state = &window.state; *state.reads.lock().unwrap() += 1; match *state.read.lock().unwrap() { super::super::ReadMode::Value(value) => Ok(NativeSample { maximized: value }), super::super::ReadMode::Error => Err(AppError::Io("read".into())) } }
 }
}
#[path = ${JSON.stringify(controllerPath)}]
mod controller;
#[cfg(test)]
mod tests {
 use super::*; use controller::close_window;
 #[cfg(target_os="linux")]
 #[tokio::test] async fn linux_close_passes_latest_logical_origin_to_native_reader(){let s=state();let w=WebviewWindow::new("a",s.clone());close_window(&w,&s).await.unwrap();assert_eq!(*s.origins_seen.lock().unwrap(),vec![origin()]);}
 #[cfg(target_os="linux")]
 #[tokio::test] async fn linux_unregistered_close_destroys_without_sampling_or_saving(){let s=state();*s.missing_origin.lock().unwrap()=true;let w=WebviewWindow::new("a",s.clone());close_window(&w,&s).await.unwrap();assert!(*w.destroyed.lock().unwrap());assert_eq!(*s.reads.lock().unwrap(),0);assert_eq!(*s.saves_attempted.lock().unwrap(),0);assert!(s.origins_seen.lock().unwrap().is_empty());}
 struct Noop; impl Wake for Noop { fn wake(self: Arc<Self>) {} }
 fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> { let waker=Waker::from(Arc::new(Noop)); future.poll(&mut Context::from_waker(&waker)) }
 fn state()->Arc<AppState>{Arc::new(AppState::new())} fn ctx(v:&'static str)->LaunchContext{LaunchContext(v)}
 #[tokio::test] async fn queue_claim_is_atomic_and_release_preserves_pending(){let q=LaunchQueue::default();assert!(q.push("a",ctx("one")));assert!(q.push("peer",ctx("peer")));assert!(q.begin_close("a"));assert!(!q.begin_close("a"));assert!(!q.push("a",ctx("two")));assert!(q.take("a").is_none());assert_eq!(q.take("peer").unwrap().0,"peer");q.set_closing("a",false);assert_eq!(q.take("a").unwrap().0,"one");assert!(q.take("a").is_none());}
 #[tokio::test] async fn close_saves_live_true_and_false_and_does_not_replay(){for value in [false,true]{let s=state();let w=WebviewWindow::new("a",s.clone());*s.read.lock().unwrap()=ReadMode::Value(value);assert!(s.launches().push("a",ctx("kept")));assert!(s.launches().push("peer",ctx("peer")));assert!(close_window(&w,&s).await.is_ok());assert_eq!(*s.saves.lock().unwrap(),vec![("a".into(),value)]);assert!(*w.destroyed.lock().unwrap());assert!(w.events.lock().unwrap().is_empty());assert!(s.launches().take("a").is_none());assert_eq!(s.launches().take("peer").unwrap().0,"peer");}}
 #[tokio::test] async fn duplicate_during_pending_is_noop(){let s=state();let w=WebviewWindow::new("a",s.clone());*s.barrier.lock().unwrap()=BarrierMode::Pending;let mut f=Box::pin(close_window(&w,&s));assert!(poll_once(f.as_mut()).is_pending());let reads=*s.reads.lock().unwrap();assert!(close_window(&w,&s).await.is_ok());assert_eq!(*s.reads.lock().unwrap(),reads);assert!(!*w.destroyed.lock().unwrap());}
 #[tokio::test] async fn read_and_save_errors_do_not_block_destroy(){let s=state();let w=WebviewWindow::new("a",s.clone());*s.read.lock().unwrap()=ReadMode::Error;assert!(close_window(&w,&s).await.is_ok());assert!(*w.destroyed.lock().unwrap());assert_eq!(*s.saves_attempted.lock().unwrap(),0);let s=state();let w=WebviewWindow::new("b",s.clone());*s.save_error.lock().unwrap()=true;assert!(close_window(&w,&s).await.is_ok());assert!(*w.destroyed.lock().unwrap());assert_eq!(*s.saves_attempted.lock().unwrap(),1);}
 #[tokio::test] async fn barrier_and_destroy_errors_restore_once_and_allow_retry(){let s=state();let w=WebviewWindow::new("a",s.clone());*s.barrier.lock().unwrap()=BarrierMode::Error;assert_eq!(close_window(&w,&s).await,Err(AppError::Io("barrier".into())));assert!(!s.launches().is_closing("a"));assert_eq!(*w.events.lock().unwrap(),vec!["a:app:launch"]);*s.barrier.lock().unwrap()=BarrierMode::Ready;*w.destroy.lock().unwrap()=Err("destroy".into());assert_eq!(close_window(&w,&s).await,Err(AppError::Io("destroy".into())));assert!(!s.launches().is_closing("a"));assert_eq!(w.events.lock().unwrap().len(),2);*w.destroy.lock().unwrap()=Ok(());assert!(close_window(&w,&s).await.is_ok());assert_eq!(w.events.lock().unwrap().len(),2);}
 #[tokio::test] async fn cancellation_while_waiting_for_mutex_or_barrier_restores_claim(){let s=state();let w=WebviewWindow::new("a",s.clone());let held=s.window_operations.lock().await;let mut f=Box::pin(close_window(&w,&s));assert!(poll_once(f.as_mut()).is_pending());drop(f);drop(held);assert!(!s.launches().is_closing("a"));assert_eq!(w.events.lock().unwrap().len(),1);*s.barrier.lock().unwrap()=BarrierMode::Pending;let mut f=Box::pin(close_window(&w,&s));assert!(poll_once(f.as_mut()).is_pending());drop(f);assert!(!s.launches().is_closing("a"));assert_eq!(w.events.lock().unwrap().len(),2);}
 #[tokio::test] async fn real_mutex_serializes_close_and_geometry_but_isolated_from_peer(){let s=state();let w=WebviewWindow::new("a",s.clone());let guard=s.window_operations.lock().await;let mut f=Box::pin(close_window(&w,&s));assert!(poll_once(f.as_mut()).is_pending());assert_eq!(*s.reads.lock().unwrap(),0);drop(guard);assert!(f.await.is_ok());let peer=state();let wp=WebviewWindow::new("peer",peer.clone());let guard=s.window_operations.lock().await;assert!(tokio::time::timeout(Duration::from_millis(50),close_window(&wp,&peer)).await.unwrap().is_ok());drop(guard);}
 #[tokio::test]
 async fn windows_share_geometry_lock_but_keep_separate_close_claims() {
  let s=state(); let first=WebviewWindow::new("a",s.clone()); let peer=WebviewWindow::new("peer",s.clone());
  let held=s.window_operations.lock().await;
  let mut a=Box::pin(close_window(&first,&s)); let mut b=Box::pin(close_window(&peer,&s));
  assert!(poll_once(a.as_mut()).is_pending()); assert!(poll_once(b.as_mut()).is_pending());
  assert!(s.launches().is_closing("a")); assert!(s.launches().is_closing("peer"));
  assert_eq!(*s.reads.lock().unwrap(),0); assert!(s.saves.lock().unwrap().is_empty());
  drop(held); assert!(a.await.is_ok()); assert!(b.await.is_ok());
  assert_eq!(*s.saves.lock().unwrap(),vec![("a".into(),false),("peer".into(),false)]);
  assert!(*first.destroyed.lock().unwrap()); assert!(*peer.destroyed.lock().unwrap());
  assert!(first.events.lock().unwrap().is_empty()); assert!(peer.events.lock().unwrap().is_empty());
 }
}
`)
    const result = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml'), '--', '--nocapture'], { cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
    if (result.error) throw result.error
    assert.equal(result.signal, null, result.stderr)
    const output = `${result.stdout}\n${result.stderr}`
    process.stdout.write(output)
    assert.equal(result.status, 0, 'production close controller and queue behavior must pass')
    assert.match(output, new RegExp(`test result: ok\\. ${process.platform === 'linux' ? 10 : 8} passed; 0 failed`), 'all close behavior checks executed')
} finally {
    try {
        for (const file of files) assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), fingerprints.get(file), 'production fixture inputs stayed unchanged')
    } finally {
        fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    }
}
