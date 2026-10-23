//! Native saved-window client bounds; durable state belongs to AppState.
use crate::{
    error::AppError,
    window_geometry::{
        linux_coordinates::{startup_bounds, StartupPosition},
        persistence::{LogicalBounds, SavedWindowState},
    },
};
use gtk::prelude::*;
use std::sync::mpsc::sync_channel;
use tauri::{LogicalPosition, LogicalSize, PhysicalPosition};

pub(super) fn on_main_thread<T, F>(
    window: &tauri::WebviewWindow,
    operation: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&tauri::WebviewWindow) -> Result<T, AppError> + Send + 'static,
{
    let (sender, receiver) = sync_channel(1);
    let owned = window.clone();
    window
        .run_on_main_thread(move || {
            let result = operation(&owned);
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

fn logical_from_x11(window: &tauri::WebviewWindow, scale: f64) -> Result<LogicalBounds, AppError> {
    let (position, size) = crate::platform::window_bounds(window)?;
    let position: LogicalPosition<i32> = position.to_logical(scale);
    let size: LogicalSize<u32> = size.to_logical(scale);
    Ok(LogicalBounds {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

fn checked_size(width: u32, height: u32) -> Result<(i32, i32), AppError> {
    let width = width.max(1);
    let height = height.max(1);
    let width = i32::try_from(width).map_err(|_| {
        AppError::InvalidArgument("saved window width exceeds the supported size limit".into())
    })?;
    let height = i32::try_from(height).map_err(|_| {
        AppError::InvalidArgument("saved window height exceeds the supported size limit".into())
    })?;
    Ok((width, height))
}

fn monitor_data(
    native: &gtk::Window,
) -> Result<(Vec<LogicalBounds>, gtk::gdk::Rectangle), AppError> {
    let display = native.display();
    let count = display.n_monitors();
    if count <= 0 {
        return Err(AppError::Io("GTK returned no monitors".into()));
    }
    let mut monitors = Vec::with_capacity(count as usize);
    let selected = native
        .window()
        .filter(|surface| !surface.is_destroyed())
        .and_then(|surface| display.monitor_at_window(&surface))
        .or_else(|| display.primary_monitor())
        .or_else(|| display.monitor(0));
    let selected =
        selected.ok_or_else(|| AppError::Io("GTK returned no default monitor".into()))?;
    for index in 0..count {
        let monitor = display
            .monitor(index)
            .ok_or_else(|| AppError::Io("GTK monitor list changed during startup".into()))?;
        let geometry = monitor.geometry();
        monitors.push(LogicalBounds {
            x: geometry.x(),
            y: geometry.y(),
            width: u32::try_from(geometry.width())
                .map_err(|_| AppError::Io("GTK returned an invalid monitor width".into()))?,
            height: u32::try_from(geometry.height())
                .map_err(|_| AppError::Io("GTK returned an invalid monitor height".into()))?,
        });
    }
    Ok((monitors, selected.workarea()))
}

/// Restore constructor options while hidden. The returned logical constructor
/// sample preserves original minimum-size timing even if GTK already clamps it.
/// initial_origin retains explicit custom construction coordinates on Wayland;
/// None selects the owning monitor's default-centered origin for unsaved windows.
pub(crate) fn restore_startup(
    window: &tauri::WebviewWindow,
    saved: SavedWindowState,
    initial_origin: Option<(i32, i32)>,
) -> Result<LogicalBounds, AppError> {
    if saved.bounds.is_none() {
        return on_main_thread(window, move |window| {
            let native = window
                .gtk_window()
                .map_err(|error| AppError::Io(error.to_string()))?;
            if native
                .display()
                .downcast_ref::<gdkx11::X11Display>()
                .is_some()
            {
                return logical_from_x11(window, native.scale_factor() as f64);
            }
            let (width, height) = native.size();
            let (_, workarea) = monitor_data(native.upcast_ref())?;
            let (x, y) = match initial_origin {
                Some(origin) => origin,
                None => centered_origin(
                    workarea,
                    u32::try_from(width.max(1)).unwrap_or(1),
                    u32::try_from(height.max(1)).unwrap_or(1),
                )?,
            };
            Ok(LogicalBounds {
                x,
                y,
                width: width.max(1) as u32,
                height: height.max(1) as u32,
            })
        });
    }

    on_main_thread(window, move |window| {
        let native = window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        let x11 = native
            .display()
            .downcast_ref::<gdkx11::X11Display>()
            .is_some();
        let (monitors, workarea) = monitor_data(native.upcast_ref())?;
        let resolved = startup_bounds(saved, &monitors)
            .map_err(AppError::InvalidArgument)?
            .ok_or_else(|| AppError::Io("saved window bounds unexpectedly missing".into()))?;
        let (width, height) = checked_size(resolved.width, resolved.height)?;
        native.resize(width, height);
        let (x, y) = match resolved.position {
            StartupPosition::Explicit { x, y } => (x, y),
            StartupPosition::DefaultCenter => {
                centered_origin(workarea, resolved.width, resolved.height)?
            }
        };
        if x11 {
            let physical: PhysicalPosition<i32> =
                LogicalPosition::new(x, y).to_physical(native.scale_factor() as f64);
            crate::platform::set_window_position(window, physical)?;
        }
        Ok(LogicalBounds {
            x,
            y,
            width: u32::try_from(width).expect("checked positive width"),
            height: u32::try_from(height).expect("checked positive height"),
        })
    })
}

fn centered_origin(
    area: gtk::gdk::Rectangle,
    width: u32,
    height: u32,
) -> Result<(i32, i32), AppError> {
    let axis = |origin: i32, area_size: i32, requested: u32| {
        let offset = (i64::from(area_size) - i64::from(requested)).max(0) / 2;
        i32::try_from(i64::from(origin) + offset).map_err(|_| {
            AppError::InvalidArgument("centered window origin exceeds the supported range".into())
        })
    };
    Ok((
        axis(area.x(), area.width(), width)?,
        axis(area.y(), area.height(), height)?,
    ))
}

fn native_maximized(native: &gtk::Window) -> Result<bool, AppError> {
    if native
        .window()
        .is_some_and(|surface| surface.is_destroyed())
    {
        return Err(AppError::Io("window surface has been destroyed".into()));
    }
    let state = native
        .window()
        .map(|surface| surface.state())
        .unwrap_or_else(gtk::gdk::WindowState::empty);
    Ok(native.is_maximized()
        && !state.intersects(gtk::gdk::WindowState::ICONIFIED | gtk::gdk::WindowState::FULLSCREEN))
}

/// Share the saved-state predicate without reading bounds or monitor geometry.
pub(crate) fn maximized(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    on_main_thread(window, |window| {
        let native = window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        native_maximized(native.upcast_ref())
    })
}

/// Read one native client snapshot. Wayland retains the last logical origin
/// because its compositor does not provide global positions.
pub(crate) fn read(
    window: &tauri::WebviewWindow,
    origin: LogicalBounds,
) -> Result<SavedWindowState, AppError> {
    on_main_thread(window, move |window| {
        let native = window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        read_native(window, &native, origin)
    })
}

fn read_native(
    window: &tauri::WebviewWindow,
    native: &gtk::ApplicationWindow,
    origin: LogicalBounds,
) -> Result<SavedWindowState, AppError> {
    let maximized = native_maximized(native.upcast_ref())?;
    if native
        .display()
        .downcast_ref::<gdkx11::X11Display>()
        .is_some()
    {
        let (position, size) = crate::platform::window_bounds(window)?;
        let scale = native.scale_factor() as f64;
        let position: LogicalPosition<i32> = position.to_logical(scale);
        let size: LogicalSize<u32> = size.to_logical(scale);
        Ok(SavedWindowState {
            bounds: Some(LogicalBounds {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            }),
            maximized,
        })
    } else {
        let (width, height) = native.size();
        Ok(SavedWindowState {
            bounds: Some(LogicalBounds {
                x: origin.x,
                y: origin.y,
                width: u32::try_from(width)
                    .map_err(|_| AppError::Io("GTK returned an invalid client width".into()))?,
                height: u32::try_from(height)
                    .map_err(|_| AppError::Io("GTK returned an invalid client height".into()))?,
            }),
            maximized,
        })
    }
}

/// Capture eligibility is sampled on the same native UI turn as the frame.
/// Mapped normal Wayland frames with a pending content allocation are omitted;
/// stable, hidden, maximized, and X11 samples retain the existing native reader.
pub(crate) fn read_for_capture(
    window: &tauri::WebviewWindow,
    origin: LogicalBounds,
) -> Result<Option<SavedWindowState>, AppError> {
    on_main_thread(window, move |window| {
        let native = window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        let sample = read_native(window, &native, origin)?;
        if !sample.maximized
            && native.is_mapped()
            && native
                .display()
                .downcast_ref::<gdkx11::X11Display>()
                .is_none()
        {
            let child = native
                .child()
                .ok_or_else(|| AppError::Io("mapped window has no content allocation".into()))?;
            let allocation = child.allocation();
            let content = sample.bounds.expect("native reader always supplies bounds");
            if i64::from(allocation.width()) != i64::from(content.width)
                || i64::from(allocation.height()) != i64::from(content.height)
            {
                return Ok(None);
            }
        }
        Ok(Some(sample))
    })
}
