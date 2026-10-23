import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const files = [
    'src-tauri/src/window_presentation_linux.rs',
    'src-tauri/src/window_geometry_initialization.rs',
    'src-tauri/src/window_geometry_persistence.rs',
    'src-tauri/Cargo.lock',
].map(file => path.join(root, file))
const hashes = new Map(files.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
let temporary
let exitCode = 0
try {
    if (process.argv.length === 3 && ['--help', '-h'].includes(process.argv[2])) {
        console.log('Usage: node scripts/test-linux-window-presentation.mjs [--help|-h]\nCompiles the actual Linux first-page presentation controller, initialization registry, and saved-window state model in an owned temporary Rust crate. Tauri calls, config, AppState, and native barriers are simulated. Runs cargo test --offline --locked; regressions return nonzero. Removes its temporary crate.')
    } else {
        assert.equal(process.argv.length, 2, 'unknown arguments are rejected; use --help')
        const lock = fs.readFileSync(files.at(-1), 'utf8')
        const version = name => {
            const match = lock.match(new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = "([^"]+)"`))
            assert.ok(match, `Cargo.lock contains ${name}`)
            return match[1]
        }
        temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-linux-window-presentation-'))
        fs.mkdirSync(path.join(temporary, 'src'))
        fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname = "tabby-linux-window-presentation-fixture"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\ntokio = { version = "=${version('tokio')}", features = ["macros", "rt-multi-thread", "sync", "time"] }\nserde = { version = "=${version('serde')}", features = ["derive"] }\nserde_json = "=${version('serde_json')}"\n`)
        const p = Object.fromEntries(files.map(file => [path.basename(file), file]))
        fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Wake, Waker}};
#[derive(Debug, Clone, PartialEq, Eq)] pub enum AppError { Io(String), InvalidArgument(String) }
impl std::fmt::Display for AppError { fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{write!(f,"{self:?}")} }
mod error { pub use crate::AppError; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Theme { Light, Dark }
#[derive(Clone)] pub struct WebviewWindow { label:String, events:Arc<Mutex<Vec<String>>>, state:Arc<state::AppState>, errors:Arc<Mutex<Vec<String>>>, barrier_calls:Arc<Mutex<usize>> }
impl WebviewWindow {
 fn new(label:&str,state:Arc<AppState>)->Self{Self{label:label.into(),events:Arc::default(),state,errors:Arc::default(),barrier_calls:Arc::new(Mutex::new(0))}}
 fn label(&self)->&str{&self.label}
 fn maybe_error(&self,op:&str)->Result<(),String>{if self.errors.lock().unwrap().iter().any(|e|e==op){Err(op.into())}else{Ok(())}}
 pub fn set_theme(&self,_t:Option<Theme>)->Result<(),String>{panic!("Linux presentation must not set a theme")}
 pub fn maximize(&self)->Result<(),String>{self.events.lock().unwrap().push("maximize".into());self.maybe_error("maximize")}
 pub fn show(&self)->Result<(),String>{self.events.lock().unwrap().push("show".into());self.maybe_error("show")}
 pub fn set_focus(&self)->Result<(),String>{self.events.lock().unwrap().push("focus".into());self.maybe_error("focus")}
}
mod launch { use std::{collections::HashSet,sync::Mutex}; #[derive(Default)] pub struct LaunchQueue(Mutex<HashSet<String>>); impl LaunchQueue { pub fn is_closing(&self,l:&str)->bool{self.0.lock().unwrap().contains(l)} pub fn set_closing(&self,l:&str,v:bool){let mut s=self.0.lock().unwrap();if v{s.insert(l.into());}else{s.remove(l);}} } }
mod window_geometry { #[path=${JSON.stringify(p['window_geometry_persistence.rs'])}] pub mod persistence; }
use window_geometry::persistence::{SavedWindowState,WindowStates};
#[derive(Clone)] struct ManualBarrier(Arc<Mutex<BarrierState>>); struct BarrierState{result:Option<Result<(),AppError>>,waiters:Vec<Waker>}
impl Default for ManualBarrier{fn default()->Self{Self(Arc::new(Mutex::new(BarrierState{result:Some(Ok(())),waiters:Vec::new()})))}}
impl ManualBarrier{fn block(&self){self.0.lock().unwrap().result=None;} fn complete_with(&self,r:Result<(),AppError>){let w={let mut s=self.0.lock().unwrap();s.result=Some(r);std::mem::take(&mut s.waiters)};for w in w{w.wake()}} fn complete(&self){self.complete_with(Ok(()))}}
struct BarrierWait(ManualBarrier); impl Future for BarrierWait{type Output=Result<(),AppError>;fn poll(self:Pin<&mut Self>,cx:&mut Context<'_>)->Poll<Self::Output>{let mut s=self.0.0.lock().unwrap();if let Some(r)=s.result.clone(){Poll::Ready(r)}else{if !s.waiters.iter().any(|w|w.will_wake(cx.waker())){s.waiters.push(cx.waker().clone())}Poll::Pending}}}
mod platform {use crate::{AppError,BarrierWait,WebviewWindow};pub fn wait_for_window_operations(w:&WebviewWindow)->BarrierWait{w.events.lock().unwrap().push("barrier".into());let mut n=w.barrier_calls.lock().unwrap();*n+=1;let marker=format!("barrier:{}",*n);let errors=w.errors.lock().unwrap();if errors.iter().any(|e|e==&format!("pause_{marker}")){w.state.barrier.block();BarrierWait(w.state.barrier.clone())}else if errors.iter().any(|e|e==&marker){let b=crate::ManualBarrier::default();b.complete_with(Err(AppError::Io(marker)));BarrierWait(b)}else{BarrierWait(w.state.barrier.clone())}} }
mod commands {pub mod app {use crate::{state::AppState,AppError};use std::collections::BTreeMap;pub fn startup_window_config(_: &AppState)->Result<BTreeMap<String,serde_json::Value>,AppError>{panic!("Linux presentation must not read startup appearance config")}}}
mod state {use super::*;pub struct AppState{pub(crate) states:Mutex<WindowStates>,pub(crate) queue:launch::LaunchQueue,pub(crate) ops:tokio::sync::Mutex<()>,pub(crate) hidden:Mutex<bool>,pub(crate) barrier:ManualBarrier}impl AppState{pub(crate) fn new()->Self{Self{states:Mutex::new(WindowStates::default()),queue:Default::default(),ops:Default::default(),hidden:Mutex::new(false),barrier:Default::default()}}pub(crate) fn startup_window_maximized(&self,l:&str)->Option<bool>{self.states.lock().unwrap().startup_maximized(l)}pub(crate) fn startup_window_hidden(&self,l:&str)->bool{l=="main"&&*self.hidden.lock().unwrap()}pub(crate) fn launches(&self)->&launch::LaunchQueue{&self.queue}pub(crate) async fn lock_window_operations(&self)->tokio::sync::MutexGuard<'_,()>{self.ops.lock().await}}}
use state::AppState;
mod initialization {#[path=${JSON.stringify(p['window_geometry_initialization.rs'])}]pub mod actual;}
use initialization::actual::Initializations;
#[path=${JSON.stringify(p['window_presentation_linux.rs'])}]mod controller;
#[cfg(test)] mod tests {use super::*;use std::time::Duration;struct N;impl Wake for N{fn wake(self:Arc<Self>){}}fn poll<F:Future>(f:Pin<&mut F>)->Poll<F::Output>{f.poll(&mut Context::from_waker(&Waker::from(Arc::new(N))))}
fn seeded(label:&str,maximized:bool)->(Arc<AppState>,WebviewWindow){let s=Arc::new(AppState::new());s.states.lock().unwrap().begin(label,SavedWindowState{bounds:None,maximized});(s.clone(),WebviewWindow::new(label,s))}
#[tokio::test]async fn normal_first_presentation_waits_then_shows_and_focuses(){let(s,w)=seeded("main",false);assert_eq!(controller::present_window(&w,&s).await,Ok(()));assert_eq!(*w.events.lock().unwrap(),vec!["barrier","show","focus","barrier"]);}
#[tokio::test]async fn hidden_main_waits_but_never_maximizes_shows_or_focuses(){let(s,w)=seeded("main",true);*s.hidden.lock().unwrap()=true;assert_eq!(controller::present_window(&w,&s).await,Ok(()));assert_eq!(*w.events.lock().unwrap(),vec!["barrier"]);}
#[tokio::test]async fn hidden_flag_is_limited_to_main_window(){let(s,main)=seeded("main",true);s.states.lock().unwrap().begin("window-1",SavedWindowState{bounds:None,maximized:true});let other=WebviewWindow::new("window-1",s.clone());*s.hidden.lock().unwrap()=true;controller::present_window(&main,&s).await.unwrap();controller::present_window(&other,&s).await.unwrap();assert!(!main.events.lock().unwrap().contains(&"show".into()));assert!(other.events.lock().unwrap().contains(&"show".into()));}
#[tokio::test]async fn maximized_window_restores_maximization_before_show_and_focus(){let(s,w)=seeded("main",true);assert_eq!(controller::present_window(&w,&s).await,Ok(()));assert_eq!(*w.events.lock().unwrap(),vec!["barrier","maximize","barrier","show","focus","barrier"]);}
#[tokio::test]async fn unknown_closing_and_removed_labels_are_noops(){let s=Arc::new(AppState::new());let unknown=WebviewWindow::new("unknown",s.clone());controller::present_window(&unknown,&s).await.unwrap();let(s2,closing)=seeded("closing",false);s2.queue.set_closing("closing",true);controller::present_window(&closing,&s2).await.unwrap();s2.states.lock().unwrap().remove("closing");let removed=WebviewWindow::new("closing",s2.clone());controller::present_window(&removed,&s2).await.unwrap();assert!(unknown.events.lock().unwrap().is_empty());assert!(closing.events.lock().unwrap().is_empty());assert!(removed.events.lock().unwrap().is_empty());}
#[tokio::test]async fn native_errors_short_circuit_at_maximize_show_focus_and_each_barrier(){let cases:Vec<(&str,bool,Vec<&str>)>=vec![("barrier:1",false,vec!["barrier"]),("maximize",true,vec!["barrier","maximize"]),("barrier:2",true,vec!["barrier","maximize","barrier"]),("show",false,vec!["barrier","show"]),("focus",false,vec!["barrier","show","focus"]),("barrier:2",false,vec!["barrier","show","focus","barrier"]),("barrier:3",true,vec!["barrier","maximize","barrier","show","focus","barrier"])];for(op,max,expected)in cases{let(s,w)=seeded("main",max);w.errors.lock().unwrap().push(op.into());assert_eq!(controller::present_window(&w,&s).await,Err(AppError::Io(op.into())),"{op}");assert_eq!(*w.events.lock().unwrap(),expected,"{op}");}}
#[tokio::test]async fn closing_during_initial_barrier_prevents_maximize_show_and_focus(){let(s,w)=seeded("main",true);w.errors.lock().unwrap().push("pause_barrier:1".into());let mut f=Box::pin(controller::present_window(&w,&s));assert!(poll(f.as_mut()).is_pending());s.queue.set_closing("main",true);s.barrier.complete();assert_eq!(tokio::time::timeout(Duration::from_millis(100),f).await.unwrap(),Ok(()));assert_eq!(*w.events.lock().unwrap(),vec!["barrier"]);}
#[tokio::test]async fn close_during_maximize_barrier_prevents_show_and_focus(){let(s,w)=seeded("main",true);w.errors.lock().unwrap().push("pause_barrier:2".into());let mut f=Box::pin(controller::present_window(&w,&s));assert!(poll(f.as_mut()).is_pending());assert_eq!(*w.events.lock().unwrap(),vec!["barrier","maximize","barrier"]);s.queue.set_closing("main",true);s.barrier.complete();assert_eq!(tokio::time::timeout(Duration::from_millis(100),f).await.unwrap(),Ok(()));let e=w.events.lock().unwrap().clone();assert!(!e.contains(&"show".into()));assert!(!e.contains(&"focus".into()));}
#[tokio::test]async fn removal_during_initial_barrier_is_safe(){let(s,w)=seeded("main",false);w.errors.lock().unwrap().push("pause_barrier:1".into());let mut f=Box::pin(controller::present_window(&w,&s));assert!(poll(f.as_mut()).is_pending());s.states.lock().unwrap().remove("main");s.barrier.complete();assert_eq!(tokio::time::timeout(Duration::from_millis(100),f).await.unwrap(),Ok(()));assert!(!w.events.lock().unwrap().contains(&"show".into()));}
#[tokio::test]async fn removal_during_maximize_barrier_prevents_show_and_focus(){let(s,w)=seeded("main",true);w.errors.lock().unwrap().push("pause_barrier:2".into());let mut f=Box::pin(controller::present_window(&w,&s));assert!(poll(f.as_mut()).is_pending());assert_eq!(*w.events.lock().unwrap(),vec!["barrier","maximize","barrier"]);s.states.lock().unwrap().remove("main");s.barrier.complete();assert_eq!(tokio::time::timeout(Duration::from_millis(100),f).await.unwrap(),Ok(()));let e=w.events.lock().unwrap().clone();assert!(!e.contains(&"show".into()));assert!(!e.contains(&"focus".into()));}
#[tokio::test]async fn operation_mutex_blocks_native_work_until_released(){let(s,w)=seeded("main",false);let held=s.ops.lock().await;let mut f=Box::pin(controller::present_window(&w,&s));assert!(poll(f.as_mut()).is_pending());assert!(w.events.lock().unwrap().is_empty());drop(held);assert_eq!(tokio::time::timeout(Duration::from_millis(100),f).await.unwrap(),Ok(()));assert!(w.events.lock().unwrap().contains(&"show".into()));}
#[tokio::test]async fn close_claim_while_waiting_for_operation_mutex_skips_presentation(){let(s,w)=seeded("main",true);let held=s.ops.lock().await;let mut f=Box::pin(controller::present_window(&w,&s));assert!(poll(f.as_mut()).is_pending());assert!(w.events.lock().unwrap().is_empty());s.queue.set_closing("main",true);drop(held);assert_eq!(tokio::time::timeout(Duration::from_millis(100),f).await.unwrap(),Ok(()));assert!(w.events.lock().unwrap().is_empty());}
#[test]fn real_window_state_model_keeps_maximized_snapshot_and_removes_label(){let mut m=WindowStates::default();assert!(m.begin("a",SavedWindowState{bounds:None,maximized:true}));assert_eq!(m.startup_maximized("a"),Some(true));m.remove("a");assert_eq!(m.startup_maximized("a"),None);}
#[tokio::test]async fn real_initialization_registry_claims_first_page_once_and_rejects_failed_or_removed_tickets(){let r=Initializations::default();assert!(r.first_page_load("x").is_none());let t=r.begin("x").unwrap();let claimed=r.first_page_load("x").unwrap();assert!(Arc::ptr_eq(&t,&claimed));assert!(r.first_page_load("x").is_none());t.complete(Err("failed".into()));assert!(!r.can_present("x"));assert_eq!(claimed.wait().await.unwrap_err(),"failed");r.remove("x");assert!(r.first_page_load("x").is_none());let fresh=r.begin("x").unwrap();let ticket=r.first_page_load("x").unwrap();fresh.complete(Ok(()));assert!(r.can_present("x"));assert!(ticket.wait().await.is_ok());}
}`)
        const manifest = path.join(temporary, 'Cargo.toml')
        const generatedLock = spawnSync('cargo', ['generate-lockfile', '--offline', '--manifest-path', manifest], { cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 4 * 1024 * 1024 })
        if (generatedLock.error) throw generatedLock.error
        assert.equal(generatedLock.status, 0, `${generatedLock.stdout}\n${generatedLock.stderr}`)
        const result = spawnSync('cargo', ['test', '--offline', '--locked', '--manifest-path', manifest, '--', '--nocapture'], { cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
        if (result.error) throw result.error
        assert.equal(result.signal, null, result.stderr)
        process.stdout.write(`${result.stdout}\n${result.stderr}`)
        exitCode = result.status ?? 1
    }
} catch (error) {
    console.error(error)
    exitCode = 1
} finally {
    try {
        for (const file of files) assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), hashes.get(file), `input unchanged: ${file}`)
    } catch (error) { console.error(error); exitCode = 1 }
    if (temporary) fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
}
process.exitCode = exitCode
