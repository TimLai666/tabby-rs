// Framework and native queue boundaries are simulated; the command is appended verbatim by the runner.
#![allow(dead_code)]
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};
extern crate self as tauri;
#[derive(Debug)]
enum AppError {
    Io(String),
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
fn io_error(e: impl std::fmt::Display) -> AppError {
    AppError::Io(e.to_string())
}
#[derive(Clone, Copy)]
struct PhysicalSize<T> {
    width: T,
    height: T,
}
impl<T> PhysicalSize<T> {
    fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}
#[derive(Clone, Copy)]
struct PhysicalPosition<T> {
    x: T,
    y: T,
}
impl<T> PhysicalPosition<T> {
    fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}
#[derive(Default)]
struct WindowStatePatch {
    frame: Option<String>,
    title: Option<String>,
    always_on_top: Option<bool>,
    fullscreen: Option<bool>,
    maximized: Option<bool>,
    bounds: Option<WindowBounds>,
    progress: Option<Option<f64>>,
    color_scheme: Option<ColorScheme>,
    opacity: Option<f64>,
    vibrancy: Option<()>,
    visible: Option<bool>,
}
enum ColorScheme {
    System,
    Light,
    Dark,
}
enum Theme {
    Light,
    Dark,
}
enum ProgressBarStatus {
    Normal,
    None,
}
struct ProgressBarState {
    status: Option<ProgressBarStatus>,
    progress: Option<u64>,
}
#[derive(Default)]
struct Snapshot {
    max: bool,
    width: u32,
    height: u32,
    shown: bool,
    pending: Vec<bool>,
    shown_max: Option<bool>,
}
#[derive(Clone, Default)]
struct WebviewWindow(Arc<Mutex<Snapshot>>);
impl WebviewWindow {
    fn maximize(&self) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        if !s.max {
            s.pending.push(true);
        }
        Ok(())
    }
    fn unmaximize(&self) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        if s.max {
            s.pending.push(false);
        }
        Ok(())
    }
    fn set_title(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_always_on_top(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn set_fullscreen(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn set_progress_bar(&self, _: ProgressBarState) -> Result<(), String> {
        Ok(())
    }
    fn set_theme(&self, _: Option<Theme>) -> Result<(), String> {
        Ok(())
    }
    fn show(&self) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.shown = true;
        s.shown_max = Some(s.max);
        Ok(())
    }
    fn hide(&self) -> Result<(), String> {
        self.0.lock().unwrap().shown = false;
        Ok(())
    }
}
struct Caps {
    absolute_positioning: bool,
}
fn window_capabilities(_: &WebviewWindow) -> Result<Caps, AppError> {
    Ok(Caps {
        absolute_positioning: true,
    })
}
fn set_bounds_size(w: &WebviewWindow, s: PhysicalSize<u32>) -> Result<(), AppError> {
    let mut v = w.0.lock().unwrap();
    v.max = false;
    v.width = s.width;
    v.height = s.height;
    Ok(())
}
fn set_bounds_position(_: &WebviewWindow, _: PhysicalPosition<i32>) -> Result<(), AppError> {
    Ok(())
}
mod platform {
    use super::*;
    pub fn set_window_frame(_: &WebviewWindow, _: &str) -> Result<(), AppError> {
        Ok(())
    }
    pub fn set_window_opacity(_: &WebviewWindow, _: f64) -> Result<(), AppError> {
        Ok(())
    }
    pub fn set_window_vibrancy(_: &WebviewWindow, _: &()) -> Result<(), AppError> {
        Ok(())
    }
    pub async fn wait_for_window_operations(w: &WebviewWindow) -> Result<(), AppError> {
        let mut s = w.0.lock().unwrap();
        let work = std::mem::take(&mut s.pending);
        for state in work {
            s.max = state;
        }
        Ok(())
    }
    pub async fn set_window_outer_size_async(
        w: &WebviewWindow,
        s: PhysicalSize<u32>,
    ) -> Result<(), AppError> {
        set_bounds_size(w, s)
    }
}
struct Noop;
impl Wake for Noop {
    fn wake(self: Arc<Self>) {}
}
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let w = Waker::from(Arc::new(Noop));
    let mut cx = Context::from_waker(&w);
    match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("unexpected pending in immediate Windows boundary"),
    }
}
#[test]
fn combined_bounds_and_maximized_preserve_requested_final_state() {
    let w = WebviewWindow::default();
    let p = WindowStatePatch {
        bounds: Some(WindowBounds {
            x: 30,
            y: 40,
            width: 900,
            height: 650,
        }),
        maximized: Some(true),
        visible: Some(true),
        ..Default::default()
    };
    block_on(window_apply_state(w.clone(), p)).unwrap();
    let state = w.0.lock().unwrap();
    assert!(
        state.max,
        "production command lost requested maximized=true after Windows outer resize"
    );
    assert_eq!((state.width, state.height), (900, 650));
    assert!(state.shown);
}

struct AppState;
static STATE: AppState = AppState;
impl AppState {
    async fn lock_window_operations(&self) {}
}
impl WebviewWindow {
    fn state<T>(&self) -> &'static AppState {
        &STATE
    }
}
#[test]
fn cancelled_queued_zoom_settles_before_next_unmaximize() {
    let w = WebviewWindow::default();
    w.maximize().unwrap();
    assert!(!w.0.lock().unwrap().max);
    block_on(window_apply_state(
        w.clone(),
        WindowStatePatch {
            maximized: Some(false),
            ..Default::default()
        },
    ))
    .unwrap();
    block_on(platform::wait_for_window_operations(&w)).unwrap();
    assert!(
        !w.0.lock().unwrap().max,
        "pending previous Tao maximize overrode next unmaximize"
    );
}
#[test]
fn reveal_waits_for_previously_queued_zoom() {
    let w = WebviewWindow::default();
    w.maximize().unwrap();
    block_on(window_apply_state(
        w.clone(),
        WindowStatePatch {
            visible: Some(true),
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(
        w.0.lock().unwrap().shown_max,
        Some(true),
        "show ran before the preceding native zoom"
    );
}
