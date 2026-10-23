//! Real AppKit completion and Tao zoom-bookkeeping checks. No renderer/IPC acceptance.
use std::sync::atomic::{AtomicUsize, Ordering};

pub static CHECKS: AtomicUsize = AtomicUsize::new(0);

fn completed<T>(
    operation: impl std::future::Future<Output = Result<T, crate::error::AppError>>,
) -> Result<T, String> {
    tauri::async_runtime::block_on(operation).map_err(|error| error.to_string())
}

fn size(window: &tauri::WebviewWindow) -> Result<tauri::PhysicalSize<u32>, String> {
    window.outer_size().map_err(|error| error.to_string())
}

fn zoom(window: &tauri::WebviewWindow, value: bool) -> Result<(), String> {
    if value {
        window.maximize()
    } else {
        window.unmaximize()
    }
    .map_err(|error| error.to_string())?;
    completed(crate::platform::wait_for_window_operations(window))?;
    if window.is_maximized().map_err(|error| error.to_string())? != value {
        return Err(format!(
            "{} zoom did not complete to {value}",
            window.label()
        ));
    }
    Ok(())
}

fn check(failures: &mut Vec<String>, name: String, result: Result<(), String>) {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = result {
        failures.push(format!("{name}: {error}"));
    }
}

pub fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    let mut failures = Vec::new();
    for (active, peer) in [(first, second), (second, first)] {
        let initial = match size(active) {
            Ok(value) => value,
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        for frame in ["native", "thin", "full"] {
            let result = (|| {
                zoom(active, false)?;
                crate::platform::set_window_frame(active, frame)
                    .map_err(|error| error.to_string())?;
                let peer_size = size(peer)?;
                let position = active.outer_position().map_err(|error| error.to_string())?;
                let scale = active.scale_factor().map_err(|error| error.to_string())?;
                let requested =
                    tauri::PhysicalSize::new((700.0 * scale) as u32, (450.0 * scale) as u32);
                completed(crate::platform::set_window_outer_size_async(
                    active, requested,
                ))?;
                // No polling, sleeps, or extra FIFO barrier may make this read pass.
                if size(active)? != requested {
                    return Err(format!(
                        "resize acknowledged before requested frame: {:?}",
                        size(active)?
                    ));
                }
                if active.outer_position().map_err(|error| error.to_string())? != position {
                    return Err("completed resize moved the top-left corner".into());
                }
                if size(peer)? != peer_size {
                    return Err("completed resize changed the other window".into());
                }
                zoom(active, true)?;
                zoom(active, false)?;
                if size(active)? != requested {
                    return Err("Tao unmaximize did not retain the completed normal frame".into());
                }
                let before_error = size(active)?;
                let invalid = tauri::PhysicalSize::new(u32::MAX, requested.height);
                if !matches!(
                    tauri::async_runtime::block_on(crate::platform::set_window_outer_size_async(
                        active, invalid
                    )),
                    Err(crate::error::AppError::InvalidArgument(_))
                ) {
                    return Err("overflow was not returned as InvalidArgument".into());
                }
                if size(active)? != before_error || size(peer)? != peer_size {
                    return Err("rejected async resize changed a window".into());
                }
                Ok(())
            })();
            check(
                &mut failures,
                format!(
                    "{} {frame} awaited resize and public Tao zoom",
                    active.label()
                ),
                result,
            );
        }
        let reset = (|| {
            zoom(active, false)?;
            crate::platform::set_window_frame(active, "full").map_err(|error| error.to_string())?;
            completed(crate::platform::set_window_outer_size_async(
                active, initial,
            ))?;
            if size(active)? != initial {
                return Err("could not restore initial dimensions".into());
            }
            Ok(())
        })();
        check(
            &mut failures,
            format!(
                "{} completion fixture restores initial dimensions",
                active.label()
            ),
            reset,
        );
    }
    failures
}

pub fn check_destroyed(window: &tauri::WebviewWindow) -> Result<(), String> {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    match tauri::async_runtime::block_on(crate::platform::set_window_outer_size_async(
        window,
        tauri::PhysicalSize::new(900, 650),
    )) {
        Err(_) => Ok(()),
        Ok(()) => Err("async resize of a destroyed window incorrectly succeeded".into()),
    }
}
