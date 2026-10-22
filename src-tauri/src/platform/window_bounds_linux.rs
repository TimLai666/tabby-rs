use crate::error::AppError;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use std::sync::mpsc::sync_channel;
use tauri::{PhysicalPosition, PhysicalSize};

pub(crate) fn window_bounds(
    window: &tauri::WebviewWindow,
) -> Result<(PhysicalPosition<i32>, PhysicalSize<u32>), AppError> {
    let (sender, receiver) = sync_channel(1);
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let native = main_window
                    .gtk_window()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                let scale = native.scale_factor() as f64;
                let display_handle = native.display();
                let Some(display) = display_handle.downcast_ref::<gdkx11::X11Display>() else {
                    let position = main_window
                        .outer_position()
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    let (width, height) = native.size();
                    let size = tauri::LogicalSize::new(width, height).to_physical(scale);
                    return Ok((position, size));
                };

                let position = if let Some(surface) = native.window() {
                    if surface.is_destroyed() {
                        return Err(AppError::Io("window surface has been destroyed".into()));
                    }
                    let x11_window = surface
                        .downcast::<gdkx11::X11Window>()
                        .map_err(|_| AppError::Io("X11 display has a non-X11 window surface".into()))?;
                    let xdisplay = unsafe {
                        gdkx11::ffi::gdk_x11_display_get_xdisplay(
                            display.to_glib_none().0,
                        )
                    };
                    if xdisplay.is_null() {
                        return Err(AppError::Io("X11 display is unavailable".into()));
                    }
                    let (mut root, mut child) = (0, 0);
                    let (mut x, mut y, mut root_x, mut root_y) = (0, 0, 0, 0);
                    let (mut width, mut height, mut border, mut depth) = (0, 0, 0, 0);
                    display.error_trap_push();
                    let geometry = unsafe {
                        gdkx11::x11::xlib::XGetGeometry(
                            xdisplay, x11_window.xid(), &mut root, &mut x, &mut y,
                            &mut width, &mut height, &mut border, &mut depth,
                        )
                    };
                    let translated = if geometry != 0 {
                        unsafe {
                            gdkx11::x11::xlib::XTranslateCoordinates(
                                xdisplay, x11_window.xid(), root, 0, 0,
                                &mut root_x, &mut root_y, &mut child,
                            )
                        }
                    } else {
                        0
                    };
                    let x_error = display.error_trap_pop();
                    if geometry == 0 || translated == 0 || x_error != 0 {
                        return Err(AppError::Io(format!(
                            "could not read X11 window bounds (geometry={geometry}, translate={translated}, error={x_error})"
                        )));
                    }

                    let size = if native.is_mapped() {
                        PhysicalSize::new(width, height)
                    } else {
                        let (width, height) = native.size();
                        tauri::LogicalSize::new(width, height).to_physical(scale)
                    };
                    normalize_bounds(PhysicalPosition::new(root_x, root_y), size, scale)
                } else {
                    let (x, y) = native.position();
                    let (width, height) = native.size();
                    (
                        tauri::LogicalPosition::new(x, y).to_physical(scale),
                        tauri::LogicalSize::new(width, height).to_physical(scale),
                    )
                };
                Ok(position)
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

pub(crate) fn set_window_position(
    window: &tauri::WebviewWindow,
    position: PhysicalPosition<i32>,
) -> Result<(), AppError> {
    let (sender, receiver) = sync_channel(1);
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let native = main_window
                    .gtk_window()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                if native
                    .display()
                    .downcast_ref::<gdkx11::X11Display>()
                    .is_some()
                {
                    let logical: tauri::LogicalPosition<f64> =
                        position.to_logical(native.scale_factor() as f64);
                    // Custom windows skip stock initialization but share client coordinates.
                    native.set_gravity(gtk::gdk::Gravity::Static);
                    native.move_(logical.x.round() as i32, logical.y.round() as i32);
                    if !native.is_mapped() {
                        if let Some(surface) = native.window() {
                            if surface.is_destroyed() {
                                return Err(AppError::Io(
                                    "window surface has been destroyed".into(),
                                ));
                            }
                            surface.move_(logical.x.round() as i32, logical.y.round() as i32);
                        }
                    }
                    Ok(())
                } else {
                    main_window
                        .set_position(position)
                        .map_err(|error| AppError::Io(error.to_string()))
                }
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

pub(crate) fn supports_absolute_positioning(
    window: &tauri::WebviewWindow,
) -> Result<bool, AppError> {
    let (sender, receiver) = sync_channel(1);
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = main_window
                .gtk_window()
                .map(|native| {
                    native
                        .display()
                        .downcast_ref::<gdkx11::X11Display>()
                        .is_some()
                })
                .map_err(|error| AppError::Io(error.to_string()));
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

fn normalize_bounds(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    scale: f64,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let left = (position.x as f64 / scale).floor();
    let top = (position.y as f64 / scale).floor();
    let right = ((position.x as f64 + size.width as f64) / scale).ceil();
    let bottom = ((position.y as f64 + size.height as f64) / scale).ceil();
    (
        tauri::LogicalPosition::new(left, top).to_physical(scale),
        tauri::LogicalSize::new(right - left, bottom - top).to_physical(scale),
    )
}
