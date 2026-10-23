//! Real Wry/Wayland fixture for the production Linux restore/capture controller,
//! native adapter, and per-window tracker. AppState and durable storage are fixture
//! boundaries; application startup/close and rendered desktop behavior are not covered.
#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;

#[cfg(target_os = "linux")]
#[path = "../src/window_persistence_linux.rs"]
mod window_persistence_linux;

#[cfg(target_os = "linux")]
mod error {
    #[derive(Debug)]
    pub(crate) enum AppError {
        Io(String),
        InvalidArgument(String),
        Unsupported(String),
    }

    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(message) | Self::InvalidArgument(message) | Self::Unsupported(message) => {
                    f.write_str(message)
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod state {
    use crate::{
        error::AppError,
        window_geometry::persistence::{LogicalBounds, SavedWindowState, WindowStates},
    };
    use std::sync::Mutex;

    /// Fixture boundary replacing only production AppState and its durable state source.
    pub(crate) struct AppState {
        states: Mutex<WindowStates>,
        seed: SavedWindowState,
    }

    impl AppState {
        pub(crate) fn new(seed: SavedWindowState) -> Self {
            Self {
                states: Mutex::new(WindowStates::default()),
                seed,
            }
        }

        pub(crate) fn begin_window_state(&self, label: &str) -> Result<SavedWindowState, AppError> {
            if self.states.lock().unwrap().begin(label, self.seed) {
                Ok(self.seed)
            } else {
                Err(AppError::InvalidArgument(format!(
                    "window state already registered for {label}"
                )))
            }
        }

        pub(crate) fn ready_window_state(&self, label: &str) {
            self.states.lock().unwrap().ready(label);
        }

        pub(crate) fn ready_window_state_with_sample(&self, label: &str, sample: LogicalBounds) {
            self.states.lock().unwrap().ready_with_sample(label, sample);
        }

        pub(crate) fn ready_window_state_with_origin(&self, label: &str, sample: LogicalBounds) {
            self.states.lock().unwrap().ready_with_origin(label, sample);
        }

        pub(crate) fn window_sampling_bounds(&self, label: &str) -> Option<LogicalBounds> {
            self.states.lock().unwrap().sampling_bounds(label)
        }

        pub(crate) fn record_window_state(&self, label: &str, sample: SavedWindowState) {
            if let Some(bounds) = sample.bounds {
                self.states
                    .lock()
                    .unwrap()
                    .record(label, bounds, sample.maximized);
            }
        }

        pub(crate) fn remove_window_state(&self, label: &str) {
            self.states.lock().unwrap().remove(label);
        }

        pub(crate) fn tracked_window_state(
            &self,
            label: &str,
            maximized: bool,
        ) -> Option<SavedWindowState> {
            self.states.lock().unwrap().for_close(label, maximized)
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_state_linux.rs"]
pub(crate) mod window_state_linux;

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_bounds_linux.rs"]
mod bounds;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_geometry_linux.rs"]
mod geometry;

#[cfg(target_os = "linux")]
mod platform {
    pub(crate) use crate::bounds::{
        set_window_position, supports_absolute_positioning, window_bounds,
    };
    use crate::error::AppError;
    pub(crate) use crate::geometry::initialize as initialize_window_geometry;
    pub(crate) use crate::window_state_linux;

    /// Fixture boundary matching the production main-thread oneshot operation barrier.
    pub(crate) async fn wait_for_window_operations(
        window: &tauri::WebviewWindow,
    ) -> Result<(), AppError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(());
            })
            .map_err(|error| AppError::Io(error.to_string()))?;
        receiver
            .await
            .map_err(|error| AppError::Io(error.to_string()))
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_capture_wayland: skipped outside Linux");
}

#[cfg(target_os = "linux")]
fn main() {
    use gtk::prelude::*;
    use std::{
        sync::{
            atomic::{AtomicI32, AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        time::Duration,
    };
    use window_geometry::persistence::{LogicalBounds, SavedWindowState};

    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: window_capture_wayland [--stock-control] [--seed-offscreen]\nExercises the actual Linux restore_window/capture_window controller, native adapter, and per-window tracker through real Wayland GTK/Wry windows.\nUse one monitor with at least 1920x1080 logical pixels. AppState and its durable seed are fixture boundaries; application startup/close, durable disk persistence, rendered desktop behavior, and desktop parity are not verified.\n--stock-control uses the former ready operation and must fail.\n--seed-offscreen starts with saved offscreen bounds instead of absent bounds.");
        return;
    }
    if args
        .iter()
        .any(|arg| arg != "--stock-control" && arg != "--seed-offscreen")
    {
        eprintln!("unknown argument; use --help");
        std::process::exit(2);
    }
    let stock = args.iter().any(|arg| arg == "--stock-control");
    let seeded = args.iter().any(|arg| arg == "--seed-offscreen");
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        eprintln!("window_capture_wayland: watchdog");
        std::process::exit(2);
    });
    let owned = tempfile::tempdir().expect("private WebView root");
    let root = owned.path().to_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = format!("io.tabbyrs.tests.capture.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let outcome = Arc::new(AtomicI32::new(2));
    let result = outcome.clone();
    let capture_errors = Arc::new(AtomicUsize::new(0));
    let app = tauri::Builder::default().setup(move |app| {
        let display = gtk::gdk::Display::default().expect("GTK display");
        assert_eq!(display.type_().name(), "GdkWaylandDisplay", "fixture requires the owning Wayland backend");
        assert_eq!(display.n_monitors(), 1, "fixture requires one monitor");
        let monitor = display.monitor(0).expect("fixture monitor").geometry();
        assert!(monitor.width() >= 1920 && monitor.height() >= 1080, "fixture desktop is too small");
        let constructor = LogicalBounds { x: monitor.x() + (monitor.width() - 800) / 2, y: monitor.y() + (monitor.height() - 600) / 2, width: 800, height: 600 };
        let resized = LogicalBounds { width: 900, height: 650, ..constructor };
        let seed = SavedWindowState { bounds: seeded.then_some(LogicalBounds { x: 5000, y: 4000, width: 800, height: 600 }), maximized: false };
        let state = Arc::new(state::AppState::new(seed));
        let mut windows = Vec::new();
        let inconsistent_normal_observations = Arc::new(AtomicUsize::new(0));
        let inconsistent_normal_details = Arc::new(Mutex::new(None::<String>));
        let after_default_errors = Arc::new(Mutex::new(Vec::<String>::new()));
        for (label, decorated) in [("capture-native", true), ("capture-frameless", false)] {
            let window = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::External("about:blank".parse()?))
                .inner_size(800.0, 600.0).min_inner_size(400.0, 300.0).center()
                .decorations(decorated).visible(false).incognito(true).data_directory(root.join(label)).build()?;
            let observed = window.clone();
            let events_state = state.clone();
            let event_errors = capture_errors.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    events_state.remove_window_state(observed.label());
                } else if matches!(event, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
                    if let Err(error) = window_persistence_linux::capture_window(&observed, &events_state) {
                        eprintln!("capture event {}: {error}", observed.label());
                        event_errors.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
            windows.push(window);
        }
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let worker_windows = windows.clone();
        let worker_state = state.clone();
        let startup_center = constructor;
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
                for window in &worker_windows {
                    platform::initialize_window_geometry(window).map_err(|e| format!("initialize {}: {e}", window.label()))?;
                    let raw = window_persistence_linux::restore_window(window, &worker_state, None)
                        .map_err(|e| format!("restore {}: {e}", window.label()))?;
                    assert_eq!(raw, startup_center, "raw constructor sample must match monitor-centered 800x600");
                    if stock {
                        worker_state.ready_window_state(window.label());
                    } else {
                        runtime.block_on(window_persistence_linux::enable_capture(window, &worker_state, raw))
                            .map_err(|e| format!("enable capture {}: {e}", window.label()))?;
                    }
                }
                Ok(())
            })();
            let _ = ready_tx.send(result);
        });
        let mut checks = 0;
        let mut failures = 0;
        let check = |phase: &str, actual: Option<SavedWindowState>, expected: Option<SavedWindowState>, checks: &mut usize, failures: &mut usize| {
            *checks += 1;
            *failures += usize::from(actual != expected);
            println!("capture {phase} actual={actual:?} expected={expected:?}");
        };
        let changed = LogicalBounds { width: 901, height: 651, ..constructor };
        let mut stage = 0;
        let mut stage_started = std::time::Instant::now();
        let handle = app.handle().clone();
        let mut ready = false;
        let mut after_default_handler: Option<gtk::glib::SignalHandlerId> = None;
        gtk::glib::timeout_add_local(Duration::from_millis(600), move || {
            if !ready {
                match ready_rx.try_recv() {
                    Ok(Ok(())) => ready = true,
                    Ok(Err(error)) => panic!("native controller setup failed: {error}"),
                    Err(mpsc::TryRecvError::Empty) => return gtk::glib::ControlFlow::Continue,
                    Err(error) => panic!("native controller setup worker stopped: {error}"),
                }
            }
            let readiness = match stage {
                1 => Some((false, Some((constructor.width as i32, constructor.height as i32)))),
                2 => Some((true, None)),
                3 => Some((false, Some((constructor.width as i32, constructor.height as i32)))),
                4 => Some((false, Some((resized.width as i32, resized.height as i32)))),
                5 => Some((true, None)),
                6 => Some((false, Some((resized.width as i32, resized.height as i32)))),
                7 => Some((false, Some((changed.width as i32, changed.height as i32)))),
                _ => None,
            };
            if let Some((target_maximized, target_size)) = readiness {
                let snapshot = windows[0].gtk_window()
                    .map_err(|error| error.to_string())
                    .and_then(|native| {
                        let maximized = native.is_maximized();
                        let native_size = native.size();
                        native.child()
                            .map(|child| {
                                let allocation = child.allocation();
                                (maximized, native_size, (allocation.width(), allocation.height()))
                            })
                            .ok_or_else(|| "Tauri default vbox child was missing".to_string())
                    });
                let settled = snapshot.as_ref().is_ok_and(|(maximized, native_size, allocation)| {
                    *maximized == target_maximized
                        && native_size == allocation
                        && target_size.map_or(true, |size| *native_size == size)
                });
                if !settled {
                    if stage_started.elapsed() < Duration::from_secs(2) {
                        return gtk::glib::ControlFlow::Continue;
                    }
                    failures += 1;
                    eprintln!("native stage readiness timeout stage={stage} target_maximized={target_maximized} target_size={target_size:?} native_body_state={snapshot:?}");
                }
            }
            match stage {
                0 => {
                    for window in &windows { window.show().unwrap(); }
                }
                1 => {
                    for window in &windows { check("unchanged-show", state.tracked_window_state(window.label(), false), Some(seed), &mut checks, &mut failures); }
                    windows[0].maximize().unwrap();
                }
                2 => {
                    check("maximized-keeps-seed", state.tracked_window_state(windows[0].label(), true), Some(SavedWindowState { maximized: true, ..seed }), &mut checks, &mut failures);
                    windows[0].unmaximize().unwrap();
                }
                3 => {
                    check("unmaximize-captures-constructor", state.tracked_window_state(windows[0].label(), false), Some(SavedWindowState { bounds: Some(constructor), maximized: false }), &mut checks, &mut failures);
                    windows[0].set_size(tauri::LogicalSize::new(900, 650)).unwrap();
                }
                4 => {
                    let expected_normal = Some(SavedWindowState { bounds: Some(resized), maximized: false });
                    let actual_normal = state.tracked_window_state(windows[0].label(), false);
                    check("real-resize", actual_normal, expected_normal, &mut checks, &mut failures);
                    check("peer-after-resize", state.tracked_window_state(windows[1].label(), false), Some(seed), &mut checks, &mut failures);
                    if actual_normal == expected_normal {
                        let observed = windows[0].clone();
                        let events_state = state.clone();
                        let observations = inconsistent_normal_observations.clone();
                        let details = inconsistent_normal_details.clone();
                        let errors = after_default_errors.clone();
                        let previous_normal = resized;
                        match observed.gtk_window() {
                            Ok(native) => {
                                after_default_handler = Some(native.connect_local("window-state-event", true, move |_| {
                                    let native = match observed.gtk_window() {
                                        Ok(native) => native,
                                        Err(error) => {
                                            if let Ok(mut errors) = errors.lock() { errors.push(format!("native GTK window lookup failed: {error}")); }
                                            return Some(false.to_value());
                                        }
                                    };
                                    let sample = match window_state_linux::read(&observed, previous_normal) {
                                        Ok(sample) => sample,
                                        Err(error) => {
                                            if let Ok(mut errors) = errors.lock() { errors.push(format!("native read failed in after-default signal: {error}")); }
                                            return Some(false.to_value());
                                        }
                                    };
                                    let child = match native.child() {
                                        Some(child) => child,
                                        None => {
                                            if let Ok(mut errors) = errors.lock() { errors.push("Tauri default vbox child was missing".into()); }
                                            return Some(false.to_value());
                                        }
                                    };
                                    let allocation = child.allocation();
                                    let allocation_size = (allocation.width().max(0) as u32, allocation.height().max(0) as u32);
                                    let sample_size = sample.bounds.map(|bounds| (bounds.width, bounds.height));
                                    let diagnostic = format!("maximized={} native_content={sample_size:?} default_vbox_allocation={allocation_size:?} previous_normal=({}, {})", native.is_maximized(), previous_normal.width, previous_normal.height);
                                    println!("after-default window-state-event {diagnostic}");
                                    if !native.is_maximized()
                                        && sample_size.is_some_and(|size| size != allocation_size)
                                        && sample.bounds != Some(previous_normal)
                                    {
                                        observations.fetch_add(1, Ordering::SeqCst);
                                        if let Ok(mut last) = details.lock() { *last = Some(diagnostic); }
                                        match window_persistence_linux::capture_window(&observed, &events_state) {
                                            Ok(()) => {
                                                let actual = events_state.tracked_window_state(observed.label(), false);
                                                let expected = Some(SavedWindowState { bounds: Some(previous_normal), maximized: false });
                                                if actual != expected {
                                                    if let Ok(mut errors) = errors.lock() {
                                                        errors.push(format!("inconsistent normal sample changed cached bounds: actual={actual:?} expected={expected:?}"));
                                                    }
                                                }
                                            }
                                            Err(error) => {
                                                if let Ok(mut errors) = errors.lock() { errors.push(format!("capture failed in inconsistent normal sample: {error}")); }
                                            }
                                        }
                                    }
                                    Some(false.to_value())
                                }));
                            }
                            Err(error) => {
                                failures += 1;
                                eprintln!("could not arm after-default window-state-event handler: {error}");
                            }
                        }
                    } else {
                        eprintln!("after-default handler not armed because 900x650 cache assertion failed");
                    }
                    windows[0].maximize().unwrap();
                }
                5 => {
                    let maximized = windows[0].gtk_window().map(|native| native.is_maximized()).unwrap_or(false);
                    if !maximized { failures += 1; }
                    println!("second maximize settled={maximized}");
                    windows[0].unmaximize().unwrap();
                }
                6 => {
                    let maximized = windows[0].gtk_window().map(|native| native.is_maximized()).unwrap_or(true);
                    if maximized { failures += 1; }
                    println!("second unmaximize settled={}", !maximized);
                    let observations = inconsistent_normal_observations.load(Ordering::SeqCst);
                    checks += 1;
                    if observations == 0 { failures += 1; }
                    println!("capture inconsistent-normal-observed count={observations} details={:?}", inconsistent_normal_details.lock().map(|details| details.clone()).unwrap_or_default());
                    let mut after_default_errors = after_default_errors.lock().map(|errors| errors.clone()).unwrap_or_default();
                    failures += after_default_errors.len();
                    for error in after_default_errors.drain(..) { eprintln!("after-default regression failure: {error}"); }
                    check("unmaximize-keeps-allocated-normal", state.tracked_window_state(windows[0].label(), false), Some(SavedWindowState { bounds: Some(resized), maximized: false }), &mut checks, &mut failures);
                    if let Some(handler) = after_default_handler.take() {
                        match windows[0].gtk_window() {
                            Ok(native) => native.disconnect(handler),
                            Err(error) => {
                                failures += 1;
                                eprintln!("could not disconnect after-default window-state-event handler: {error}");
                            }
                        }
                    }
                    windows[0].set_size(tauri::LogicalSize::new(901, 651)).unwrap();
                }
                7 => {
                    check("post-allocation-real-resize", state.tracked_window_state(windows[0].label(), false), Some(SavedWindowState { bounds: Some(changed), maximized: false }), &mut checks, &mut failures);
                    check("peer-after-post-allocation-resize", state.tracked_window_state(windows[1].label(), false), Some(seed), &mut checks, &mut failures);
                    windows[0].destroy().unwrap();
                }
                8 => {
                    check("destroyed-owner", state.tracked_window_state(windows[0].label(), false), None, &mut checks, &mut failures);
                    check("surviving-peer", state.tracked_window_state(windows[1].label(), false), Some(seed), &mut checks, &mut failures);
                    let event_errors = capture_errors.load(Ordering::SeqCst);
                    failures += event_errors;
                    println!("window_capture_wayland completed: checks={checks} failures={failures} inconsistent_normal_observations={} capture_errors={event_errors} stock={stock} seeded={seeded}", inconsistent_normal_observations.load(Ordering::SeqCst));
                    result.store(i32::from(failures != 0), Ordering::SeqCst);
                    windows[1].destroy().unwrap();
                    handle.exit(i32::from(failures != 0));
                    return gtk::glib::ControlFlow::Break;
                }
                _ => unreachable!(),
            }
            stage += 1;
            stage_started = std::time::Instant::now();
            gtk::glib::ControlFlow::Continue
        });
        Ok(())
    }).build(context).expect("build isolated Wry capture fixture");
    app.run_return(|_, _| {});
    drop(owned);
    std::process::exit(outcome.load(std::sync::atomic::Ordering::SeqCst));
}
