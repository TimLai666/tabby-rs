//! Wayland regression for completion waits while another mapped Wry window is fullscreen.
//! This checks native GTK state and allocation, not rendered UI or desktop acceptance.
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_completion_occluded_linux: skipped outside Linux");
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_bounds_linux.rs"]
mod bounds;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_completion_linux.rs"]
mod completion;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_geometry_linux.rs"]
mod geometry;
#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;

#[cfg(target_os = "linux")]
mod desktop {
    #[derive(Debug, Clone, Copy, Default)]
    pub(crate) struct WindowBounds {
        pub x: i32,
        pub y: i32,
        pub width: u32,
        pub height: u32,
    }
}

#[cfg(target_os = "linux")]
mod error {
    #[derive(Debug, Clone, PartialEq, Eq)]
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
mod platform {
    pub(crate) use crate::bounds::{set_window_position, window_bounds};
    pub(crate) use crate::saved_native as window_state_linux;
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_state_linux.rs"]
pub(crate) mod saved_native;

#[cfg(target_os = "linux")]
fn main() {
    use std::{
        sync::{
            atomic::{AtomicI32, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: window_completion_occluded_linux [--without-peer]\nExercises real Wry/GTK Linux completion helpers. Default mode maps a fullscreen peer and observes whether it suppresses the main window frame clock before checking no-op and resize completion. --without-peer is the active-redraw control. Requires a working Linux display; 40-second watchdog. Native primitive regression only, not rendered UI or desktop acceptance.");
        return;
    }
    let without_peer = args == ["--without-peer"];
    if !args.is_empty() && !without_peer {
        eprintln!("unknown argument; use --help");
        std::process::exit(2);
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(40));
        eprintln!("window_completion_occluded_linux: watchdog expired");
        std::process::exit(2);
    });

    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().to_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("io.tabbyrs.tests.occluded.{}", std::process::id());
    let outcome = Arc::new(AtomicI32::new(2));
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            use gtk::prelude::*;
            use tauri::Manager;

            let create = |label: &'static str, width: f64, height: f64| {
                tauri::WebviewWindowBuilder::new(
                    app,
                    label,
                    tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                )
                .inner_size(width, height)
                .min_inner_size(400., 300.)
                .visible(false)
                .incognito(true)
                .data_directory(root.join(label))
                .build()
                .unwrap()
            };
            let main_window = create("occluded-main", 800., 600.);
            let peer = (!without_peer).then(|| create("occluded-peer", 700., 500.));
            geometry::initialize(&main_window).unwrap();
            if let Some(peer) = peer.as_ref() {
                geometry::initialize(peer).unwrap();
            }
            let scale = main_window.gtk_window().unwrap().scale_factor() as f64;
            let paint_count = Arc::new(AtomicUsize::new(0));
            let observed = paint_count.clone();
            let native = main_window.gtk_window().unwrap();
            native.realize();
            let clock = native.frame_clock().expect("main GTK frame clock");
            clock.connect_after_paint(move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
            });

            let failures = Arc::new(Mutex::new(Vec::<String>::new()));
            let failures_task = failures.clone();
            let done = result.clone();
            let main_task = main_window.clone();
            let peer_task = peer.clone();
            let cleanup = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::time::Instant;
                async fn bounded<T>(
                    future: impl std::future::Future<Output = T>,
                ) -> Result<T, &'static str> {
                    tokio::time::timeout(Duration::from_secs(3), future)
                        .await
                        .map_err(|_| "3s step watchdog expired")
                }
                fn log_phase(
                    name: &str,
                    pass: bool,
                    detail: &str,
                    main: &tauri::WebviewWindow,
                    peer: Option<&tauri::WebviewWindow>,
                    failures: &Arc<Mutex<Vec<String>>>,
                ) {
                    let main_state = platform::window_state_linux::on_main_thread(main, |w| {
                        use gtk::prelude::*;
                        let native = w.gtk_window().map_err(|e| error::AppError::Io(e.to_string()))?;
                        let body = w.default_vbox().map_err(|e| error::AppError::Io(e.to_string()))?;
                        Ok((native.is_realized(), native.is_mapped(), native.is_maximized(),
                            native.window().is_some_and(|s| s.state().contains(gtk::gdk::WindowState::FULLSCREEN)),
                            native.size(), native.allocation(), body.allocation()))
                    });
                    let main_bounds = platform::window_bounds(main);
                    let peer_state = peer.map(|p| platform::window_state_linux::on_main_thread(p, |w| {
                        use gtk::prelude::*;
                        let native = w.gtk_window().map_err(|e| error::AppError::Io(e.to_string()))?;
                        Ok((native.is_mapped(), native.window().is_some_and(|s| s.state().contains(gtk::gdk::WindowState::FULLSCREEN))))
                    }));
                    println!("phase={name} pass={pass} detail={detail} peer={peer_state:?} main={main_state:?} bounds={main_bounds:?}");
                    if !pass { failures.lock().unwrap().push(name.to_owned()); }
                }
                async fn wait_paints(counter: &AtomicUsize, target: usize, deadline: Duration) -> bool {
                    let until = Instant::now() + deadline;
                    while Instant::now() < until {
                        if counter.load(Ordering::SeqCst) >= target { return true; }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    counter.load(Ordering::SeqCst) >= target
                }

                let _ = main_task.show();
                let initial = wait_paints(&paint_count, 1, Duration::from_secs(2)).await;
                log_phase("initial-main-frame", initial, &format!("paint_count={}", paint_count.load(Ordering::SeqCst)), &main_task, peer_task.as_ref(), &failures_task);
                let initial_target = tauri::PhysicalSize::new(800, 600);
                let initial_set = completion::set_size(&main_task, initial_target);
                let initial_wait = bounded(completion::wait_for_size(&main_task, initial_target)).await;
                log_phase("main-normal-800x600", initial_set.is_ok() && initial_wait.as_ref().is_ok_and(|r| r.is_ok()), &format!("set={initial_set:?} wait={initial_wait:?}"), &main_task, peer_task.as_ref(), &failures_task);
                let mut peer_fullscreen = false;
                if let Some(peer) = peer_task.as_ref() {
                    let shown = peer.show();
                    let request = peer.set_fullscreen(true);
                    let ack = bounded(completion::wait_for_fullscreen(peer, true)).await;
                    peer_fullscreen = shown.is_ok() && request.is_ok() && ack.as_ref().is_ok_and(|v| v.is_ok());
                    log_phase("peer-fullscreen-ack", peer_fullscreen, &format!("show={shown:?} request={request:?} ack={ack:?}"), &main_task, Some(peer), &failures_task);
                    let main_mapped = platform::window_state_linux::on_main_thread(&main_task, |w| {
                        use gtk::prelude::*;
                        Ok(w.gtk_window().map_err(|e| error::AppError::Io(e.to_string()))?.is_mapped())
                    });
                    log_phase("main-remains-mapped", main_mapped.as_ref().is_ok_and(|v| *v), &format!("mapped={main_mapped:?}"), &main_task, Some(peer), &failures_task);
                    let before = paint_count.load(Ordering::SeqCst);
                    let queued = platform::window_state_linux::on_main_thread(&main_task, |w| {
                        use gtk::prelude::*;
                        w.gtk_window().map(|native| native.queue_draw()).map_err(|e| error::AppError::Io(e.to_string()))
                    });
                    let advanced = wait_paints(&paint_count, before + 1, Duration::from_millis(200)).await;
                    let paused = queued.is_ok() && !advanced;
                    log_phase("covered-redraw-pause-observed", paused, &format!("queued={queued:?} before={before} after={} observed_no_paint_for_200ms={}", paint_count.load(Ordering::SeqCst), !advanced), &main_task, Some(peer), &failures_task);
                }

                let initial_bounds = platform::window_bounds(&main_task);
                let initial_size = initial_bounds.as_ref().map(|(_, s)| *s).unwrap_or_default();
                let same_size = initial_target;
                let size_wait = bounded(completion::wait_for_size(&main_task, same_size)).await;
                log_phase("normal-size-noop", initial_size == same_size && size_wait.as_ref().is_ok_and(|r| r.is_ok()), &format!("initial={initial_bounds:?} wait={size_wait:?}"), &main_task, peer_task.as_ref(), &failures_task);
                let max_wait = bounded(completion::wait_for_maximized(&main_task, false)).await;
                log_phase("maximized-false-noop", max_wait.as_ref().is_ok_and(|r| r.is_ok()), &format!("wait={max_wait:?}"), &main_task, peer_task.as_ref(), &failures_task);
                let fullscreen_wait = bounded(completion::wait_for_fullscreen(&main_task, false)).await;
                log_phase("fullscreen-false-noop", fullscreen_wait.as_ref().is_ok_and(|r| r.is_ok()), &format!("wait={fullscreen_wait:?}"), &main_task, peer_task.as_ref(), &failures_task);
                let same_bounds = initial_bounds.as_ref().map(|(p, s)| desktop::WindowBounds { x: p.x, y: p.y, width: s.width, height: s.height }).unwrap_or_default();
                let bounds_wait = bounded(completion::wait_for_bounds(&main_task, same_bounds)).await;
                log_phase("normal-bounds-noop", initial_bounds.is_ok() && bounds_wait.as_ref().is_ok_and(|r| r.is_ok()), &format!("target={same_bounds:?} wait={bounds_wait:?}"), &main_task, peer_task.as_ref(), &failures_task);

                let resize = tauri::PhysicalSize::new(900, 650);
                let set = completion::set_size(&main_task, resize);
                let resize_wait = bounded(completion::wait_for_size(&main_task, resize)).await;
                log_phase("resize-size-completion", set.is_ok() && resize_wait.as_ref().is_ok_and(|r| r.is_ok()), &format!("set={set:?} wait={resize_wait:?}"), &main_task, peer_task.as_ref(), &failures_task);
                let before_bounds = platform::window_bounds(&main_task);
                let target = before_bounds.as_ref().map(|(p, _)| desktop::WindowBounds { x: p.x, y: p.y, width: resize.width, height: resize.height }).unwrap_or_default();
                let bounds_wait = bounded(completion::wait_for_bounds(&main_task, target)).await;
                let native = platform::window_state_linux::on_main_thread(&main_task, |w| {
                    use gtk::prelude::*;
                    let native = w.gtk_window().map_err(|e| error::AppError::Io(e.to_string()))?;
                    let body = w.default_vbox().map_err(|e| error::AppError::Io(e.to_string()))?;
                    Ok((native.size(), native.allocation(), body.allocation(), native.scale_factor()))
                });
                let scale_now = native.as_ref().map(|v| v.3 as f64).unwrap_or(scale);
                let expected = tauri::PhysicalSize::new((900.0 / scale_now).round() as i32, (650.0 / scale_now).round() as i32);
                let actual_bounds = platform::window_bounds(&main_task);
                let expected_physical = tauri::PhysicalSize::new((expected.width as f64 * scale_now).round() as u32, (expected.height as f64 * scale_now).round() as u32);
                let allocation_ok = native.as_ref().is_ok_and(|(size, outer, body, _)| {
                    *size == (expected.width, expected.height)
                        && body.width() == expected.width && body.height() == expected.height
                        && body.x() >= 0 && body.y() >= 0
                        && outer.width() >= body.x() + body.width()
                        && outer.height() >= body.y() + body.height()
                });
                let bounds_ok = actual_bounds.as_ref().is_ok_and(|(_, s)| *s == expected_physical);
                let peer_still_fullscreen = match peer_task.as_ref() {
                    None => without_peer,
                    Some(peer) => peer_fullscreen && platform::window_state_linux::on_main_thread(peer, |w| {
                        use gtk::prelude::*;
                        let native = w.gtk_window().map_err(|e| error::AppError::Io(e.to_string()))?;
                        Ok(native.is_mapped() && native.window().is_some_and(|s| s.state().contains(gtk::gdk::WindowState::FULLSCREEN)))
                    }).is_ok_and(|v| v),
                };
                log_phase("resize-bounds-and-content-allocation", bounds_wait.as_ref().is_ok_and(|r| r.is_ok()) && bounds_ok && allocation_ok && peer_still_fullscreen,
                    &format!("target={target:?} wait={bounds_wait:?} bounds={actual_bounds:?} allocation={native:?} expected_logical={expected:?} expected_physical={expected_physical:?} peer_still_fullscreen={peer_still_fullscreen}"), &main_task, peer_task.as_ref(), &failures_task);

                let _ = main_task.destroy();
                if let Some(peer) = peer_task.as_ref() { let _ = peer.destroy(); }
                let deadline = Instant::now() + Duration::from_secs(2);
                let mut removed = false;
                while Instant::now() < deadline {
                    removed = cleanup.get_webview_window("occluded-main").is_none()
                        && cleanup.get_webview_window("occluded-peer").is_none();
                    if removed { break; }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                if !removed { failures_task.lock().unwrap().push("window-cleanup".into()); }
                let failures = failures_task.lock().unwrap();
                println!("window_completion_occluded_linux completed: failures={} without_peer={} scale={}", failures.len(), without_peer, scale);
                for failure in failures.iter() { eprintln!("FAILED: {failure}"); }
                done.store(i32::from(!failures.is_empty()), Ordering::SeqCst);
            });

            let handle = app.handle().clone();
            let done = result.clone();
            gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
                let value = done.load(Ordering::SeqCst);
                if value != 2 { handle.exit(value); gtk::glib::ControlFlow::Break }
                else { gtk::glib::ControlFlow::Continue }
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    let exit_outcome = outcome.clone();
    app.run_return(move |_, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if exit_outcome.load(Ordering::SeqCst) == 2 {
                api.prevent_exit();
            }
        }
    });
    drop(owned);
    std::process::exit(outcome.load(Ordering::SeqCst));
}
