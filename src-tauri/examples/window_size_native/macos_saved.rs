//! Owned blank-Wry checks for saved logical outer frames, not desktop parity.
use crate::window_geometry::persistence::LogicalBounds;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::{MainThreadMarker, NSSize};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::sync_channel,
    },
    time::{Duration, Instant},
};

pub static CHECKS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn on_main<T: Send + 'static>(
    window: &tauri::WebviewWindow,
    task: impl FnOnce(&NSWindow, MainThreadMarker) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = sync_channel(1);
    let owned = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let main =
                    MainThreadMarker::new().ok_or("native probe requires AppKit's main thread")?;
                let raw = owned.ns_window().map_err(|error| error.to_string())?;
                // SAFETY: Tauri owns this live NSWindow; this callback is on AppKit's main thread.
                task(unsafe { &*raw.cast() }, main)
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(8))
        .map_err(|error| error.to_string())?
}

pub(super) fn native_bounds(window: &tauri::WebviewWindow) -> Result<LogicalBounds, String> {
    on_main(window, |native, main| {
        let screens = NSScreen::screens(main);
        let primary = screens
            .firstObject()
            .ok_or("native probe has no screen")?
            .frame();
        let frame = native.frame();
        Ok(LogicalBounds {
            x: frame.origin.x as i32,
            y: (primary.size.height - frame.origin.y - frame.size.height) as i32,
            width: frame.size.width as u32,
            height: frame.size.height as u32,
        })
    })
}

fn check(failures: &mut Vec<String>, name: impl Into<String>, result: Result<(), String>) {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = result {
        failures.push(format!("{}: {error}", name.into()));
    }
}

fn wait_mini(window: &tauri::WebviewWindow, expected: bool) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if on_main(window, |native, _| Ok(native.isMiniaturized()))? == expected {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err(format!("miniaturized state did not settle to {expected}"))
}

fn settle(window: &tauri::WebviewWindow) -> Result<(), String> {
    tauri::async_runtime::block_on(crate::platform::wait_for_window_operations(window))
        .map_err(|error| error.to_string())
}

pub fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    use crate::platform::window_state_macos::{nearest_screen, read, restore_bounds};
    let mut failures = Vec::new();
    for (active, peer) in [(first, second), (second, first)] {
        let constraints = on_main(active, |native, _| {
            let minimum = native.minSize();
            let maximum = native.maxSize();
            Ok((minimum.width, minimum.height, maximum.width, maximum.height))
        });
        println!(
            "saved-window native constraints {}: {constraints:?}",
            active.label()
        );
        let initial = match native_bounds(active) {
            Ok(value) => value,
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        for frame in ["native", "thin", "full"] {
            let result = (|| {
                crate::platform::set_window_frame(active, frame)
                    .map_err(|error| error.to_string())?;
                let before = native_bounds(active)?;
                let state = read(active).map_err(|error| error.to_string())?;
                if state.bounds != Some(before) {
                    return Err(format!(
                        "logical GetBounds differs from AppKit: {state:?} vs {before:?}"
                    ));
                }
                let other = native_bounds(peer)?;
                let requested = LogicalBounds {
                    x: 80,
                    y: 120,
                    width: 700,
                    height: 450,
                };
                let restored =
                    restore_bounds(active, requested).map_err(|error| error.to_string())?;
                if restored != requested || native_bounds(active)? != requested {
                    return Err(format!("restored logical frame differs: requested={requested:?}, returned={restored:?}"));
                }
                if active.is_visible().map_err(|error| error.to_string())? {
                    return Err("restore unexpectedly presented a hidden window".into());
                }
                if native_bounds(peer)? != other {
                    return Err("restoring one window changed its peer".into());
                }
                let invalid = LogicalBounds {
                    width: u32::MAX,
                    ..requested
                };
                if !matches!(
                    restore_bounds(active, invalid),
                    Err(crate::error::AppError::InvalidArgument(_))
                ) {
                    return Err("out-of-range logical size was not rejected".into());
                }
                if native_bounds(active)? != requested {
                    return Err("rejected restore changed the frame".into());
                }
                Ok(())
            })();
            check(
                &mut failures,
                format!("{} {frame} logical read/restore", active.label()),
                result,
            );
        }

        let limits = on_main(active, |native, _| {
            let minimum = native.minSize();
            let maximum = native.maxSize();
            native.setMinSize(NSSize::new(400.0, 300.0));
            native.setMaxSize(NSSize::new(600.0, 420.0));
            Ok((minimum.width, minimum.height, maximum.width, maximum.height))
        });
        match limits {
            Ok(old) => {
                let result = (|| {
                    let small = restore_bounds(
                        active,
                        LogicalBounds {
                            x: 80,
                            y: 120,
                            width: 1,
                            height: 1,
                        },
                    )
                    .map_err(|error| error.to_string())?;
                    if (small.width, small.height) != (400, 300) {
                        return Err(format!("logical minimum was not applied: {small:?}"));
                    }
                    let large = restore_bounds(
                        active,
                        LogicalBounds {
                            x: 80,
                            y: 120,
                            width: 800,
                            height: 700,
                        },
                    )
                    .map_err(|error| error.to_string())?;
                    if (large.width, large.height) != (600, 420) {
                        return Err(format!("logical maximum was not applied: {large:?}"));
                    }
                    Ok(())
                })();
                let reset = on_main(active, move |native, _| {
                    native.setMaxSize(NSSize::new(old.2, old.3));
                    native.setMinSize(NSSize::new(old.0, old.1));
                    Ok(())
                });
                check(
                    &mut failures,
                    format!("{} logical min/max constraints", active.label()),
                    result.and(reset),
                );
            }
            Err(error) => check(
                &mut failures,
                "capture owned native constraints",
                Err(error),
            ),
        }

        let zoom_result = (|| {
            active.maximize().map_err(|error| error.to_string())?;
            settle(active)?;
            let zoomed = read(active).map_err(|error| error.to_string())?;
            if !zoomed.maximized {
                return Err(format!("zoomed window was not maximized: {zoomed:?}"));
            }
            active.minimize().map_err(|error| error.to_string())?;
            wait_mini(active, true)?;
            let minimized = read(active).map_err(|error| error.to_string())?;
            if minimized.maximized || minimized.bounds != Some(native_bounds(active)?) {
                return Err(format!("minimized read substituted a normal frame or retained maximized: {minimized:?}"));
            }
            Ok(())
        })();
        let recovery = (|| {
            active.unminimize().map_err(|error| error.to_string())?;
            wait_mini(active, false)?;
            active.unmaximize().map_err(|error| error.to_string())?;
            settle(active)
        })();
        check(
            &mut failures,
            format!("{} current zoomed/minimized frame", active.label()),
            zoom_result.and(recovery),
        );

        let screen_result = (|| {
            let primary = on_main(active, |_, main| {
                let screens = NSScreen::screens(main);
                let frame = screens
                    .firstObject()
                    .ok_or("native probe has no primary screen")?
                    .frame();
                Ok((
                    frame.origin.x as i32,
                    frame.origin.y as i32,
                    frame.size.width as u32,
                    frame.size.height as u32,
                ))
            })?;
            let expected = LogicalBounds {
                x: primary.0,
                y: primary.1,
                width: primary.2,
                height: primary.3,
            };
            let actual = nearest_screen(
                active,
                primary.0 + primary.2 as i32 / 2,
                primary.3 as i32 / 2,
            )
            .map_err(|error| error.to_string())?;
            if actual != expected {
                return Err(format!(
                    "primary screen logical bounds differ: {actual:?} vs {expected:?}"
                ));
            }
            Ok(())
        })();
        check(
            &mut failures,
            format!("{} native primary screen", active.label()),
            screen_result,
        );

        let reset = (|| {
            crate::platform::set_window_frame(active, "full").map_err(|error| error.to_string())?;
            restore_bounds(active, initial).map_err(|error| error.to_string())?;
            if native_bounds(active)? != initial {
                return Err("could not restore owned window's initial frame".into());
            }
            Ok(())
        })();
        check(
            &mut failures,
            format!(
                "{} saved-state fixture restores original frame",
                active.label()
            ),
            reset,
        );
    }
    failures
}

pub fn check_destroyed(window: &tauri::WebviewWindow) -> Result<(), String> {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if crate::platform::window_state_macos::read(window).is_ok() {
        return Err("saved-window read accepted a destroyed handle".into());
    }
    if crate::platform::window_state_macos::restore_bounds(
        window,
        LogicalBounds {
            x: 0,
            y: 0,
            width: 700,
            height: 450,
        },
    )
    .is_ok()
    {
        return Err("saved-window restore accepted a destroyed handle".into());
    }
    Ok(())
}
