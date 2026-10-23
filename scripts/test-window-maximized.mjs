import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const predicate = path.join(root, 'src-tauri/src/platform/window_maximized.rs')
const desktop = path.join(root, 'src-tauri/src/commands/desktop.rs')
const lock = path.join(root, 'src-tauri/Cargo.lock')
const topmost = path.join(root, 'src-tauri/src/platform/window_topmost.rs')
const topmostLinux = path.join(root, 'src-tauri/src/platform/window_topmost_linux.rs')
const inputs = [predicate, topmost, topmostLinux, desktop, lock]
const hashes = new Map(inputs.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
let temporary
try {
    if (process.argv.length === 3 && ['--help', '-h'].includes(process.argv[2])) {
        console.log('Usage: node scripts/test-window-maximized.mjs [--help]\nTests the actual maximized predicate and extracted public snapshot/toggle methods in an owned temporary Rust crate. Native/Tauri boundaries are simulated; this does not establish desktop, IPC serialization, or native platform acceptance. Runs Cargo offline with locked dependency versions; unknown arguments fail. Removes its temporary crate.')
    } else {
        assert.equal(process.argv.length, 2, 'unknown argument; use --help')
        const source = fs.readFileSync(desktop, 'utf8')
        const extract = signature => {
            const start = source.indexOf(signature)
            assert.ok(start >= 0 && source.indexOf(signature, start + 1) < 0, `unique ${signature}`)
            const body = source.indexOf('{', start)
            let depth = 0
            for (let index = body; index < source.length; index++) {
                if (source[index] === '{') depth++
                else if (source[index] === '}' && --depth === 0) return source.slice(start, index + 1)
            }
            throw new Error(`incomplete ${signature}`)
        }
        const snapshot = extract('fn window_state(')
        const toggle = extract('pub async fn window_toggle_maximize(').replace('#[cfg(target_os = "linux")]', '#[cfg(any(target_os = "linux", feature = "fixture-linux"))]')
        assert.equal(toggle.includes('#[cfg(target_os = "linux")]'), false)
        const tokioVersion = fs.readFileSync(lock, 'utf8').match(/\[\[package\]\]\r?\nname = "tokio"\r?\nversion = "([^"]+)"/)
        assert.ok(tokioVersion)
        const version = fs.readFileSync(lock, 'utf8').match(/\[\[package\]\]\r?\nname = "serde_json"\r?\nversion = "([^"]+)"/)
        assert.ok(version)
        temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-window-maximized-'))
        fs.mkdirSync(path.join(temporary, 'src'))
        fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname="tabby-window-maximized-fixture"\nversion="0.0.0"\nedition="2021"\n[features]\nfixture-linux=[]\n[dependencies]\nserde_json="=${version[1]}"\ntokio={version="=${tokioVersion[1]}",features=["macros","rt","sync"]}\n`)
        fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{cell::{Cell,RefCell},collections::VecDeque,rc::Rc,sync::Arc};
#[derive(Debug,PartialEq,Eq)] pub enum AppError { Io(String) }
mod error { pub use crate::AppError; }
fn io_error(e:impl std::fmt::Display)->AppError { AppError::Io(e.to_string()) }
struct AppState { operations: Arc<tokio::sync::Mutex<()>> }
impl AppState { async fn lock_window_operations(&self)->tokio::sync::MutexGuard<'_,()>{self.operations.lock().await} }
#[derive(Clone,Copy)] struct Position { x:i32,y:i32 }
#[derive(Clone,Copy)] struct Size { width:u32,height:u32 }
#[derive(Clone)] pub struct WebviewWindow { raw:Rc<Cell<bool>>,native:Rc<Cell<bool>>,minimized:bool,fullscreen:bool,fail:Option<&'static str>,calls:Rc<RefCell<Vec<&'static str>>>,state:Arc<AppState>,barriers:Rc<RefCell<VecDeque<tokio::sync::oneshot::Receiver<Result<(),AppError>>>>>,completion_acks:Rc<RefCell<VecDeque<tokio::sync::oneshot::Receiver<Result<(),AppError>>>>>,completion_requests:Rc<RefCell<Vec<bool>>> }
impl WebviewWindow {
 fn new(raw:bool,native:bool,minimized:bool,fullscreen:bool)->Self{Self{raw:Rc::new(Cell::new(raw)),native:Rc::new(Cell::new(native)),minimized,fullscreen,fail:None,calls:Default::default(),state:Arc::new(AppState{operations:Arc::new(tokio::sync::Mutex::new(()))}),barriers:Default::default(),completion_acks:Default::default(),completion_requests:Default::default()}}
 fn with_state(raw:bool,native:bool,state:Arc<AppState>)->Self{let mut w=Self::new(raw,native,false,false);w.state=state;w}
 fn state<T>(&self)->Arc<AppState>{self.state.clone()}
 fn set_barrier(&self)->tokio::sync::oneshot::Sender<Result<(),AppError>>{let(tx,rx)=tokio::sync::oneshot::channel();self.barriers.borrow_mut().push_back(rx);tx}
 fn set_completion_ack(&self)->tokio::sync::oneshot::Sender<Result<(),AppError>>{let(tx,rx)=tokio::sync::oneshot::channel();self.completion_acks.borrow_mut().push_back(rx);tx}
 fn get(&self,op:&'static str,value:bool)->Result<bool,&'static str>{self.calls.borrow_mut().push(op);if self.fail==Some(op){Err(op)}else{Ok(value)}}
 fn native_max(&self)->Result<bool,AppError>{self.get("native",self.native.get()).map_err(io_error)}
 fn is_maximized(&self)->Result<bool,&'static str>{self.get("raw",self.raw.get())}
 fn is_minimized(&self)->Result<bool,&'static str>{self.get("minimized",self.minimized)}
 fn is_fullscreen(&self)->Result<bool,&'static str>{self.get("fullscreen",self.fullscreen)}
 fn is_visible(&self)->Result<bool,&'static str>{Ok(true)}
 fn is_always_on_top(&self)->Result<bool,&'static str>{self.get("topmost-raw",false)}
 fn set_always_on_top(&self,value:bool)->Result<(),&'static str>{self.get(if value{"topmost-set-true"}else{"topmost-set-false"},true).map(|_|())}
 fn is_focused(&self)->Result<bool,&'static str>{Ok(true)}
 fn outer_position(&self)->Result<Position,&'static str>{Ok(Position{x:11,y:22})}
 fn outer_size(&self)->Result<Size,&'static str>{Ok(Size{width:800,height:600})}
 fn scale_factor(&self)->Result<f64,&'static str>{Ok(1.5)}
 fn maximize(&self)->Result<(),&'static str>{self.get("maximize",true).map(|_|())?;self.raw.set(true);self.native.set(true);Ok(())}
 fn unmaximize(&self)->Result<(),&'static str>{self.get("unmaximize",true).map(|_|())?;self.raw.set(false);self.native.set(false);Ok(())}
}
#[derive(Debug,PartialEq,Eq)] struct WindowBounds{x:i32,y:i32,width:u32,height:u32}
struct WindowStateSnapshot{visible:bool,always_on_top:bool,fullscreen:bool,maximized:bool,minimized:bool,focused:bool,bounds:WindowBounds,scale_factor:f64,capabilities:(),caption_controls:()}
fn window_capabilities(_: &WebviewWindow)->Result<(),AppError>{Ok(())}
mod platform {
 pub mod window_state_linux {pub fn maximized(w:&crate::WebviewWindow)->Result<bool,crate::AppError>{w.native_max()}}
 pub mod window_state_macos {pub fn maximized(w:&crate::WebviewWindow)->Result<bool,crate::AppError>{w.native_max()}}
 pub mod window_topmost_linux {pub fn read(w:&crate::WebviewWindow)->Result<bool,crate::AppError>{w.is_always_on_top().map_err(crate::io_error)} pub fn set(w:&crate::WebviewWindow,v:bool)->Result<(),crate::AppError>{w.set_always_on_top(v).map_err(crate::io_error)}}
 #[path=${JSON.stringify(topmost)}] mod topmost_actual;
 pub(crate) use topmost_actual::read as window_is_always_on_top;
 #[path=${JSON.stringify(predicate)}] mod actual;
 pub(crate) use actual::read as window_is_maximized;
 pub async fn wait_for_window_operations(w:&crate::WebviewWindow)->Result<(),crate::AppError>{w.calls.borrow_mut().push("barrier");let rx=w.barriers.borrow_mut().pop_front();if let Some(rx)=rx{rx.await.map_err(crate::io_error)?}else{Ok(())}}
 pub mod window_completion_linux {pub async fn wait_for_maximized(w:&crate::WebviewWindow,value:bool)->Result<(),crate::AppError>{w.calls.borrow_mut().push("linux-ack");w.completion_requests.borrow_mut().push(value);let rx=w.completion_acks.borrow_mut().pop_front();if let Some(rx)=rx{rx.await.map_err(crate::io_error)?}else{Ok(())}}}
 pub fn window_bounds(_: &crate::WebviewWindow)->Result<(crate::Position,crate::Size),crate::AppError>{Ok((crate::Position{x:11,y:22},crate::Size{width:800,height:600}))}
 pub fn window_caption_controls(_: &crate::WebviewWindow)->Result<(),crate::AppError>{Ok(())}
}
${snapshot}
${toggle}
#[cfg(test)] mod tests {
 use super::*;
 const NATIVE:bool=cfg!(any(target_os="linux",target_os="macos"));
 const LINUX:bool=cfg!(any(target_os="linux",feature="fixture-linux"));
 const CANONICAL_READ:&'static str=if NATIVE{"native"}else{"raw"};
 const ROWS:[(bool,bool,bool,bool);5]=[(false,false,false,false),(true,true,false,false),(true,false,true,false),(true,false,false,true),(false,true,false,false)];
 fn expected(raw:bool,native:bool)->bool{if NATIVE{native}else{raw}}
 #[test] fn predicate_uses_native_host_and_preserves_other_host_fallback(){for (raw,native,min,full) in ROWS {let w=WebviewWindow::new(raw,native,min,full);assert_eq!(platform::window_is_maximized(&w),Ok(expected(raw,native)));let calls=w.calls.borrow();assert_eq!(calls.as_slice(),if NATIVE{&["native"]}else{&["raw"]});}}
 #[test] fn snapshot_uses_canonical_flag_and_retains_other_fields(){for (raw,native,min,full) in ROWS{let w=WebviewWindow::new(raw,native,min,full);let s=window_state(&w).unwrap();assert_eq!(s.maximized,expected(raw,native),"raw={raw} native={native}");assert_eq!((s.minimized,s.fullscreen),(min,full));assert_eq!(s.bounds,WindowBounds{x:11,y:22,width:800,height:600});assert_eq!(s.scale_factor,1.5);assert!(s.visible&&s.focused&&!s.always_on_top);assert_eq!(w.calls.borrow().iter().filter(|c|**c=="native").count(),usize::from(NATIVE));}}
 #[tokio::test] async fn toggle_uses_canonical_flag_for_each_direction(){for (raw,native,min,full) in ROWS{let w=WebviewWindow::new(raw,native,min,full);window_toggle_maximize(w.clone(),serde_json::Value::Null).await.unwrap();let action=if expected(raw,native){"unmaximize"}else{"maximize"};assert_eq!(w.calls.borrow().iter().copied().filter(|c|["maximize","unmaximize"].contains(c)).collect::<Vec<_>>(),vec![action]);}}
 #[tokio::test] async fn native_read_errors_reach_snapshot_and_toggle_without_action(){let op=if NATIVE{"native"}else{"raw"};let mut w=WebviewWindow::new(true,false,true,false);w.fail=Some(op);assert!(matches!(window_state(&w),Err(AppError::Io(e)) if e==op));w.calls.borrow_mut().clear();assert_eq!(window_toggle_maximize(w.clone(),serde_json::Value::Null).await,Err(AppError::Io(op.into())));assert!(!w.calls.borrow().iter().any(|c|["maximize","unmaximize"].contains(c)));}
 #[tokio::test] async fn toggle_preserves_native_action_errors(){for value in [false,true]{let mut w=WebviewWindow::new(value,value,false,false);let op=if value{"unmaximize"}else{"maximize"};w.fail=Some(op);assert_eq!(window_toggle_maximize(w.clone(),serde_json::Value::Null).await,Err(AppError::Io(op.into())));assert_eq!(w.calls.borrow().iter().filter(|c|["maximize","unmaximize"].contains(c)).count(),1);}}
 async fn turns(){tokio::task::yield_now().await;tokio::task::yield_now().await;}
 #[tokio::test] async fn lock_precedes_state_and_native_work_and_covers_completion(){tokio::task::LocalSet::new().run_until(async{let state=Arc::new(AppState{operations:Arc::new(tokio::sync::Mutex::new(()))});let held=state.operations.lock().await;let w=WebviewWindow::with_state(false,false,state.clone());let before_ack=w.set_barrier();let completion_ack=w.set_barrier();let native_ack=w.set_completion_ack();let w2=w.clone();let task=tokio::task::spawn_local(async move{window_toggle_maximize(w2,serde_json::Value::Null).await});turns().await;assert!(w.calls.borrow().is_empty(),"must not read state or request native work before lock");drop(held);turns().await;assert_eq!(w.calls.borrow().as_slice(),["barrier"],"must wait for queued native operations before reading canonical state");assert!(!task.is_finished(),"must not reply before the initial native-operation barrier");before_ack.send(Ok(())).unwrap();turns().await;if LINUX{assert_eq!(w.calls.borrow().as_slice(),["barrier",CANONICAL_READ,"maximize","linux-ack"]);assert_eq!(w.completion_requests.borrow().as_slice(),[true]);assert!(!task.is_finished(),"must wait for Linux native acknowledgement");native_ack.send(Ok(())).unwrap();turns().await;}let expected=if LINUX{vec!["barrier",CANONICAL_READ,"maximize","linux-ack","barrier"]}else{vec!["barrier",CANONICAL_READ,"maximize","barrier"]};assert_eq!(w.calls.borrow().as_slice(),expected);assert!(!task.is_finished(),"must not reply before the completion barrier");let second=w.clone();let second_task=tokio::task::spawn_local(async move{window_toggle_maximize(second,serde_json::Value::Null).await});turns().await;assert_eq!(w.calls.borrow().as_slice(),expected,"later toggle must wait through first completion barrier");let close_state=state.clone();let close=tokio::task::spawn_local(async move{let _guard=close_state.lock_window_operations().await;});turns().await;assert!(!close.is_finished(),"close must share the operation lock");completion_ack.send(Ok(())).unwrap();assert_eq!(task.await.unwrap(),Ok(()));turns().await;assert_eq!(w.calls.borrow().iter().filter(|c|**c=="unmaximize").count(),1);drop(second_task);drop(close);}).await}
 #[tokio::test] async fn initial_barrier_error_prevents_state_read_and_native_request(){tokio::task::LocalSet::new().run_until(async{let w=WebviewWindow::new(false,false,false,false);let ack=w.set_barrier();let task=tokio::task::spawn_local({let w=w.clone();async move{window_toggle_maximize(w,serde_json::Value::Null).await}});turns().await;ack.send(Err(AppError::Io("initial barrier".into()))).unwrap();let result=task.await.unwrap();assert!(!w.calls.borrow().iter().any(|c|["native","raw","maximize","unmaximize"].contains(c)),"initial barrier error must prevent canonical reads and native requests");assert_eq!(result,Err(AppError::Io("initial barrier".into())));}).await}
 #[tokio::test] async fn barrier_error_is_returned_after_native_request(){tokio::task::LocalSet::new().run_until(async{let w=WebviewWindow::new(false,false,false,false);let before_ack=w.set_barrier();let completion_ack=w.set_barrier();let native_ack=w.set_completion_ack();let task=tokio::task::spawn_local({let w=w.clone();async move{window_toggle_maximize(w,serde_json::Value::Null).await}});turns().await;assert_eq!(w.calls.borrow().as_slice(),["barrier"]);before_ack.send(Ok(())).unwrap();turns().await;if LINUX{native_ack.send(Ok(())).unwrap();turns().await;}completion_ack.send(Err(AppError::Io("completion barrier".into()))).unwrap();assert_eq!(task.await.unwrap(),Err(AppError::Io("completion barrier".into())));}).await}
 #[cfg(any(target_os="linux",feature="fixture-linux"))] #[tokio::test] async fn linux_ack_waits_reports_requested_direction_and_propagates_error(){tokio::task::LocalSet::new().run_until(async{let w=WebviewWindow::new(false,false,false,false);let before=w.set_barrier();let native=w.set_completion_ack();let task=tokio::task::spawn_local({let w=w.clone();async move{window_toggle_maximize(w,serde_json::Value::Null).await}});turns().await;before.send(Ok(())).unwrap();turns().await;assert_eq!(w.calls.borrow().as_slice(),["barrier",CANONICAL_READ,"maximize","linux-ack"]);assert_eq!(w.completion_requests.borrow().as_slice(),[true]);assert!(!task.is_finished(),"must await Linux native acknowledgement");native.send(Err(AppError::Io("Linux ack".into()))).unwrap();assert_eq!(task.await.unwrap(),Err(AppError::Io("Linux ack".into())));}).await}
 #[tokio::test] async fn cancelling_wait_releases_lock_and_peer_finishes(){tokio::task::LocalSet::new().run_until(async{let w=WebviewWindow::new(false,false,false,false);let _ack=w.set_barrier();let task=tokio::task::spawn_local({let w=w.clone();async move{window_toggle_maximize(w,serde_json::Value::Null).await}});turns().await;let waiting_at_barrier=!task.is_finished()&&w.calls.borrow().as_slice()==["barrier"];task.abort();turns().await;assert!(w.state.operations.try_lock().is_ok(),"cancelled operation must release lock");let peer=WebviewWindow::with_state(false,false,w.state.clone());let result=tokio::task::spawn_local(async move{window_toggle_maximize(peer,serde_json::Value::Null).await}).await.unwrap();assert_eq!(result,Ok(()),"peer operation must acquire the released lock and finish");assert!(waiting_at_barrier,"toggle must be cancelled while waiting for its native-operation barrier");}).await}
 #[tokio::test] async fn successive_toggles_choose_opposite_canonical_direction(){let w=WebviewWindow::new(false,false,false,false);window_toggle_maximize(w.clone(),serde_json::Value::Null).await.unwrap();window_toggle_maximize(w.clone(),serde_json::Value::Null).await.unwrap();let actions=w.calls.borrow().iter().copied().filter(|c|["maximize","unmaximize"].contains(c)).collect::<Vec<_>>();assert_eq!(actions,["maximize","unmaximize"]);}
}
`)
        process.exitCode = 0
        for (const featureArgs of [[], ['--features', 'fixture-linux']]) {
            const result = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml'), ...featureArgs], { encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
            if (result.error) throw result.error
            process.stdout.write(`\n=== cargo test ${featureArgs.join(' ')} ===\n`)
            process.stdout.write(result.stdout ?? '')
            process.stderr.write(result.stderr ?? '')
            if (result.status !== 0) process.exitCode = result.status ?? 1
        }
    }
} finally {
    if (temporary) fs.rmSync(temporary, { recursive: true, force: true })
    for (const file of inputs) assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), hashes.get(file), `source stayed unchanged: ${file}`)
}
