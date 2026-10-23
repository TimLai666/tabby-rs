import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

if (process.argv.length === 3 && process.argv[2] === '--help') {
    console.log('Usage: node scripts/test-window-apply-geometry.mjs\nExtracts and runs the production window.applyState command and geometry helpers in an offline temporary Rust crate. Framework and OS operations are simulated; removes owned temporary files.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'no arguments supported; use --help')

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const commandFile = path.join(root, 'src-tauri/src/commands/desktop.rs')
const fixtureFile = path.join(root, 'scripts/fixtures/window-apply-regressions.rs')
const targetFile = fileURLToPath(import.meta.url)
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
const fingerprints = new Map([commandFile, fixtureFile, targetFile].map(file => [file, hash(file)]))

const extractFunction = (source, name, nextMarker) => {
    const start = source.indexOf(`fn ${name}(`)
    assert.ok(start >= 0, `production ${name} exists`)
    const end = source.indexOf(nextMarker, start)
    assert.ok(end > start, `production ${name} has a complete boundary`)
    return source.slice(start, end).trim()
}
const source = fs.readFileSync(commandFile, 'utf8')
const commandStart = source.search(/pub async fn window_apply_state\(/)
assert.ok(commandStart >= 0, 'production async window_apply_state exists')
const commandEnd = source.indexOf('\n#[tauri::command]', commandStart)
assert.ok(commandEnd > commandStart, 'production command ends before next command')
const selectFeatures = value => value.replaceAll('target_os = "macos"', 'feature = "macos"').replaceAll('target_os = "windows"', 'feature = "windows"').replaceAll('target_os = "linux"', 'feature = "linux"')
const command = selectFeatures(source.slice(commandStart, commandEnd).replace('pub async fn window_apply_state(', 'async fn window_apply_state('))
const ioError = extractFunction(source, 'io_error', '\n\nfn set_bounds_size')
const setSize = extractFunction(source, 'set_bounds_size', '\n\nfn set_bounds_position')
const setPosition = extractFunction(source, 'set_bounds_position', '\n\nfn window_capabilities')
const runnableSize = selectFeatures(setSize)
const runnablePosition = selectFeatures(setPosition)

let fixture = fs.readFileSync(fixtureFile, 'utf8')
fixture = fixture.replace('mod platform {\n    use super::*;\n', String.raw`mod platform {
    use super::*;
    #[cfg(feature = "linux")]
    pub mod window_completion_linux {
        use super::*;
        // This boundary records requests and completion, not GTK sizing policy.
        pub fn set_size(w: &WebviewWindow, z: PhysicalSize<u32>) -> Result<(), AppError> {
            let mut s = w.0.lock().unwrap();
            s.operations.push("resize".into());
            if s.fail == Some("resize") { return Err(AppError::Io("resize failure".into())); }
            s.width=z.width; s.height=z.height; s.native_width=z.width; s.native_height=z.height;
            Ok(())
        }
        pub fn accepts_bounds(w: &WebviewWindow) -> Result<bool, AppError> {
            let s = w.0.lock().unwrap();
            if s.fail == Some("accepts") { return Err(AppError::Io("accepts failure".into())); }
            Ok(!s.max && !s.fullscreen)
        }
        async fn wait(w: &WebviewWindow, kind: &'static str) -> Result<(), AppError> {
            {
                let mut s = w.0.lock().unwrap();
                s.operations.push(kind.into());
                if s.fail == Some(kind) { return Err(AppError::Io(format!("{kind} failure"))); }
            }
            std::future::poll_fn(|cx| {
                let s = w.0.lock().unwrap();
                let paused = if kind == "wait-size" { s.pause_native_size }
                    else if kind == "wait-bounds" { s.pause_native_bounds }
                    else if kind.starts_with("wait-fullscreen") { s.pause_native_fullscreen }
                    else { s.pause_native_max };
                if paused { cx.waker().wake_by_ref(); std::task::Poll::Pending }
                else { std::task::Poll::Ready(Ok(())) }
            }).await
        }
        pub async fn wait_for_maximized(w: &WebviewWindow, value: bool) -> Result<(), AppError> {
            wait(w, if value { "wait-max-true" } else { "wait-max-false" }).await
        }
        pub async fn wait_for_fullscreen(w: &WebviewWindow, value: bool) -> Result<(), AppError> {
            wait(w, if value { "wait-fullscreen-true" } else { "wait-fullscreen-false" }).await
        }
        pub async fn wait_for_size(w: &WebviewWindow, _: PhysicalSize<u32>) -> Result<(), AppError> {
            wait(w, "wait-size").await
        }
        pub async fn wait_for_bounds(w: &WebviewWindow, _: WindowBounds) -> Result<(), AppError> {
            wait(w, "wait-bounds").await
        }
    }
    pub fn set_window_always_on_top(window: &WebviewWindow, value: bool) -> Result<(), AppError> {
        window.set_always_on_top(value).map_err(io_error)
    }
`)
fixture = fixture.replace('fn io_error(e: impl std::fmt::Display) -> AppError {\n    AppError::Io(e.to_string())\n}', 'PRODUCTION_IO_ERROR')
fixture = fixture.replace('fn set_bounds_size(w: &WebviewWindow, s: PhysicalSize<u32>) -> Result<(), AppError> {\n    let mut v = w.0.lock().unwrap();\n    v.max = false;\n    v.width = s.width;\n    v.height = s.height;\n    Ok(())\n}', 'PRODUCTION_SET_SIZE')
fixture = fixture.replace('fn set_bounds_position(_: &WebviewWindow, _: PhysicalPosition<i32>) -> Result<(), AppError> {\n    Ok(())\n}', 'PRODUCTION_SET_POSITION')
for (const marker of ['PRODUCTION_IO_ERROR', 'PRODUCTION_SET_SIZE', 'PRODUCTION_SET_POSITION']) {
    assert.equal(fixture.split(marker).length, 2, `${marker} replaces exactly one simulated helper`)
}
fixture = fixture.replace('PRODUCTION_IO_ERROR', selectFeatures(ioError))
fixture = fixture.replace('PRODUCTION_SET_SIZE', runnableSize)
fixture = fixture.replace('PRODUCTION_SET_POSITION', runnablePosition)
fixture = fixture.replace('fn window_capabilities(_: &WebviewWindow)', 'fn window_capabilities(window: &WebviewWindow)')
fixture = fixture.replace('    shown_max: Option<bool>,\n}', '    shown_max: Option<bool>,\n    fullscreen: bool,\n    operations: Vec<String>,\n    fail: Option<&\'static str>,\n    position_supported: bool,\n    position: (i32, i32),\n    pause_resize: bool,\n    pause_queue: bool,\n    native_width: u32,\n    native_height: u32,\n    cached_width: u32,\n    cached_height: u32,\n    capture_count: usize,\n    ui_callback_pending: bool,\n    fail_ui_dispatch: bool,\n    fail_capture_read: bool,\n    fail_capture_barrier: bool,\n    disk_writes: usize,\n}')
fixture = fixture.replace('    fullscreen: bool,', '    fullscreen: bool,\n    pause_native_size: bool,\n    pause_native_bounds: bool,\n    pause_native_max: bool,\n    pause_native_fullscreen: bool,')
fixture = fixture.replace('    fn set_fullscreen(&self, _: bool) -> Result<(), String> {\n        Ok(())\n    }', '    fn set_fullscreen(&self, value: bool) -> Result<(), String> { self.0.lock().unwrap().fullscreen=value; Ok(()) }')
fixture = fixture.replace('#[derive(Clone, Default)]\nstruct WebviewWindow(Arc<Mutex<Snapshot>>);', '#[derive(Clone)]\nstruct WebviewWindow(Arc<Mutex<Snapshot>>);\nimpl Default for WebviewWindow { fn default() -> Self { Self(Arc::new(Mutex::new(Snapshot { position_supported: true, ..Snapshot::default() }))) } }')
fixture = fixture.replace('    fn maximize(&self) -> Result<(), String> {\n        let mut s = self.0.lock().unwrap();', '    fn maximize(&self) -> Result<(), String> {\n        let mut s = self.0.lock().unwrap();\n        s.operations.push("maximize".into());\n        if s.fail == Some("maximize") { return Err("maximize failure".into()); }')
fixture = fixture.replace('    fn unmaximize(&self) -> Result<(), String> {\n        let mut s = self.0.lock().unwrap();', '    fn unmaximize(&self) -> Result<(), String> {\n        let mut s = self.0.lock().unwrap();\n        s.operations.push("unmaximize".into());\n        if s.fail == Some("unmaximize") { return Err("unmaximize failure".into()); }')

fixture = fixture.replace('    fn set_title(&self, _: &str) -> Result<(), String> {', '    fn run_on_main_thread(&self, callback: impl FnOnce() + Send + \'static) -> Result<(), String> { let mut s=self.0.lock().unwrap(); s.operations.push(\"ui-dispatch\".into()); if s.fail_ui_dispatch { return Err(\"UI dispatch failure\".into()); } s.ui_callback_pending=true; drop(s); UI_TASKS.with(|tasks| tasks.borrow_mut().push(Box::new(callback))); Ok(()) }\n    fn set_title(&self, _: &str) -> Result<(), String> {')
fixture = fixture.replace('struct Noop;\n', `thread_local! { static CAPTURE_UI: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; static UI_TASKS: std::cell::RefCell<Vec<Box<dyn FnOnce() + Send>>> = std::cell::RefCell::new(Vec::new()); }\n#[cfg(feature = \"linux\")]\nmod window_persistence_linux { use super::*; pub fn capture_window(window: &WebviewWindow, _: &AppState) -> Result<(), AppError> { let mut s=window.0.lock().unwrap(); s.operations.push(\"capture\".into()); s.capture_count+=1; assert!(CAPTURE_UI.with(|v|v.get()), \"capture must execute in the UI callback\"); if s.fail_capture_read { return Err(AppError::Io(\"native capture read failure\".into())); } if !s.max { s.cached_width=s.native_width; s.cached_height=s.native_height; } Ok(()) } }\nstruct Noop;\n`)
fixture = fixture.replace('    fn show(&self) -> Result<(), String> {', '    fn set_position(&self, p: PhysicalPosition<i32>) -> Result<(), String> { let mut s=self.0.lock().unwrap(); s.operations.push("position".into()); if s.fail==Some("position") { return Err("position failure".into()); } s.position=(p.x,p.y); Ok(()) }\n    fn set_size(&self, z: PhysicalSize<u32>) -> Result<(), String> { let mut s=self.0.lock().unwrap(); s.operations.push("resize".into()); if s.fail==Some("resize") { return Err("resize failure".into()); } s.max=false; s.width=z.width; s.height=z.height; s.native_width=z.width; s.native_height=z.height; Ok(()) }\n    fn show(&self) -> Result<(), String> {')
fixture = fixture.replace('    fn hide(&self) -> Result<(), String> {\n        self.0.lock().unwrap().shown = false;\n        Ok(())\n    }', '    fn hide(&self) -> Result<(), String> {\n        let mut s=self.0.lock().unwrap(); s.operations.push(\"hide\".into()); s.shown=false; Ok(())\n    }')
fixture = fixture.replace('    fn show(&self) -> Result<(), String> {\n        let mut s = self.0.lock().unwrap();', '    fn show(&self) -> Result<(), String> {\n        let mut s = self.0.lock().unwrap();\n        s.operations.push("show".into());\n        if s.fail == Some("show") { return Err("show failure".into()); }')
fixture = fixture.replace('pub async fn wait_for_window_operations(w: &WebviewWindow) -> Result<(), AppError> {\n        let mut s = w.0.lock().unwrap();', 'pub async fn wait_for_window_operations(w: &WebviewWindow) -> Result<(), AppError> {\n        std::future::poll_fn(|cx| { let s=w.0.lock().unwrap(); if !s.pending.is_empty() && s.pause_queue { cx.waker().wake_by_ref(); return std::task::Poll::Pending; } std::task::Poll::Ready(()) }).await;\n        let mut s = w.0.lock().unwrap();\n        s.operations.push("wait".into());')
fixture = fixture.replace('        for state in work {\n            s.max = state;\n        }', '        for state in work {\n            if s.fail == Some("queued") { return Err(AppError::Io("queue failure".into())); }\n            s.max = state;\n        }')
fixture = fixture.replace('        let mut s = w.0.lock().unwrap();\n        s.operations.push(\"wait\".into());', '        let mut s = w.0.lock().unwrap();\n        s.operations.push(\"wait\".into());\n        if s.ui_callback_pending { if s.fail_capture_barrier { return Err(AppError::Io(\"capture barrier failure\".into())); } s.ui_callback_pending=false; drop(s); UI_TASKS.with(|tasks| { let callbacks=std::mem::take(&mut *tasks.borrow_mut()); for callback in callbacks { CAPTURE_UI.with(|v|v.set(true)); callback(); CAPTURE_UI.with(|v|v.set(false)); } }); s=w.0.lock().unwrap(); }')
assert.equal((fixture.match(/if s.ui_callback_pending/g) || []).length, 1, 'capture completion wait injection exists exactly once')
assert.equal(fixture.includes('s.operations.push(\"wait\".into())'), true, 'native operation wait injection exists')
assert.equal((fixture.match(/mod window_persistence_linux/g) || []).length, 1, 'Linux capture fixture exists exactly once')
fixture = fixture.replace('    pub async fn set_window_outer_size_async(\n', '    pub fn set_window_outer_size(w: &WebviewWindow, size: PhysicalSize<u32>) -> Result<(), AppError> { let mut s=w.0.lock().unwrap(); s.operations.push("resize".into()); if s.fail==Some("resize") { return Err(AppError::Io("resize failure".into())); } s.max=false; s.width=size.width; s.height=size.height; Ok(()) }\n    pub fn set_window_position(w: &WebviewWindow, position: PhysicalPosition<i32>) -> Result<(), AppError> { let mut s=w.0.lock().unwrap(); s.operations.push("position".into()); if s.fail==Some("position") { return Err(AppError::Io("position failure".into())); } s.position=(position.x,position.y); Ok(()) }\n    pub async fn set_window_outer_size_async(\n')
fixture = fixture.replace('        set_bounds_size(w, s)\n', '        std::future::poll_fn(|cx| { let state=w.0.lock().unwrap(); if state.pause_resize { cx.waker().wake_by_ref(); return std::task::Poll::Pending; } std::task::Poll::Ready(()) }).await;\n        let mut state = w.0.lock().unwrap();\n        state.operations.push("resize".into());\n        if state.fail == Some("resize") { return Err(AppError::Io("resize failure".into())); }\n        state.max = false; state.width = s.width; state.height = s.height; state.native_width=s.width; state.native_height=s.height; Ok(())\n')
fixture = fixture.replace('        absolute_positioning: true,', '        absolute_positioning: window.0.lock().unwrap().position_supported,')

const extraTests = String.raw`
#[test] fn bounds_then_max_true_keeps_requested_normal_size_and_finishes_maximized() {
 let w=WebviewWindow::default(); block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:11,y:22,width:800,height:600}),maximized:Some(true),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); assert!(s.max); assert_eq!((s.width,s.height),(800,600)); assert!(s.operations.iter().position(|x|x=="resize").unwrap()<s.operations.iter().position(|x|x=="maximize").unwrap());
}
#[test] fn bounds_then_max_false_ends_normal_with_requested_bounds() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.max=true; }
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:7,y:8,width:700,height:500}),maximized:Some(false),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); assert!(!s.max); assert_eq!((s.width,s.height),(700,500)); assert_eq!(s.position,(7,8));
}
#[test] fn bounds_only_resizes_and_preserves_position_capability_behavior() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.max=true; s.width=800; s.height=600; s.position=(80,90); }
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:3,y:4,width:321,height:222}),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap();
 #[cfg(not(feature = "linux"))] { assert!(!s.max); assert_eq!((s.width,s.height),(321,222)); assert_eq!(s.position,(3,4)); }
 #[cfg(feature = "linux")] { assert!(s.max); assert_eq!((s.width,s.height),(800,600)); assert_eq!(s.position,(80,90)); assert!(!s.operations.iter().any(|x|x=="resize"||x=="position"||x=="capture")); }
}
#[cfg(feature = "linux")]
#[test] fn bounds_only_applies_to_a_normal_native_window() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.width=100; s.height=90; s.native_width=100; s.native_height=90; }
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:5,y:6,width:321,height:222}),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); assert!(!s.max); assert_eq!((s.width,s.height),(321,222)); assert_eq!(s.position,(5,6)); assert!(s.operations.iter().any(|x|x=="resize"));
}
#[test] fn max_only_true_false_and_empty_patch() {
 let w=WebviewWindow::default(); block_on(window_apply_state(w.clone(),WindowStatePatch{maximized:Some(true),..Default::default()})).unwrap(); assert!(w.0.lock().unwrap().max);
 block_on(window_apply_state(w.clone(),WindowStatePatch{maximized:Some(false),..Default::default()})).unwrap(); assert!(!w.0.lock().unwrap().max);
 let before=w.0.lock().unwrap().operations.len(); block_on(window_apply_state(w.clone(),WindowStatePatch::default())).unwrap(); assert_eq!(before,w.0.lock().unwrap().operations.len());
}
#[test] fn unsupported_position_skips_only_position() {
 let w=WebviewWindow::default(); {w.0.lock().unwrap().position_supported=false;}
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:8,y:9,width:321,height:222}),maximized:Some(true),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); assert_eq!((s.width,s.height),(321,222)); assert!(s.max); assert!(!s.operations.iter().any(|x|x=="position"));
}
#[test] fn geometry_errors_are_returned_and_never_show() {
 for (case,expected) in [("resize","Io(\"resize failure\")"),("position","Io(\"position failure\")"),("maximize","Io(\"maximize failure\")"),("queued","Io(\"queue failure\")"),("unmaximize","Io(\"unmaximize failure\")")] { let w=WebviewWindow::default(); {w.0.lock().unwrap().fail=Some(case);}
  let result=block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:30,height:40}),maximized:Some(true),visible:Some(true),..Default::default()}));
  assert_eq!(format!("{:?}",result.unwrap_err()),expected,"{case} error changed"); assert!(!w.0.lock().unwrap().shown,"{case} failure showed window"); }
}
#[test] fn separate_windows_keep_independent_geometry() {
 let a=WebviewWindow::default(); let b=WebviewWindow::default(); block_on(window_apply_state(a.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:300,height:200}),maximized:Some(true),..Default::default()})).unwrap();
 block_on(window_apply_state(b.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:9,y:8,width:600,height:400}),maximized:Some(false),..Default::default()})).unwrap();
 {let s=a.0.lock().unwrap(); assert_eq!((s.width,s.height),(300,200)); assert!(s.max);} {let s=b.0.lock().unwrap(); assert_eq!((s.width,s.height),(600,400)); assert!(!s.max);}
}
#[test] fn visible_is_set_only_after_final_geometry_and_pending_queue_is_bounded() {
 let w=WebviewWindow::default(); w.maximize().unwrap(); block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:300,height:200}),maximized:Some(true),visible:Some(true),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); let show=s.operations.iter().position(|x|x=="show").unwrap(); let max=s.operations.iter().rposition(|x|x=="maximize").unwrap(); assert!(max<show); assert_eq!(s.shown_max,Some(true));
}
#[test] fn io_error_preserves_original_message() { let error=io_error("marker"); assert_eq!(format!("{error:?}"),"Io(\"marker\")"); }
#[test] fn production_geometry_helpers_use_the_boundary_and_preserve_errors() {
 let w=WebviewWindow::default(); set_bounds_size(&w,PhysicalSize::new(77,88)).unwrap(); {let s=w.0.lock().unwrap(); assert_eq!((s.width,s.height),(77,88));}
 set_bounds_position(&w,PhysicalPosition::new(5,6)).unwrap(); assert_eq!(w.0.lock().unwrap().position,(5,6));
 {w.0.lock().unwrap().fail=Some("resize");} assert!(set_bounds_size(&w,PhysicalSize::new(1,1)).is_err());
}
`
const linuxTests = String.raw`
#[cfg(feature = "linux")]
#[test] fn hidden_normal_bounds_capture_native_size_before_hide() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.width=800; s.height=600; s.native_width=800; s.native_height=600; s.cached_width=800; s.cached_height=600; }
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:900,height:650}),visible:Some(false),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); assert_eq!(s.capture_count,1); assert_eq!((s.cached_width,s.cached_height),(900,650),"capture must sample the native normal size"); assert!(s.operations.iter().position(|x|x=="capture").unwrap()<s.operations.iter().position(|x|x=="hide").unwrap()); assert_eq!(s.disk_writes,0);
}
#[cfg(feature = "linux")]
#[test] fn bounds_then_max_true_captures_normal_size_before_final_maximize() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.width=800; s.height=600; s.native_width=800; s.native_height=600; s.cached_width=800; s.cached_height=600; }
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:900,height:650}),maximized:Some(true),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap(); assert_eq!(s.capture_count,1); assert_eq!((s.cached_width,s.cached_height),(900,650)); assert!(s.operations.iter().position(|x|x=="capture").unwrap()<s.operations.iter().position(|x|x=="maximize").unwrap()); assert!(s.max);
}
#[cfg(feature = "linux")]
#[test] fn empty_visible_only_and_max_only_patches_do_not_capture() {
 for patch in [WindowStatePatch::default(),WindowStatePatch{visible:Some(false),..Default::default()},WindowStatePatch{maximized:Some(true),..Default::default()}] { let w=WebviewWindow::default(); block_on(window_apply_state(w.clone(),patch)).unwrap(); assert_eq!(w.0.lock().unwrap().capture_count,0); }
}
#[cfg(feature = "linux")]
#[test] fn native_read_failure_keeps_cache_and_still_allows_hide() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.width=800; s.height=600; s.native_width=800; s.native_height=600; s.cached_width=800; s.cached_height=600; s.shown=true; s.fail_capture_read=true; }
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:900,height:650}),visible:Some(false),..Default::default()})).unwrap(); let s=w.0.lock().unwrap(); assert_eq!((s.cached_width,s.cached_height),(800,600)); assert!(!s.shown); assert_eq!(s.capture_count,1);
}
#[cfg(feature = "linux")]
#[test] fn ui_dispatch_failure_propagates_before_visibility_change() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.shown=true; s.fail_ui_dispatch=true; } let result=block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:900,height:650}),visible:Some(false),..Default::default()})); assert_eq!(format!("{:?}",result.unwrap_err()),"Io(\"UI dispatch failure\")"); assert!(w.0.lock().unwrap().shown);
}
#[cfg(feature = "linux")]
#[test] fn capture_completion_barrier_failure_propagates_before_visibility_change() {
 let w=WebviewWindow::default(); { let mut s=w.0.lock().unwrap(); s.shown=true; s.fail_capture_barrier=true; } let result=block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:900,height:650}),visible:Some(false),..Default::default()})); assert_eq!(format!("{:?}",result.unwrap_err()),"Io(\"capture barrier failure\")"); assert!(w.0.lock().unwrap().shown);
}
#[cfg(feature = "linux")]
#[test] fn each_window_has_an_independent_normal_cache() {
 let a=WebviewWindow::default(); let b=WebviewWindow::default(); for w in [&a,&b] { let mut s=w.0.lock().unwrap(); s.width=800; s.height=600; s.native_width=800; s.native_height=600; s.cached_width=800; s.cached_height=600; }
 block_on(window_apply_state(a.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:900,height:650}),..Default::default()})).unwrap(); block_on(window_apply_state(b.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:4,y:5,width:700,height:500}),..Default::default()})).unwrap(); {let sa=a.0.lock().unwrap(); assert_eq!((sa.cached_width,sa.cached_height),(900,650)); assert_eq!(sa.capture_count,1);} {let sb=b.0.lock().unwrap(); assert_eq!((sb.cached_width,sb.cached_height),(700,500)); assert_eq!(sb.capture_count,1);}
}
`
const nativeCompletionTests = String.raw`
#[cfg(feature = "linux")]
#[test] fn native_size_ack_precedes_move_capture_and_visibility() {
 let w=WebviewWindow::default();w.0.lock().unwrap().pause_native_size=true;
 let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1920,y:540,width:1920,height:1080}),visible:Some(true),..Default::default()}));
 assert!(poll_pinned(f.as_mut()).is_pending()); {let s=w.0.lock().unwrap();assert!(!s.operations.iter().any(|v|v=="position"));assert_eq!(s.capture_count,0);assert!(!s.shown);}
 w.0.lock().unwrap().pause_native_size=false;assert!(poll_pinned(f.as_mut()).is_ready());
 let s=w.0.lock().unwrap();assert!(s.operations.iter().position(|v|v=="wait-size").unwrap()<s.operations.iter().position(|v|v=="position").unwrap());assert!(s.shown);
}
#[cfg(feature = "linux")]
#[test] fn unavailable_absolute_position_skips_the_pre_move_gate() {
 let w=WebviewWindow::default();{let mut s=w.0.lock().unwrap();s.position_supported=false;s.pause_native_size=true;}
 let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1920,y:540,width:1920,height:1080}),..Default::default()}));assert!(poll_pinned(f.as_mut()).is_ready());
 let s=w.0.lock().unwrap();assert!(!s.operations.iter().any(|v|v=="wait-size"||v=="position"));assert_eq!(s.capture_count,1);
}
#[cfg(feature = "linux")]
#[test] fn cancellation_while_size_is_pending_does_not_move_or_capture() {
 let w=WebviewWindow::default();w.0.lock().unwrap().pause_native_size=true;
 {let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1920,y:540,width:1920,height:1080}),..Default::default()}));assert!(poll_pinned(f.as_mut()).is_pending());}
 {let s=w.0.lock().unwrap();assert!(!s.operations.iter().any(|v|v=="position"));assert_eq!(s.capture_count,0);}
 w.0.lock().unwrap().pause_native_size=false;block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:3,y:4,width:700,height:500}),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap();assert_eq!(s.position,(3,4));assert_eq!(s.capture_count,1);
}

#[cfg(feature = "linux")]
#[test] fn native_unmaximize_ack_precedes_resize() {
 let w=WebviewWindow::default(); {let mut s=w.0.lock().unwrap(); s.max=true;s.pause_native_max=true;}
 let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:800,height:600}),maximized:Some(false),..Default::default()}));
 assert!(poll_pinned(f.as_mut()).is_pending());assert!(!w.0.lock().unwrap().operations.iter().any(|x|x=="resize"));
 w.0.lock().unwrap().pause_native_max=false;assert!(poll_pinned(f.as_mut()).is_ready());
 let s=w.0.lock().unwrap();assert!(s.operations.iter().position(|x|x=="wait-max-false").unwrap()<s.operations.iter().position(|x|x=="resize").unwrap());
}
#[cfg(feature = "linux")]
#[test] fn native_bounds_ack_precedes_capture_and_hide() {
 let w=WebviewWindow::default(); {let mut s=w.0.lock().unwrap();s.pause_native_bounds=true;s.shown=true;}
 let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:800,height:600}),visible:Some(false),..Default::default()}));
 assert!(poll_pinned(f.as_mut()).is_pending());{let s=w.0.lock().unwrap();assert_eq!(s.capture_count,0);assert!(s.shown);}
 w.0.lock().unwrap().pause_native_bounds=false;assert!(poll_pinned(f.as_mut()).is_ready());
 let s=w.0.lock().unwrap();assert_eq!(s.capture_count,1);assert!(!s.shown);assert!(s.operations.iter().position(|x|x=="wait-bounds").unwrap()<s.operations.iter().position(|x|x=="capture").unwrap());
}
#[cfg(feature = "linux")]
#[test] fn native_completion_errors_propagate_without_late_visibility() {
 for case in ["wait-max-false","wait-max-true","wait-size","wait-bounds","wait-fullscreen-false"] {
  let w=WebviewWindow::default();w.0.lock().unwrap().fail=Some(case);
  let result=block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:800,height:600}),fullscreen:Some(false),maximized:Some(true),visible:Some(true),..Default::default()}));
  assert_eq!(format!("{:?}",result.unwrap_err()),format!("Io(\"{case} failure\")"));
  let s=w.0.lock().unwrap();assert!(!s.shown);if case!="wait-max-true"{assert_eq!(s.capture_count,0);}
 }
}
#[cfg(feature = "linux")]
#[test] fn accepts_bounds_error_preserves_geometry() {
 let w=WebviewWindow::default();w.0.lock().unwrap().fail=Some("accepts");
 let result=block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:800,height:600}),..Default::default()}));
 assert_eq!(format!("{:?}",result.unwrap_err()),"Io(\"accepts failure\")");assert!(!w.0.lock().unwrap().operations.iter().any(|x|x=="resize"));
}
#[cfg(feature = "linux")]
#[test] fn fullscreen_bounds_are_ignored_until_explicit_exit() {
 let w=WebviewWindow::default();{let mut s=w.0.lock().unwrap();s.fullscreen=true;s.width=800;s.height=600;}
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:700,height:500}),..Default::default()})).unwrap();
 {let s=w.0.lock().unwrap();assert_eq!((s.width,s.height),(800,600));assert_eq!(s.capture_count,0);}
 block_on(window_apply_state(w.clone(),WindowStatePatch{fullscreen:Some(false),bounds:Some(WindowBounds{x:1,y:2,width:700,height:500}),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap();assert!(!s.fullscreen);assert_eq!((s.width,s.height),(700,500));assert_eq!(s.capture_count,1);
}
#[cfg(feature = "linux")]
#[test] fn cancellation_while_waiting_for_bounds_does_not_capture_old_state() {
 let w=WebviewWindow::default();w.0.lock().unwrap().pause_native_bounds=true;
 {let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:800,height:600}),visible:Some(false),..Default::default()}));assert!(poll_pinned(f.as_mut()).is_pending());}
 assert_eq!(w.0.lock().unwrap().capture_count,0);w.0.lock().unwrap().pause_native_bounds=false;
 block_on(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:3,y:4,width:700,height:500}),..Default::default()})).unwrap();
 let s=w.0.lock().unwrap();assert_eq!(s.capture_count,1);assert_eq!((s.cached_width,s.cached_height),(700,500));
}
#[cfg(feature = "linux")]
#[test] fn final_native_maximize_ack_precedes_visibility() {
 let w=WebviewWindow::default();w.0.lock().unwrap().pause_native_max=true;
 let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{maximized:Some(true),visible:Some(true),..Default::default()}));assert!(poll_pinned(f.as_mut()).is_pending());assert!(!w.0.lock().unwrap().shown);
 w.0.lock().unwrap().pause_native_max=false;assert!(poll_pinned(f.as_mut()).is_ready());assert!(w.0.lock().unwrap().shown);
}
#[cfg(feature = "linux")]
#[test] fn fullscreen_ack_precedes_bounds_application() {
 let w=WebviewWindow::default();{let mut s=w.0.lock().unwrap();s.fullscreen=true;s.pause_native_fullscreen=true;}
 let mut f=pin!(window_apply_state(w.clone(),WindowStatePatch{fullscreen:Some(false),bounds:Some(WindowBounds{x:1,y:2,width:700,height:500}),..Default::default()}));assert!(poll_pinned(f.as_mut()).is_pending());assert!(!w.0.lock().unwrap().operations.iter().any(|x|x=="resize"));
 w.0.lock().unwrap().pause_native_fullscreen=false;assert!(poll_pinned(f.as_mut()).is_ready());
}
`
const pendingTests = String.raw`
#[cfg(feature = "macos")]
#[test] fn show_waits_for_pending_async_resize() {
 let w=WebviewWindow::default(); w.0.lock().unwrap().pause_resize=true;
 let mut future=pin!(window_apply_state(w.clone(),WindowStatePatch{bounds:Some(WindowBounds{x:1,y:2,width:333,height:222}),visible:Some(true),..Default::default()}));
 assert!(poll_pinned(future.as_mut()).is_pending()); assert!(!w.0.lock().unwrap().shown);
 w.0.lock().unwrap().pause_resize=false; assert!(poll_pinned(future.as_mut()).is_ready()); assert!(w.0.lock().unwrap().shown);
}
#[test] fn show_waits_for_pending_queued_maximize() {
 let w=WebviewWindow::default(); w.maximize().unwrap(); w.0.lock().unwrap().pause_queue=true;
 let mut future=pin!(window_apply_state(w.clone(),WindowStatePatch{visible:Some(true),..Default::default()}));
 assert!(poll_pinned(future.as_mut()).is_pending()); assert!(!w.0.lock().unwrap().shown);
 w.0.lock().unwrap().pause_queue=false; assert!(poll_pinned(future.as_mut()).is_ready()); assert_eq!(w.0.lock().unwrap().shown_max,Some(true));
}
#[test] fn dropping_pending_queue_wait_then_next_command_flushes_it() {
 let w=WebviewWindow::default(); w.maximize().unwrap(); w.0.lock().unwrap().pause_queue=true;
 { let mut abandoned=pin!(window_apply_state(w.clone(),WindowStatePatch{maximized:Some(false),..Default::default()})); assert!(poll_pinned(abandoned.as_mut()).is_pending()); }
 w.0.lock().unwrap().pause_queue=false;
 let mut next=pin!(window_apply_state(w.clone(),WindowStatePatch{maximized:Some(false),..Default::default()})); assert!(poll_pinned(next.as_mut()).is_ready());
 assert!(!w.0.lock().unwrap().max);
}
`
fixture += extraTests + '\n' + command + '\n' + linuxTests
fixture = fixture.replace('fn block_on<F: Future>(f: F) -> F::Output {', 'fn poll_pinned<F: Future>(mut f: std::pin::Pin<&mut F>) -> Poll<F::Output> { let w = Waker::from(Arc::new(Noop)); let mut cx = Context::from_waker(&w); f.as_mut().poll(&mut cx) }\nfn block_on<F: Future>(f: F) -> F::Output {')
fixture += pendingTests + nativeCompletionTests

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-window-apply-geometry-'))
try {
    fs.writeFileSync(path.join(temporary, 'Cargo.toml'), '[package]\nname="window-apply-geometry-harness"\nversion="0.1.0"\nedition="2021"\n\n[features]\ndefault=[]\nmacos=[]\nwindows=[]\nlinux=[]\n')
    fs.mkdirSync(path.join(temporary, 'src'))
    fs.writeFileSync(path.join(temporary, 'src/lib.rs'), fixture)
    const cargo = process.env.CARGO || 'cargo'
    for (const feature of ['macos', 'windows', 'linux']) {
        const result = spawnSync(cargo, ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml'), '--features', feature, '--', '--test-threads=1'], { encoding: 'utf8', timeout: 60000 })
        assert.equal(result.status, 0, `${feature}: ${result.error ?? ''}\n${result.stdout ?? ''}${result.stderr ?? ''}`)
        const expected = feature === 'macos' ? 16 : feature === 'windows' ? 15 : 34
        assert.match(result.stdout, new RegExp(`test result: ok\\. ${expected} passed; 0 failed`))
        process.stdout.write(`feature=${feature}\n${result.stdout}`)
    }
} finally {
    fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    for (const [file, expected] of fingerprints) assert.equal(hash(file), expected, `source fingerprint unchanged: ${file}`)
}
