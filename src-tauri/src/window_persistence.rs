//! Saves and restores window placement like upstream `app/lib/window.ts`:
//! the latest unmaximized bounds are remembered on move/resize, written to
//! `window.json` when a window closes with renderer approval, and applied to
//! the next window before it becomes visible.
//!
//! Units are the platform's native placement space: physical pixels on
//! Windows and Linux (one global coordinate space across monitors of mixed
//! DPI) and points on macOS (where Tao scales each monitor separately).

use tauri::{Manager, PhysicalPosition, PhysicalSize};

use crate::{
    error::AppError,
    state::AppState,
    storage::{
        paths::StoragePaths,
        window_state::{
            load_window_state, restored_placement, save_window_state, MonitorRect,
            WindowBoundaries, WindowStateFile,
        },
    },
};

fn io_error(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

fn window_state_path(window: &tauri::WebviewWindow) -> std::path::PathBuf {
    let state = window.state::<AppState>();
    StoragePaths::from_app_paths(state.paths())
        .window_state_file()
        .to_path_buf()
}

/// Divides native pixels by `scale` only where the consistent cross-monitor
/// space is logical (macOS).
fn to_placement_units(value: f64, scale: f64) -> f64 {
    if cfg!(target_os = "macos") {
        value / scale
    } else {
        value
    }
}

fn from_placement_units(value: f64, scale: f64) -> f64 {
    if cfg!(target_os = "macos") {
        value * scale
    } else {
        value
    }
}

/// Current placement using the platform bounds API.
fn current_bounds(window: &tauri::WebviewWindow) -> Result<WindowBoundaries, AppError> {
    #[cfg(target_os = "linux")]
    let (position, size) = crate::platform::window_bounds(window)?;
    #[cfg(not(target_os = "linux"))]
    let (position, size) = (
        window.outer_position().map_err(io_error)?,
        window.outer_size().map_err(io_error)?,
    );
    let scale = window.scale_factor().map_err(io_error)?;
    Ok(WindowBoundaries {
        x: to_placement_units(f64::from(position.x), scale),
        y: to_placement_units(f64::from(position.y), scale),
        width: to_placement_units(f64::from(size.width), scale),
        height: to_placement_units(f64::from(size.height), scale),
    })
}

/// Whether the window currently shows its normal (unmaximized, unminimized,
/// windowed) placement, read from the native window where Tao's cache lags.
fn has_normal_placement(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    if window.is_fullscreen().map_err(io_error)? {
        return Ok(false);
    }
    let flags = match crate::platform::live_window_flags(window)? {
        Some(flags) => flags,
        None => crate::platform::WindowFlags {
            maximized: window.is_maximized().map_err(io_error)?,
            minimized: window.is_minimized().map_err(io_error)?,
        },
    };
    Ok(!flags.maximized && !flags.minimized)
}

/// Remembers the placement after a move or resize, skipping maximized,
/// fullscreen, and minimized states (upstream skips maximized; fullscreen and
/// minimized windows report placeholder geometry that must not be restored).
pub(crate) fn remember(window: &tauri::WebviewWindow) {
    if !matches!(has_normal_placement(window), Ok(true)) {
        return;
    }
    if let Ok(bounds) = current_bounds(window) {
        window
            .state::<AppState>()
            .remember_window_bounds(window.label(), bounds);
    }
}

/// Writes `window.json` for a window that is about to be destroyed after the
/// renderer approved the close.
pub(crate) fn save_on_close(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    let state = window.state::<AppState>();
    let maximized = match crate::platform::live_window_flags(window)? {
        Some(flags) => flags.maximized,
        None => window.is_maximized().map_err(io_error)?,
    };
    let remembered = state.remembered_window_bounds(window.label()).or_else(|| {
        matches!(has_normal_placement(window), Ok(true))
            .then(|| current_bounds(window).ok())
            .flatten()
    });
    let path = window_state_path(window);
    let mut file = load_window_state(&path).unwrap_or_default();
    if let Some(bounds) = remembered {
        file.window_boundaries = Some(bounds);
    }
    file.maximized = maximized;
    save_window_state(&path, &file)
}

/// Applies the saved placement to a still-hidden window: position first when
/// the platform supports absolute positioning (so a later size applies on the
/// target monitor's DPI), then size. A saved maximized state is deferred until the window becomes visible
/// (`window.applyState { visible: true }`), matching upstream's maximize on
/// `did-finish-load`; maximizing a hidden Win32 window would show it early.
pub(crate) fn restore(
    window: &tauri::WebviewWindow,
    set_size: impl Fn(&tauri::WebviewWindow, PhysicalSize<u32>) -> Result<(), AppError>,
    set_position: impl Fn(&tauri::WebviewWindow, PhysicalPosition<i32>) -> Result<(), AppError>,
    absolute_positioning: bool,
) -> Result<(), AppError> {
    let path = window_state_path(window);
    let saved = match load_window_state(&path) {
        Ok(saved) => saved,
        Err(error) => {
            eprintln!("ignoring unreadable window.json: {error}");
            WindowStateFile::default()
        }
    };
    let monitors: Vec<MonitorRect> = window
        .available_monitors()
        .map_err(io_error)?
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            MonitorRect {
                x: to_placement_units(f64::from(monitor.position().x), scale),
                y: to_placement_units(f64::from(monitor.position().y), scale),
                width: to_placement_units(f64::from(monitor.size().width), scale),
                height: to_placement_units(f64::from(monitor.size().height), scale),
            }
        })
        .collect();
    let Some(placement) = restored_placement(&saved, &monitors) else {
        return Ok(());
    };
    let state = window.state::<AppState>();
    if let Some(bounds) = placement.bounds {
        let scale = window.scale_factor().map_err(io_error)?;
        if absolute_positioning {
            set_position(
                window,
                PhysicalPosition::new(
                    from_placement_units(bounds.x, scale).round() as i32,
                    from_placement_units(bounds.y, scale).round() as i32,
                ),
            )?;
        }
        let scale = window.scale_factor().map_err(io_error)?;
        set_size(
            window,
            PhysicalSize::new(
                from_placement_units(bounds.width, scale).round().max(1.0) as u32,
                from_placement_units(bounds.height, scale).round().max(1.0) as u32,
            ),
        )?;
        state.remember_window_bounds(window.label(), bounds);
    }
    if placement.maximized {
        state.set_pending_maximize(window.label());
    }
    Ok(())
}

/// Maximizes a window whose saved state asked for it, once it is shown.
pub(crate) fn apply_pending_maximize(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    if window
        .state::<AppState>()
        .take_pending_maximize(window.label())
    {
        window.maximize().map_err(io_error)?;
    }
    Ok(())
}
