//! Saved-window coordinates follow Electron 38.8.6 logical outer frames.
use std::sync::mpsc::sync_channel;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSMouseInRect, NSPoint, NSRect};

use crate::{
    error::AppError,
    window_geometry::persistence::{LogicalBounds, SavedWindowState},
};

fn on_main_thread<T, F>(window: &tauri::WebviewWindow, operation: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&NSWindow, MainThreadMarker) -> Result<T, AppError> + Send + 'static,
{
    let (sender, receiver) = sync_channel(1);
    let owned_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let marker = MainThreadMarker::new().ok_or_else(|| {
                    AppError::Io("window operation requires the main thread".into())
                })?;
                let native = owned_window
                    .ns_window()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                // SAFETY: Tauri owns this live NSWindow; this closure runs on AppKit's main thread.
                let native: &NSWindow = unsafe { &*native.cast() };
                operation(native, marker)
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

fn native_geometry_error() -> AppError {
    AppError::Io("macOS returned invalid or out-of-range window geometry".into())
}

fn trunc_i32(value: f64) -> Result<i32, AppError> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(native_geometry_error());
    }
    Ok(value.trunc() as i32)
}

fn validate_frame(frame: NSRect) -> Result<(), AppError> {
    if frame.size.width <= 0.0 || frame.size.height <= 0.0 {
        return Err(native_geometry_error());
    }
    trunc_i32(frame.origin.x)?;
    trunc_i32(frame.origin.y)?;
    trunc_i32(frame.size.width)?;
    trunc_i32(frame.size.height)?;
    trunc_i32(frame.origin.x + frame.size.width)?;
    trunc_i32(frame.origin.y + frame.size.height)?;
    Ok(())
}

fn screen_frames(marker: MainThreadMarker) -> Result<Vec<NSRect>, AppError> {
    let screens = NSScreen::screens(marker);
    let frames = (0..screens.count())
        .map(|index| screens.objectAtIndex(index).frame())
        .collect::<Vec<_>>();
    if frames.is_empty() {
        return Err(AppError::Io("macOS returned no screens".into()));
    }
    for frame in &frames {
        validate_frame(*frame)?;
    }
    Ok(frames)
}

fn bounds_from_frame(frame: NSRect, primary_height: f64) -> Result<LogicalBounds, AppError> {
    validate_frame(frame)?;
    Ok(LogicalBounds {
        x: trunc_i32(frame.origin.x)?,
        y: trunc_i32(primary_height - (frame.origin.y + frame.size.height))?,
        width: trunc_i32(frame.size.width)? as u32,
        height: trunc_i32(frame.size.height)? as u32,
    })
}

fn raw_screen_bounds(frame: NSRect) -> Result<LogicalBounds, AppError> {
    validate_frame(frame)?;
    Ok(LogicalBounds {
        x: trunc_i32(frame.origin.x)?,
        y: trunc_i32(frame.origin.y)?,
        width: trunc_i32(frame.size.width)? as u32,
        height: trunc_i32(frame.size.height)? as u32,
    })
}

fn current_bounds(window: &NSWindow, marker: MainThreadMarker) -> Result<LogicalBounds, AppError> {
    let frames = screen_frames(marker)?;
    bounds_from_frame(window.frame(), frames[0].size.height)
}

fn is_fullscreen(window: &NSWindow) -> bool {
    window.styleMask().contains(NSWindowStyleMask::FullScreen)
}

fn is_maximized(window: &NSWindow, marker: MainThreadMarker) -> Result<bool, AppError> {
    if window.isMiniaturized() || is_fullscreen(window) {
        return Ok(false);
    }
    if window.styleMask().contains(NSWindowStyleMask::Resizable) {
        return Ok(window.isZoomed());
    }

    let main_screen = NSScreen::mainScreen(marker)
        .ok_or_else(|| AppError::Io("macOS returned no main screen".into()))?;
    let frame = window.frame();
    let visible = main_screen.visibleFrame();
    validate_frame(frame)?;
    validate_frame(visible)?;
    Ok(frame == visible)
}

/// Reuse the fixed native maximization rule without reading the saved frame.
pub(crate) fn maximized(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    on_main_thread(window, is_maximized)
}

pub(crate) fn read(window: &tauri::WebviewWindow) -> Result<SavedWindowState, AppError> {
    on_main_thread(window, |native, marker| {
        Ok(SavedWindowState {
            bounds: Some(current_bounds(native, marker)?),
            maximized: is_maximized(native, marker)?,
        })
    })
}

pub(crate) fn restore_bounds(
    window: &tauri::WebviewWindow,
    bounds: LogicalBounds,
) -> Result<LogicalBounds, AppError> {
    on_main_thread(window, move |native, marker| {
        let frames = screen_frames(marker)?;
        if is_fullscreen(native) {
            return bounds_from_frame(native.frame(), frames[0].size.height);
        }
        if bounds.width > i32::MAX as u32 || bounds.height > i32::MAX as u32 {
            return Err(AppError::InvalidArgument(
                "saved window dimensions exceed the supported size limit".into(),
            ));
        }

        let minimum = native.minSize();
        let maximum = native.maxSize();
        if !minimum.width.is_finite()
            || !minimum.height.is_finite()
            || !maximum.width.is_finite()
            || !maximum.height.is_finite()
            || minimum.width < 0.0
            || minimum.height < 0.0
        {
            return Err(native_geometry_error());
        }
        trunc_i32(minimum.width)?;
        trunc_i32(minimum.height)?;

        let mut width = (bounds.width as f64).max(minimum.width);
        let mut height = (bounds.height as f64).max(minimum.height);
        if maximum.width > 0.0 && maximum.height > 0.0 {
            width = width.min(maximum.width);
            height = height.min(maximum.height);
        }
        let origin_y = frames[0].size.height - height - bounds.y as f64;
        trunc_i32(origin_y)?;
        let frame = NSRect::new(
            NSPoint::new(bounds.x as f64, origin_y),
            objc2_foundation::NSSize::new(width, height),
        );
        validate_frame(frame)?;
        native.setFrame_display(frame, false);
        current_bounds(native, marker)
    })
}

pub(crate) fn nearest_screen(
    window: &tauri::WebviewWindow,
    x: i32,
    y: i32,
) -> Result<LogicalBounds, AppError> {
    let frames = on_main_thread(window, |_, marker| screen_frames(marker))?;
    nearest_screen_in_frames(&frames, x, y)
}

/// Restore the fixed constructor's saved bounds before first presentation.
/// Saved maximization controls relocation here; presentation applies it later.
pub(crate) fn restore_startup(
    window: &tauri::WebviewWindow,
    saved: SavedWindowState,
) -> Result<Option<LogicalBounds>, AppError> {
    let Some(mut bounds) = saved.bounds else {
        return Ok(None);
    };
    let frames = on_main_thread(window, |_, marker| screen_frames(marker))?;
    let screen = nearest_screen_in_frames(&frames, bounds.x, bounds.y)?;
    let disjoint = screen.x as i64 > bounds.x as i64 + bounds.width as i64
        || (screen.x as i64 + screen.width as i64) < bounds.x as i64
        || screen.y as i64 > bounds.y as i64 + bounds.height as i64
        || (screen.y as i64 + screen.height as i64) < bounds.y as i64;
    if disjoint && !saved.maximized {
        // The fixed constructor omits the selected screen's origin.
        let x = (screen.width as f64 - bounds.width as f64) / 2.0;
        let y = (screen.height as f64 - bounds.height as f64) / 2.0;
        if x.fract() == 0.0 && y.fract() == 0.0 {
            bounds.x = trunc_i32(x)?;
            bounds.y = trunc_i32(y)?;
        } else {
            // Electron rejects both position options if either is fractional.
            bounds.x = trunc_i32(((frames[0].size.width - bounds.width as f64) / 2.0).round())?;
            bounds.y = trunc_i32(((frames[0].size.height - bounds.height as f64) / 2.0).round())?;
        }
    }
    restore_bounds(window, bounds).map(Some)
}

pub(super) fn nearest_screen_in_frames(
    screens: &[NSRect],
    x: i32,
    y: i32,
) -> Result<LogicalBounds, AppError> {
    let Some(primary) = screens.first().copied() else {
        return Err(AppError::Io("macOS returned no screens".into()));
    };
    for frame in screens {
        validate_frame(*frame)?;
    }
    if screens.len() == 1 {
        return raw_screen_bounds(primary);
    }

    let point = NSPoint::new(x as f64, primary.origin.y + primary.size.height - y as f64);
    let selected = screens
        .iter()
        .position(|frame| NSMouseInRect(point, *frame, false))
        .or_else(|| {
            let mut nearest = 0usize;
            let mut minimum_distance = f64::INFINITY;
            for (index, frame) in screens.iter().enumerate() {
                let min_x = frame.origin.x;
                let min_y = frame.origin.y;
                let max_x = min_x + frame.size.width;
                let max_y = min_y + frame.size.height;
                let distance = [
                    (min_x, min_y),
                    (max_x, min_y),
                    (min_x, max_y),
                    (max_x, max_y),
                ]
                .into_iter()
                .map(|(cx, cy)| (point.x - cx).abs() + (point.y - cy).abs())
                .fold(f64::INFINITY, f64::min);
                if distance < minimum_distance {
                    minimum_distance = distance;
                    nearest = index;
                }
            }
            Some(nearest)
        })
        .ok_or_else(native_geometry_error)?;

    if selected == 0 {
        raw_screen_bounds(screens[selected])
    } else {
        let frame = screens[selected];
        Ok(LogicalBounds {
            x: trunc_i32(frame.origin.x)?,
            y: trunc_i32(
                primary.origin.y + primary.size.height - frame.origin.y - frame.size.height,
            )?,
            width: trunc_i32(frame.size.width)? as u32,
            height: trunc_i32(frame.size.height)? as u32,
        })
    }
}

#[cfg(test)]
#[path = "window_state_macos_tests.rs"]
mod tests;
