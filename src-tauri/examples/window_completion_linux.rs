//! Native Linux completion primitive checks using real Wry/GTK windows.
//! These checks establish request completion against native state and bounds only.
//! They are not a complete application, rendered UI, packaging, or desktop acceptance.
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_completion_linux: skipped outside Linux");
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
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    pub(crate) use crate::saved_native::on_main_thread;

    // Keep the framework's real main-thread rendezvous from platform/mod.rs.
    pub(crate) async fn wait_for_window_operations(
        window: &tauri::WebviewWindow,
    ) -> Result<(), crate::error::AppError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(());
            })
            .map_err(|error| crate::error::AppError::Io(error.to_string()))?;
        receiver
            .await
            .map_err(|error| crate::error::AppError::Io(error.to_string()))
    }
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_state_linux.rs"]
pub(crate) mod saved_native;

#[cfg(target_os = "linux")]
fn main() {
    use std::{
        sync::{
            atomic::{AtomicI32, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: window_completion_linux\nExercises production completion primitives on real Native and frameless Wry Linux windows. Uses isolated incognito WebViews and a 30-second watchdog. Requires a working Linux display. Covers native bounds/state acknowledgement, hidden-window preservation, cancellation, destruction, and bounded declined requests. This is primitive-level evidence, not complete application, rendered UI, packaging, or desktop acceptance.");
        return;
    }
    if !args.is_empty() {
        eprintln!("unknown argument; use --help");
        std::process::exit(2);
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("window_completion_linux: watchdog expired");
        std::process::exit(2);
    });

    let owned = tempfile::tempdir().unwrap();
    let data_root = owned.path().to_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("io.tabbyrs.tests.completion.{}", std::process::id());
    let outcome = Arc::new(AtomicI32::new(2));
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            use gtk::prelude::*;
            use tauri::Manager;

            let display = gtk::gdk::Display::default();
            let x11 = display.as_ref().is_some_and(|d| d.downcast_ref::<gdkx11::X11Display>().is_some());
            let create = |label: &'static str, decorated: bool| {
                tauri::WebviewWindowBuilder::new(
                    app,
                    label,
                    tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                )
                .inner_size(700., 500.)
                .min_inner_size(400., 300.)
                .decorations(decorated)
                .visible(false)
                .incognito(true)
                .data_directory(data_root.join(label))
                .build()
                .unwrap()
            };
            let native = create("completion-native", true);
            let frameless = create("completion-frameless", false);
            let hidden_window = create("completion-never-shown", true);
            let scale = native.gtk_window().unwrap().scale_factor();
            let animations_before = gtk::Settings::default().map(|settings| settings.is_gtk_enable_animations());
            geometry::initialize(&native).unwrap();
            geometry::initialize(&frameless).unwrap();
            geometry::initialize(&hidden_window).unwrap();
            let animations_after = gtk::Settings::default().map(|settings| settings.is_gtk_enable_animations());
            let failures = Arc::new(Mutex::new(Vec::<String>::new()));
            let windows = vec![native, frameless, hidden_window];
            let failures_task = failures.clone();
            let done = result.clone();
            let task_windows = windows.clone();
            let cleanup_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::time::{timeout, Instant};
                async fn bounded<T>(future: impl std::future::Future<Output = T>) -> Result<T, &'static str> {
                    timeout(Duration::from_secs(4), future).await.map_err(|_| "step timed out")
                }
                fn expected(actual: Result<(tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>), error::AppError>, target: desktop::WindowBounds, scale: f64, x11: bool) -> bool {
                    actual.is_ok_and(|(p, s)| {
                        let w = ((target.width as f64 / scale).round() * scale).round() as u32;
                        let h = ((target.height as f64 / scale).round() * scale).round() as u32;
                        let x = ((target.x as f64 / scale).round() * scale).round() as i32;
                        let y = ((target.y as f64 / scale).round() * scale).round() as i32;
                        s.width == w && s.height == h && (!x11 || (p.x == x && p.y == y))
                    })
                }
                let scale = scale as f64;
                let mut case_count = 0usize;
                let mut run = |name: &str, pass: bool, detail: String| {
                    case_count += 1;
                    println!("completion {name} pass={pass} {detail}");
                    if !pass { failures_task.lock().unwrap().push(name.to_owned()); }
                };
                run("global-animation-preference-preserved", animations_before.is_some() && animations_before == animations_after, format!("before={animations_before:?} after={animations_after:?}"));
                for (index, window) in task_windows.iter().take(2).enumerate() {
                    let label = if index == 0 { "native" } else { "frameless" };
                    let show = window.show();
                    run(&format!("{label}-show"), show.as_ref().is_ok(), format!("{show:?}"));
                    let target = desktop::WindowBounds { x: 100, y: 110, width: 900, height: 650 };
                    let set = completion::set_size(window, tauri::PhysicalSize::new(target.width, target.height));
                    let position = platform::set_window_position(window, tauri::PhysicalPosition::new(target.x, target.y));
                    let wait = bounded(completion::wait_for_bounds(window, target)).await;
                    let actual = platform::window_bounds(window);
                    run(&format!("{label}-resize-wait-bounds"), set.as_ref().is_ok() && position.as_ref().is_ok() && wait.as_ref().is_ok_and(|r| r.is_ok()) && expected(actual.clone(), target, scale, x11), format!("set={set:?} position={position:?} wait={wait:?} actual={actual:?}"));

                    if x11 {
                        let moved = desktop::WindowBounds { x: 190, y: 210, ..target };
                        let position = platform::set_window_position(window, tauri::PhysicalPosition::new(moved.x, moved.y));
                        let wait = bounded(completion::wait_for_bounds(window, moved)).await;
                        let actual = platform::window_bounds(window);
                        run(&format!("{label}-position-wait"), position.as_ref().is_ok() && wait.as_ref().is_ok_and(|r| r.is_ok()) && expected(actual.clone(), moved, scale, true), format!("set={position:?} wait={wait:?} actual={actual:?}"));
                    } else {
                        run(&format!("{label}-position-wayland-skip"), true, "global origin is not asserted on Wayland".into());
                    }
                    let same = platform::window_bounds(window).ok().map(|(p,s)| desktop::WindowBounds{x:p.x,y:p.y,width:s.width,height:s.height});
                    let same_wait = same.map(|bounds| bounded(completion::wait_for_bounds(window, bounds)));
                    let same_ok = match same_wait { Some(f) => f.await.is_ok_and(|r| r.is_ok()), None => false };
                    run(&format!("{label}-same-bounds-noop"), same_ok, format!("done={same_ok}"));
                    let accepts_normal = completion::accepts_bounds(window);
                    run(&format!("{label}-accept-normal"), accepts_normal.as_ref().is_ok_and(|value| *value), format!("{accepts_normal:?}"));
                    if x11 {
                        let target = desktop::WindowBounds { x: 1920, y: 540, width: 1920, height: 1080 };
                        let size = tauri::PhysicalSize::new(target.width, target.height);
                        let resize = completion::set_size(window, size);
                        let size_wait = bounded(completion::wait_for_size(window, size)).await;
                        let before_move = platform::window_bounds(window);
                        let move_request = platform::set_window_position(window, tauri::PhysicalPosition::new(target.x,target.y));
                        let complete = bounded(completion::wait_for_bounds(window,target)).await;
                        let actual = platform::window_bounds(window);
                        run(&format!("{label}-size-completion-before-right-edge-move"), resize.is_ok() && size_wait.as_ref().is_ok_and(|r| r.is_ok()) && before_move.as_ref().is_ok_and(|(_, s)| *s == size) && move_request.is_ok() && complete.as_ref().is_ok_and(|r| r.is_ok()) && expected(actual.clone(),target,scale,true), format!("size-wait={size_wait:?} before-move={before_move:?} complete={complete:?} actual={actual:?}"));
                    }
                }

                let hidden = &task_windows[2];
                let hidden_initial = platform::window_bounds(hidden);
                let hidden_origin = hidden_initial.as_ref().map(|(p, _)| (p.x, p.y)).unwrap_or((0, 0));
                let hidden_max_wait = bounded(completion::wait_for_maximized(hidden, true)).await;
                let hidden_full_wait = bounded(completion::wait_for_fullscreen(hidden, true)).await;
                let hidden_state = platform::on_main_thread(hidden, |w| {
                    let native = w.gtk_window().map_err(|error| error::AppError::Io(error.to_string()))?;
                    Ok((native.is_realized(), native.is_mapped(), native.window().is_some()))
                });
                let hidden_unrealized = hidden_state.as_ref().is_ok_and(|(realized, mapped, _)| !*realized && !*mapped);
                run("hidden-maximize-wait-does-not-map", hidden_max_wait.as_ref().is_ok_and(|r| r.is_ok()) && hidden_unrealized, format!("wait={hidden_max_wait:?} native={hidden_state:?}"));
                run("hidden-fullscreen-wait-does-not-map", hidden_full_wait.as_ref().is_ok_and(|r| r.is_ok()) && hidden_unrealized, format!("wait={hidden_full_wait:?} native={hidden_state:?}"));
                for (name, size) in [("zero", tauri::PhysicalSize::new(0,0)), ("small", tauri::PhysicalSize::new(200,100))] {
                    let set = completion::set_size(hidden, size);
                    let size_wait = bounded(completion::wait_for_size(hidden, size)).await;
                    let target = desktop::WindowBounds { x: hidden_origin.0, y: hidden_origin.1, width: size.width, height: size.height };
                    let wait = bounded(completion::wait_for_bounds(hidden, target)).await;
                    let native_state = platform::on_main_thread(hidden, |w| {
                        let native = w.gtk_window().map_err(|error| error::AppError::Io(error.to_string()))?;
                        Ok((native.is_realized(), native.is_mapped(), native.window().is_some(), native.size()))
                    });
                    let minimum = platform::on_main_thread(hidden, |w| {
                        w.default_vbox()
                            .map(|body| body.size_request() == (400, 300))
                            .map_err(|error| error::AppError::Io(error.to_string()))
                    }).unwrap_or(false);
                    let effective = platform::window_bounds(hidden);
                    let effective_minimum = effective.as_ref().is_ok_and(|(_, size)| {
                        size.width == (400.0 * scale as f64).round() as u32
                            && size.height == (300.0 * scale as f64).round() as u32
                    });
                    run(&format!("hidden-{name}-minimum"), set.as_ref().is_ok() && size_wait.as_ref().is_ok_and(|r| r.is_ok()) && wait.as_ref().is_ok_and(|r| r.is_ok()) && minimum && effective_minimum, format!("set={set:?} wait={wait:?} content-minimum={minimum} effective={effective:?} native={native_state:?}"));
                    run(&format!("hidden-{name}-not-realized-or-mapped"), native_state.as_ref().is_ok_and(|(realized,mapped,_,_)| !*realized && !*mapped), format!("native={native_state:?}"));
                }

                for (state_index, window) in task_windows.iter().take(2).enumerate() {
                println!("state-flow frame={}", if state_index == 0 { "native" } else { "frameless" });
                window.show().ok();
                let max = window.maximize();
                let wait_max = bounded(completion::wait_for_maximized(window, true)).await;
                let raw_max = platform::on_main_thread(window, |w| {
                    w.gtk_window()
                        .map(|native| native.is_maximized())
                        .map_err(|error| error::AppError::Io(error.to_string()))
                });
                let accepts_max = completion::accepts_bounds(window);
                run("maximize-wait-native-state", max.as_ref().is_ok() && wait_max.as_ref().is_ok_and(|r| r.is_ok()) && raw_max.as_ref().is_ok_and(|value| *value), format!("request={max:?} wait={wait_max:?} native={raw_max:?}"));
                run("accept-bounds-maximized-false", accepts_max.as_ref().is_ok_and(|value| !*value), format!("{accepts_max:?}"));
                let unmax = window.unmaximize();
                let wait_unmax = bounded(completion::wait_for_maximized(window, false)).await;
                let raw_unmax = platform::on_main_thread(window, |w| {
                    w.gtk_window()
                        .map(|native| native.is_maximized())
                        .map_err(|error| error::AppError::Io(error.to_string()))
                });
                run("unmaximize-wait-native-state", unmax.as_ref().is_ok() && wait_unmax.as_ref().is_ok_and(|r| r.is_ok()) && raw_unmax.as_ref().is_ok_and(|value| !*value), format!("request={unmax:?} wait={wait_unmax:?} native={raw_unmax:?}"));
                let before_resize = platform::window_bounds(window);
                let (resize_x, resize_y) = before_resize.as_ref().map(|(p, _)| (p.x, p.y)).unwrap_or((190, 210));
                let after = desktop::WindowBounds { x: resize_x, y: resize_y, width: 920, height: 670 };
                let resize_position = if x11 {
                    platform::set_window_position(window, tauri::PhysicalPosition::new(after.x, after.y))
                } else {
                    Ok(())
                };
                let set = completion::set_size(window, tauri::PhysicalSize::new(after.width, after.height));
                let wait = bounded(completion::wait_for_bounds(window, after)).await;
                let actual = platform::window_bounds(window);
                let allocation = platform::on_main_thread(window, |w| {
                    let native = w.gtk_window().map_err(|e| error::AppError::Io(e.to_string()))?;
                    let body = w.default_vbox().map_err(|e| error::AppError::Io(e.to_string()))?;
                    Ok(format!("gtk-size={:?} allocation={:?} surface={:?} body={:?} titlebar={:?}", native.size(), native.allocation(), native.window().map(|s| s.geometry()), body.allocation(), native.titlebar().map(|t| t.allocation())))
                });
                println!("state-flow native-allocation {allocation:?}");
                run("restore-resize-after-max", before_resize.as_ref().is_ok() && resize_position.as_ref().is_ok() && set.as_ref().is_ok() && wait.as_ref().is_ok_and(|r| r.is_ok()) && expected(actual.clone(), after, scale, x11), format!("before={before_resize:?} position={resize_position:?} set={set:?} wait={wait:?} bounds={actual:?}"));
                let hide = window.hide();
                let hidden_deadline = Instant::now() + Duration::from_secs(2);
                let mut hidden_ack = false;
                while Instant::now() < hidden_deadline {
                    hidden_ack = platform::on_main_thread(window, |w| {
                        w.gtk_window().map(|native| !native.is_mapped() && !native.is_visible())
                            .map_err(|error| error::AppError::Io(error.to_string()))
                    }).unwrap_or(false);
                    if hidden_ack { break; }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let after_hide = platform::window_bounds(window);
                run("hide-retains-restored-bounds", hide.is_ok() && hidden_ack && expected(after_hide.clone(), after, scale, x11), format!("hidden={hidden_ack} {after_hide:?}"));

                window.show().ok();
                let enter = window.set_fullscreen(true);
                let wait_enter = bounded(completion::wait_for_fullscreen(window, true)).await;
                let raw_enter = platform::on_main_thread(window, |w| {
                    let native = w.gtk_window().map_err(|error| error::AppError::Io(error.to_string()))?;
                    Ok(native.window().is_some_and(|surface| surface.state().contains(gtk::gdk::WindowState::FULLSCREEN)))
                });
                let accepts_full = completion::accepts_bounds(window);
                run("fullscreen-enter-wait-native-state", enter.as_ref().is_ok() && wait_enter.as_ref().is_ok_and(|r| r.is_ok()) && raw_enter.as_ref().is_ok_and(|value| *value), format!("request={enter:?} wait={wait_enter:?} native={raw_enter:?}"));
                run("accept-bounds-fullscreen-false", accepts_full.as_ref().is_ok_and(|value| !*value), format!("{accepts_full:?}"));
                let exit = window.set_fullscreen(false);
                let wait_exit = bounded(completion::wait_for_fullscreen(window, false)).await;
                let raw_exit = platform::on_main_thread(window, |w| {
                    let native = w.gtk_window().map_err(|error| error::AppError::Io(error.to_string()))?;
                    Ok(native.window().is_some_and(|surface| surface.state().contains(gtk::gdk::WindowState::FULLSCREEN)))
                });
                run("fullscreen-exit-wait-native-state", exit.as_ref().is_ok() && wait_exit.as_ref().is_ok_and(|r| r.is_ok()) && raw_exit.as_ref().is_ok_and(|value| !*value), format!("request={exit:?} wait={wait_exit:?} native={raw_exit:?}"));

                }
                let window = &task_windows[1];

                // An ordinary oversized target is never requested; only its waiter is tested.
                let target = desktop::WindowBounds { x: 0, y: 0, width: 2000, height: 1500 };
                let started = Instant::now();
                let declined = bounded(completion::wait_for_bounds(window, target)).await;
                run("unrequested-large-target-bounded-error", declined.as_ref().is_ok_and(|r| r.is_err()) && started.elapsed() <= Duration::from_secs(4), format!("elapsed={:?} result={declined:?}", started.elapsed()));

                let size_started = Instant::now();
                let unrequested_size = bounded(completion::wait_for_size(window, tauri::PhysicalSize::new(2000,1500))).await;
                run("unrequested-size-bounded-error", unrequested_size.as_ref().is_ok_and(|r| r.is_err()) && size_started.elapsed() <= Duration::from_secs(4), format!("elapsed={:?} result={unrequested_size:?}", size_started.elapsed()));

                // Cancel one wait future, then prove a peer waiter can complete on the same window.
                let cancel_target = desktop::WindowBounds { x: 0, y: 0, width: 1700, height: 1200 };
                let cancelled = tokio::select! { biased; _ = tokio::time::sleep(Duration::from_millis(40)) => true, result = bounded(completion::wait_for_bounds(window, cancel_target)) => { let _ = result; false } };
                let peer_bounds = platform::window_bounds(window);
                let peer_origin = peer_bounds.as_ref().map(|(p, _)| (p.x, p.y)).unwrap_or(hidden_origin);
                let peer_target = desktop::WindowBounds { x: peer_origin.0, y: peer_origin.1, width: 920, height: 670 };
                let peer_position = if x11 {
                    platform::set_window_position(window, tauri::PhysicalPosition::new(peer_target.x, peer_target.y))
                } else {
                    Ok(())
                };
                let set_peer = completion::set_size(window, tauri::PhysicalSize::new(peer_target.width, peer_target.height));
                let peer_wait = bounded(completion::wait_for_bounds(window, peer_target)).await;
                let peer_actual = platform::window_bounds(window);
                run("cancelled-wait-does-not-block-peer", cancelled && peer_bounds.as_ref().is_ok() && peer_position.as_ref().is_ok() && set_peer.as_ref().is_ok() && peer_wait.as_ref().is_ok_and(|r| r.is_ok()) && expected(peer_actual.clone(), peer_target, scale, x11), format!("before={peer_bounds:?} position={peer_position:?} set={set_peer:?} wait={peer_wait:?} bounds={peer_actual:?}"));

                let doomed = task_windows[1].clone();
                let doomed_bounds = platform::window_bounds(&doomed).ok();
                let (doomed_x, doomed_y) = doomed_bounds.map(|(p, _)| (p.x, p.y)).unwrap_or((hidden_origin.0, hidden_origin.1));
                let destroy_wait = completion::wait_for_bounds(&doomed, desktop::WindowBounds{x:doomed_x,y:doomed_y,width:1800,height:1300});
                let (destroy_tx, destroy_rx) = tokio::sync::oneshot::channel::<()>();
                let destroyer = doomed.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let _ = destroyer.destroy();
                    let _ = destroy_tx.send(());
                });
                let destroyed_wait = bounded(destroy_wait).await;
                let _ = destroy_rx.await;
                run("destroy-pending-wait-errors", destroyed_wait.as_ref().is_ok_and(|r| r.is_err()), format!("{destroyed_wait:?}"));

                for owned in &task_windows {
                    let _ = owned.destroy();
                }
                let cleanup_deadline = Instant::now() + Duration::from_secs(2);
                let mut removed = false;
                while Instant::now() < cleanup_deadline {
                    removed = ["completion-native", "completion-frameless", "completion-never-shown"]
                        .iter()
                        .all(|label| cleanup_handle.get_webview_window(label).is_none());
                    if removed { break; }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                drop(run);
                let mut run = |name: &str, pass: bool, detail: String| {
                    case_count += 1;
                    println!("completion {name} pass={pass} {detail}");
                    if !pass { failures_task.lock().unwrap().push(name.to_owned()); }
                };
                run("all-owned-windows-removed", removed, format!("removed={removed}"));
                drop(run);
                let failed = failures_task.lock().unwrap();
                println!("window_completion_linux completed: cases={case_count} failures={} scale={} x11={}", failed.len(), scale, x11);
                for failure in failed.iter() { eprintln!("FAILED: {failure}"); }
                done.store(i32::from(!failed.is_empty()), Ordering::SeqCst);
                drop(failed);
            });

            // The spawned driver owns the checks; a GTK timer exits only after it reports.
            let handle = app.handle().clone();
            let task_result = result.clone();
            gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
                if task_result.load(Ordering::SeqCst) != 2 {
                    handle.exit(task_result.load(Ordering::SeqCst));
                    gtk::glib::ControlFlow::Break
                } else {
                    gtk::glib::ControlFlow::Continue
                }
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    // Keep the loop alive until the asynchronous cleanup assertions have reported.
    // Destroying the last fixture window otherwise requests Tauri's default exit.
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
