import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

if (process.argv.length === 3 && process.argv[2] === '--help') {
    console.log('Usage: node scripts/test-linux-window-docking.mjs\nExtracts the production window_set_docking command into a temporary offline Rust crate. Linux framework/window boundaries are simulated; the temporary crate is removed after the run.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'no arguments supported; use --help')

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'src-tauri/src/commands/desktop.rs')
const scriptPath = fileURLToPath(import.meta.url)
const geometryPath = path.join(root, 'src-tauri/src/desktop.rs')
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
const fingerprints = new Map([sourcePath, scriptPath, geometryPath].map(file => [file, hash(file)]))
const source = fs.readFileSync(sourcePath, 'utf8')
const start = source.search(/pub\s+(?:async\s+)?fn\s+window_set_docking\s*\(/)
assert.ok(start >= 0, 'production window_set_docking method exists')
const brace = source.indexOf('{', start)
assert.ok(brace > start, 'production window_set_docking method has a body')
let depth = 0
let end = -1
for (let i = brace; i < source.length; i++) {
    if (source[i] === '{') depth++
    if (source[i] === '}' && --depth === 0) { end = i + 1; break }
}
assert.ok(end > brace, 'production window_set_docking method is complete')
let command = source.slice(start, end).trim()
command = command.replace(/^pub\s+/, '').replace(/^fn\s+window_set_docking\s*\(/, 'async fn window_set_docking(')
command = command.replaceAll('target_os = "linux"', 'feature = "linux"').replaceAll('target_os = "macos"', 'feature = "macos"').replaceAll('target_os = "windows"', 'feature = "windows"')
assert.match(command, /^async fn window_set_docking\(/, 'harness normalizes the extracted production method to async')

const geometrySource = fs.readFileSync(geometryPath, 'utf8')
const geometryStart = geometrySource.indexOf('pub fn docked_bounds(')
const geometryEnd = geometrySource.indexOf('\npub fn clamp_bounds(', geometryStart)
assert.ok(geometryStart >= 0 && geometryEnd > geometryStart, 'production docking geometry has a complete boundary')
const geometry = geometrySource.slice(geometryStart, geometryEnd).replace('pub fn docked_bounds(', 'fn docked_bounds(')

const rust = String.raw`
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

#[derive(Debug, Clone, PartialEq)] enum AppError { Io(String), Unsupported(String), NotFound(String), InvalidArgument(String) }
fn io_error(error: impl std::fmt::Display) -> AppError { AppError::Io(error.to_string()) }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] struct WindowBounds { x:i32, y:i32, width:u32, height:u32 }
#[derive(Clone, Debug, Default)] struct WindowStateSnapshot { visible:bool, always_on_top:bool, fullscreen:bool, maximized:bool, minimized:bool, focused:bool, bounds:WindowBounds, scale_factor:f64, capabilities:WindowCapabilities, caption_controls:Option<()> }
#[derive(Clone, Debug, Default)] struct WindowCapabilities { absolute_positioning:bool, docking:bool }
#[derive(Clone, Debug)] struct DockingOptions { side:String, screen_id:Option<i64>, fill:f64, space:f64, always_on_top:bool, min_width:u32, min_height:u32 }
#[derive(Clone, Debug)] struct ScreenInfo { id:i64, primary:bool, work_area:WindowBounds }
#[derive(Clone, Debug)] struct Monitor { id:i64, primary:bool, work_area:WindowBounds }
#[derive(Clone, Debug, Default)] struct WindowStatePatch { bounds:Option<WindowBounds>, always_on_top:Option<bool>, maximized:Option<bool>, visible:Option<bool> }

#[derive(Default)] struct Data { requested:WindowBounds, native:WindowBounds, cached:WindowBounds, top:bool, visible:bool, calls:Vec<String>, patches:Vec<WindowStatePatch>, apply_error:bool, barrier_error:bool, bounds_error:bool, position_error:bool, top_error:bool, monitor_error:bool, primary_error:bool, capabilities:bool, monitors:Vec<Monitor>, primary:Option<i64>, barrier_done:bool, show_hide:usize, disk_writes:usize }
#[derive(Clone, Default)] pub struct WebviewWindow(Arc<Mutex<Data>>);
impl WebviewWindow {
 fn state(&self) -> WindowStateSnapshot { let d=self.0.lock().unwrap(); WindowStateSnapshot { visible:d.visible, always_on_top:d.top, bounds:d.native, capabilities:WindowCapabilities { absolute_positioning:d.capabilities, docking:d.capabilities }, ..Default::default() } }
 fn set_size(&self, size:PhysicalSize<u32>) -> Result<(),String> { let mut d=self.0.lock().unwrap(); d.calls.push("set_size".into()); if d.bounds_error{return Err("size failure".into())} d.requested.width=size.width;d.requested.height=size.height;Ok(()) }
 fn set_position(&self, pos:PhysicalPosition<i32>) -> Result<(),String> { let mut d=self.0.lock().unwrap();d.calls.push("set_position".into());if d.position_error{return Err("position failure".into())}d.requested.x=pos.x;d.requested.y=pos.y;Ok(()) }
 fn set_always_on_top(&self,v:bool)->Result<(),String>{let mut d=self.0.lock().unwrap();d.calls.push("set_top".into());if d.top_error{return Err("top failure".into())}d.top=v;Ok(())}
 fn available_monitors(&self)->Result<Vec<Monitor>,String>{let mut d=self.0.lock().unwrap();d.calls.push("available_monitors".into());if d.monitor_error{return Err("monitor failure".into())}Ok(d.monitors.clone())}
 fn primary_monitor(&self)->Result<Option<Monitor>,String>{let mut d=self.0.lock().unwrap();d.calls.push("primary_monitor".into());if d.primary_error{return Err("primary failure".into())}Ok(d.monitors.iter().find(|m|Some(m.id)==d.primary).cloned())}
}
#[derive(Clone,Copy)] struct PhysicalSize<T>{width:T,height:T} impl<T> PhysicalSize<T>{fn new(width:T,height:T)->Self{Self{width,height}}}
#[derive(Clone,Copy)] struct PhysicalPosition<T>{x:T,y:T} impl<T> PhysicalPosition<T>{fn new(x:T,y:T)->Self{Self{x,y}}}
mod tauri { pub use crate::WebviewWindow; }
fn window_capabilities(w:&WebviewWindow)->Result<WindowCapabilities,AppError>{let d=w.0.lock().unwrap();Ok(WindowCapabilities{absolute_positioning:d.capabilities,docking:d.capabilities})}
fn window_state(w:&WebviewWindow)->Result<WindowStateSnapshot,AppError>{Ok(w.state())}
fn screen_info(monitors:&[Monitor], primary:Option<&Monitor>)->Vec<ScreenInfo>{let mut result:Vec<_>=monitors.iter().map(|m|ScreenInfo{id:m.id,primary:primary.map(|p|p.id)==Some(m.id),work_area:m.work_area}).collect();result.sort_by_key(|s|s.work_area.x);result}
__GEOMETRY__
fn set_bounds_size(w:&WebviewWindow,s:PhysicalSize<u32>)->Result<(),AppError>{w.set_size(s).map_err(io_error)}
fn set_bounds_position(w:&WebviewWindow,p:PhysicalPosition<i32>)->Result<(),AppError>{w.set_position(p).map_err(io_error)}
async fn wait_for_window_operations(w:&WebviewWindow)->Result<(),AppError>{let mut d=w.0.lock().unwrap();d.calls.push("barrier".into());if d.barrier_error{return Err(AppError::Io("barrier failure".into()))}d.native=d.requested;d.barrier_done=true;Ok(())}
async fn window_apply_state(w:WebviewWindow,p:WindowStatePatch)->Result<(),AppError>{let needs_bounds=p.bounds.is_some();{let mut d=w.0.lock().unwrap();d.calls.push("apply_state".into());d.patches.push(p.clone());if d.apply_error{return Err(AppError::Io("apply failure".into()))}if let Some(b)=p.bounds{d.requested=b}if let Some(v)=p.always_on_top{d.top=v}}if needs_bounds{wait_for_window_operations(&w).await?;let mut d=w.0.lock().unwrap();d.calls.push("capture".into());d.cached=d.native;}Ok(())}
__METHOD__

struct Noop; impl Wake for Noop{fn wake(self:Arc<Self>){}}
fn block_on<F:Future>(f:F)->F::Output{let w=Waker::from(Arc::new(Noop));let mut cx=Context::from_waker(&w);let mut f=Box::pin(f);match Pin::as_mut(&mut f).poll(&mut cx){Poll::Ready(v)=>v,Poll::Pending=>panic!("unexpected pending operation in the docking command harness")}}
fn monitor(id:i64,primary:bool,work_area:WindowBounds)->Monitor{Monitor{id,primary,work_area}}
fn window()->WebviewWindow{let w=WebviewWindow::default();let mut d=w.0.lock().unwrap();d.capabilities=true;d.cached=WindowBounds{x:3,y:4,width:640,height:480};d.native=d.cached;d.requested=d.native;d.visible=true;d.monitors=vec![monitor(1,true,WindowBounds{x:0,y:0,width:1000,height:800})];d.primary=Some(1);drop(d);w}
fn request(side:&str)->DockingOptions{DockingOptions{side:side.into(),screen_id:None,fill:0.5,space:0.8,always_on_top:true,min_width:100,min_height:100}}
#[cfg(test)] mod tests{use super::*;
 fn expected(side:&str)->WindowBounds{match side{"left"=>WindowBounds{x:0,y:80,width:500,height:640},"right"=>WindowBounds{x:500,y:80,width:500,height:640},"top"=>WindowBounds{x:100,y:0,width:800,height:400},_=>WindowBounds{x:100,y:400,width:800,height:400}}}
 #[test]fn all_four_edges_finish_size_and_normal_cache_before_return(){for side in ["left","right","top","bottom"]{let w=window();let result=block_on(window_set_docking(w.clone(),request(side))).unwrap();let d=w.0.lock().unwrap();assert_eq!(result.bounds,expected(side),"{side}: returned snapshot must contain completed bounds");assert_eq!(d.native,expected(side),"{side}: native size/position must complete");assert_eq!(d.cached,expected(side),"{side}: normal cache must be sampled");assert!(d.barrier_done,"{side}: native operation barrier must complete");assert_eq!(result.always_on_top,true);assert_eq!(d.show_hide,0);assert_eq!(d.disk_writes,0);}}
 #[test]fn successful_linux_routing_changes_only_bounds_and_top(){for side in ["left","right","top","bottom"]{let w=window();block_on(window_set_docking(w.clone(),request(side))).unwrap();let d=w.0.lock().unwrap();assert_eq!(d.calls.iter().filter(|c|c.as_str()=="apply_state").count(),1,"{side}: use shared state application");let p=d.patches.last().unwrap();assert_eq!(p.bounds,Some(expected(side)));assert_eq!(p.always_on_top,Some(true));assert_eq!(p.maximized,None);assert_eq!(p.visible,None);}}
 #[test]fn off_uses_top_only_patch_without_bounds_capture_or_resize(){let w=window();{let mut d=w.0.lock().unwrap();d.top=true;}let before=w.state().bounds;let out=block_on(window_set_docking(w.clone(),request("off"))).unwrap();let d=w.0.lock().unwrap();assert!(!out.always_on_top);assert_eq!(out.bounds,before);assert_eq!(d.calls.iter().filter(|c|c.as_str()=="apply_state").count(),1);assert!(!d.calls.iter().any(|c|c=="set_size"||c=="set_position"||c=="capture"));let p=d.patches.last().unwrap();assert_eq!(p.bounds,None);assert_eq!(p.always_on_top,Some(false));assert_eq!(p.maximized,None);assert_eq!(p.visible,None);}
 #[test]fn unsupported_docking_rejects_without_mutation(){let w=window();w.0.lock().unwrap().capabilities=false;let before=w.state().bounds;assert!(matches!(block_on(window_set_docking(w.clone(),request("left"))),Err(AppError::Unsupported(_))));let d=w.0.lock().unwrap();assert_eq!(d.native,before);assert_eq!(d.cached,before);assert!(!d.calls.iter().any(|c|c=="apply_state"||c=="set_size"||c=="set_position"||c=="set_top"));}
 #[test]fn selected_screen_then_primary_fallback_are_preserved(){let w=window();{let mut d=w.0.lock().unwrap();d.monitors=vec![monitor(2,false,WindowBounds{x:1200,y:20,width:800,height:600}),monitor(1,true,WindowBounds{x:0,y:0,width:1000,height:800})];d.primary=Some(1);}let mut r=request("left");r.screen_id=Some(2);block_on(window_set_docking(w.clone(),r)).unwrap();assert_eq!(w.0.lock().unwrap().requested.x,1200);let mut r=request("left");r.screen_id=Some(99);block_on(window_set_docking(w.clone(),r)).unwrap();assert_eq!(w.0.lock().unwrap().requested.x,0);}
 #[test]fn missing_primary_uses_first_sorted_screen(){let w=window();{let mut d=w.0.lock().unwrap();d.monitors=vec![monitor(2,false,WindowBounds{x:1200,y:0,width:800,height:600}),monitor(1,false,WindowBounds{x:-1000,y:0,width:900,height:700})];d.primary=None;}block_on(window_set_docking(w.clone(),request("left"))).unwrap();assert_eq!(w.0.lock().unwrap().requested.x,-1000);}
 #[test]fn invalid_geometry_and_monitor_errors_are_propagated(){let w=window();assert!(matches!(block_on(window_set_docking(w,request("diagonal"))),Err(AppError::InvalidArgument(s)) if s=="invalid docking side"));let w=window();w.0.lock().unwrap().monitor_error=true;assert!(matches!(block_on(window_set_docking(w,request("left"))),Err(AppError::Io(s)) if s=="monitor failure"));let w=window();w.0.lock().unwrap().primary_error=true;assert!(matches!(block_on(window_set_docking(w,request("left"))),Err(AppError::Io(s)) if s=="primary failure"));}
 #[test]fn apply_error_is_propagated(){let w=window();w.0.lock().unwrap().apply_error=true;assert!(matches!(block_on(window_set_docking(w,request("left"))),Err(AppError::Io(s)) if s=="apply failure"));}
 #[test]fn barrier_error_is_propagated(){let w=window();w.0.lock().unwrap().barrier_error=true;assert!(matches!(block_on(window_set_docking(w,request("left"))),Err(AppError::Io(s)) if s=="barrier failure"));}
 #[test]fn windows_do_not_share_native_or_cached_state(){let a=window();let b=window();let mut r=request("right");r.always_on_top=false;block_on(window_set_docking(a.clone(),r)).unwrap();let b_state=block_on(window_set_docking(b.clone(),request("top"))).unwrap();assert_eq!(a.state().bounds,expected("right"));assert_eq!(b_state.bounds,expected("top"));assert_eq!(b.0.lock().unwrap().cached,expected("top"));}
}
`
const crateSource = rust.replace('__METHOD__', command).replace('__GEOMETRY__', geometry)
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-linux-docking-'))
let result
try {
    fs.writeFileSync(path.join(tempRoot, 'Cargo.toml'), '[package]\nname = "linux-window-docking-regression"\nversion = "0.1.0"\nedition = "2021"\n[features]\ndefault = ["linux"]\nlinux = []\nmacos = []\nwindows = []\n')
    fs.mkdirSync(path.join(tempRoot, 'src'))
    fs.writeFileSync(path.join(tempRoot, 'src/lib.rs'), crateSource)
    result = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(tempRoot, 'Cargo.toml'), '--', '--test-threads=1'], { cwd: root, encoding: 'utf8', timeout: 120000, maxBuffer: 8 * 1024 * 1024 })
    if (result.error) throw result.error
    process.stdout.write(result.stdout ?? '')
    process.stderr.write(result.stderr ?? '')
    assert.equal(result.status, 0, `cargo regression runner failed with status ${result.status}`)
    assert.match(result.stdout, /test result: ok\. 10 passed; 0 failed/)
} finally {
    fs.rmSync(tempRoot, { recursive: true, force: true })
    for (const [file, before] of fingerprints) assert.equal(hash(file), before, `${path.relative(root, file)} did not change while running`)
}
