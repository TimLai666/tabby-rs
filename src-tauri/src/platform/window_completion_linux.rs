//! Linux request completion; native regression checks define its contract.
use crate::{desktop::WindowBounds, error::AppError};
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Duration};
use tauri::{LogicalPosition, LogicalSize, PhysicalPosition};

const MIN_WIDTH: u32 = 400;
const MIN_HEIGHT: u32 = 300;
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

/// Match the fixed original setter minimum without changing constructor options.
pub(crate) fn set_size(
    window: &tauri::WebviewWindow,
    size: tauri::PhysicalSize<u32>,
) -> Result<(), AppError> {
    let scale = window
        .scale_factor()
        .map_err(|error| AppError::Io(error.to_string()))?;
    let logical_size = normalized_size(size, scale, "window")?;

    // Keep the framework request queued through Tao. Calling gtk::Window::resize
    // here would overtake requests already waiting in Tao's event queue.
    window
        .set_size(logical_size)
        .map_err(|error| AppError::Io(error.to_string()))
}

fn normalized_size(
    size: tauri::PhysicalSize<u32>,
    scale: f64,
    description: &str,
) -> Result<LogicalSize<u32>, AppError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(AppError::Io("GTK returned an invalid scale factor".into()));
    }
    let dimension = |physical: u32, minimum: u32, name: &str| {
        let logical = (f64::from(physical) / scale)
            .round()
            .max(f64::from(minimum));
        if logical > f64::from(i32::MAX) {
            return Err(AppError::InvalidArgument(format!(
                "{description} {name} exceeds the supported logical size limit"
            )));
        }
        Ok(logical as u32)
    };
    Ok(LogicalSize::new(
        dimension(size.width, MIN_WIDTH, "width")?,
        dimension(size.height, MIN_HEIGHT, "height")?,
    ))
}

fn x11_client_size(
    display: &gdkx11::X11Display,
    surface: &gtk::gdk::Window,
) -> Result<tauri::PhysicalSize<u32>, AppError> {
    if surface.is_destroyed() {
        return Err(AppError::Io("window surface has been destroyed".into()));
    }
    let x11_window = surface
        .downcast_ref::<gdkx11::X11Window>()
        .ok_or_else(|| AppError::Io("X11 display has a non-X11 window surface".into()))?;
    let xdisplay = unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(display.to_glib_none().0) };
    if xdisplay.is_null() {
        return Err(AppError::Io("X11 display is unavailable".into()));
    }

    let (mut root, mut x, mut y) = (0, 0, 0);
    let (mut width, mut height, mut border, mut depth) = (0, 0, 0, 0);
    display.error_trap_push();
    let geometry = unsafe {
        gdkx11::x11::xlib::XGetGeometry(
            xdisplay,
            x11_window.xid(),
            &mut root,
            &mut x,
            &mut y,
            &mut width,
            &mut height,
            &mut border,
            &mut depth,
        )
    };
    let x_error = display.error_trap_pop();
    if geometry == 0 || x_error != 0 {
        return Err(AppError::Io(format!(
            "could not read X11 window size (geometry={geometry}, error={x_error})"
        )));
    }
    Ok(tauri::PhysicalSize::new(width, height))
}

/// Native maximized/fullscreen windows ignore ordinary bounds-only requests.
pub(crate) fn accepts_bounds(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    crate::platform::window_state_linux::on_main_thread(window, |window| {
        let native = window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        let surface = native.window();
        if surface
            .as_ref()
            .is_some_and(|surface| surface.is_destroyed())
        {
            return Err(AppError::Io("window surface has been destroyed".into()));
        }
        let fullscreen = surface
            .as_ref()
            .is_some_and(|surface| surface.state().contains(gtk::gdk::WindowState::FULLSCREEN));
        Ok(!native.is_maximized() && !fullscreen)
    })
}

#[derive(Clone, Copy)]
enum Completion {
    Maximized(bool),
    Fullscreen(bool),
    Size(tauri::PhysicalSize<u32>),
    Bounds(WindowBounds),
}

struct Snapshot {
    mapped: bool,
    scale: f64,
    x11: bool,
    size: tauri::PhysicalSize<u32>,
    complete: bool,
}

/// Queue a GLib idle after the Tao request queue, without blocking GTK while it runs.
async fn after_tao_queue(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            gtk::glib::idle_add_local_once(move || {
                let _ = sender.send(());
            });
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    tokio::time::timeout(COMPLETION_TIMEOUT, receiver)
        .await
        .map_err(|_| AppError::Io("timed out draining the Linux window request queue".into()))?
        .map_err(|error| AppError::Io(error.to_string()))
}

// Disposing the owning native window can disconnect the signal while this
// waiter still holds the frame clock for cancellation cleanup.
fn disconnect_paint(clock: &gtk::gdk::FrameClock, id: gtk::glib::SignalHandlerId) {
    // Both the live object reference and its original handler ID are owned by
    // this UI-thread waiter. GObject may already have removed that handler.
    let connected = unsafe {
        gtk::glib::gobject_ffi::g_signal_handler_is_connected(
            clock.as_ptr() as *mut gtk::glib::gobject_ffi::GObject,
            id.as_raw(),
        ) != 0
    };
    if connected {
        clock.disconnect(id);
    }
}

/// Install one after-paint hook. A GLib timer disconnects the hook when its
/// receiver is dropped, so cancellation leaves neither a window nor a callback
/// attached to the waiter.
async fn after_paint(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    type PaintReceiver = tokio::sync::oneshot::Receiver<()>;
    let (setup_sender, setup_receiver) =
        tokio::sync::oneshot::channel::<Result<Option<PaintReceiver>, AppError>>();
    let owned = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let native = owned
                    .gtk_window()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                if !native.is_mapped() {
                    return Ok(None);
                }
                let clock = native
                    .frame_clock()
                    .ok_or_else(|| AppError::Io("mapped GTK window has no frame clock".into()))?;
                let (paint_sender, paint_receiver) = tokio::sync::oneshot::channel();
                let sender = Rc::new(RefCell::new(Some(paint_sender)));
                let handler = Rc::new(RefCell::new(None));

                let signal_sender = sender.clone();
                let signal_handler = handler.clone();
                let id = clock.connect_after_paint(move |clock| {
                    if let Some(id) = signal_handler.borrow_mut().take() {
                        disconnect_paint(clock, id);
                    }
                    if let Some(sender) = signal_sender.borrow_mut().take() {
                        let _ = sender.send(());
                    }
                });
                *handler.borrow_mut() = Some(id);

                let timer_clock = clock.clone();
                let timer_sender = sender.clone();
                let timer_handler = handler.clone();
                gtk::glib::timeout_add_local(Duration::from_millis(10), move || {
                    let cancelled = timer_sender
                        .borrow()
                        .as_ref()
                        .map_or(true, tokio::sync::oneshot::Sender::is_closed);
                    if cancelled {
                        if let Some(id) = timer_handler.borrow_mut().take() {
                            disconnect_paint(&timer_clock, id);
                        }
                        timer_sender.borrow_mut().take();
                        gtk::glib::ControlFlow::Break
                    } else {
                        gtk::glib::ControlFlow::Continue
                    }
                });
                native.queue_draw();
                Ok(Some(paint_receiver))
            })();
            let _ = setup_sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;

    let receiver = tokio::time::timeout(COMPLETION_TIMEOUT, setup_receiver)
        .await
        .map_err(|_| AppError::Io("timed out preparing a GTK paint wait".into()))?
        .map_err(|error| AppError::Io(error.to_string()))??;
    let Some(receiver) = receiver else {
        return Ok(false);
    };
    tokio::time::timeout(COMPLETION_TIMEOUT, receiver)
        .await
        .map_err(|_| AppError::Io("timed out waiting for GTK after-paint".into()))?
        .map_err(|error| AppError::Io(error.to_string()))?;
    Ok(true)
}

fn snapshot(window: &tauri::WebviewWindow, completion: Completion) -> Result<Snapshot, AppError> {
    crate::platform::window_state_linux::on_main_thread(window, move |window| {
        let native = window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        if native
            .window()
            .as_ref()
            .is_some_and(|surface| surface.is_destroyed())
        {
            return Err(AppError::Io("window surface has been destroyed".into()));
        }
        let surface_state = native
            .window()
            .map(|surface| surface.state())
            .unwrap_or_else(gtk::gdk::WindowState::empty);
        let complete = match completion {
            Completion::Maximized(value) => native.is_maximized() == value,
            Completion::Fullscreen(value) => {
                surface_state.contains(gtk::gdk::WindowState::FULLSCREEN) == value
            }
            Completion::Size(_) => false,
            Completion::Bounds(_) => false,
        };
        let x11 = native
            .display()
            .downcast_ref::<gdkx11::X11Display>()
            .is_some();
        let scale = native.scale_factor() as f64;
        let size = if matches!(completion, Completion::Size(_)) {
            if !scale.is_finite() || scale <= 0.0 {
                return Err(AppError::Io("GTK returned an invalid scale factor".into()));
            }
            if native.is_mapped() && x11 {
                let surface = native
                    .window()
                    .ok_or_else(|| AppError::Io("mapped GTK window has no surface".into()))?;
                let display = native
                    .display()
                    .downcast::<gdkx11::X11Display>()
                    .map_err(|_| AppError::Io("GTK window is not on an X11 display".into()))?;
                x11_client_size(&display, &surface)?
            } else {
                let (width, height) = native.size();
                LogicalSize::new(width.max(0) as u32, height.max(0) as u32)
                    .to_physical::<u32>(scale)
            }
        } else {
            tauri::PhysicalSize::new(0, 0)
        };
        Ok(Snapshot {
            mapped: native.is_mapped(),
            scale,
            x11,
            size,
            complete,
        })
    })
}

fn bounds_match(
    window: &tauri::WebviewWindow,
    target: WindowBounds,
    scale: f64,
    x11: bool,
) -> Result<bool, AppError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(AppError::Io("GTK returned an invalid scale factor".into()));
    }
    let logical_width = (f64::from(target.width) / scale)
        .round()
        .max(f64::from(MIN_WIDTH));
    let logical_height = (f64::from(target.height) / scale)
        .round()
        .max(f64::from(MIN_HEIGHT));
    if logical_width > f64::from(i32::MAX) || logical_height > f64::from(i32::MAX) {
        return Err(AppError::InvalidArgument(
            "requested window bounds exceed the supported size limit".into(),
        ));
    }
    let expected =
        LogicalSize::new(logical_width as u32, logical_height as u32).to_physical::<u32>(scale);
    let (position, size) = crate::platform::window_bounds(window)?;
    if size != expected {
        return Ok(false);
    }

    if x11 {
        let logical = PhysicalPosition::new(target.x, target.y).to_logical::<f64>(scale);
        let expected_position =
            LogicalPosition::new(logical.x.round() as i32, logical.y.round() as i32)
                .to_physical(scale);
        Ok(position == expected_position)
    } else {
        Ok(true)
    }
}

async fn wait_for(window: &tauri::WebviewWindow, completion: Completion) -> Result<(), AppError> {
    let deadline = tokio::time::Instant::now() + COMPLETION_TIMEOUT;
    after_tao_queue(window).await?;

    loop {
        let current = snapshot(window, completion)?;
        if !current.mapped
            && matches!(
                completion,
                Completion::Maximized(_) | Completion::Fullscreen(_)
            )
        {
            // State requests on never-shown windows retain initial metadata
            // behavior; they must not force a native window to be realized.
            return Ok(());
        }
        let complete = match completion {
            Completion::Size(target) => {
                current.size
                    == normalized_size(target, current.scale, "window")?
                        .to_physical::<u32>(current.scale)
            }
            Completion::Bounds(target) => bounds_match(window, target, current.scale, current.x11)?,
            _ => current.complete,
        };
        if complete {
            if !current.mapped {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            if tokio::time::timeout(remaining, after_paint(window))
                .await
                .map_err(|_| AppError::Io("timed out waiting for GTK after-paint".into()))??
            {
                // Re-read after a real paint before confirming the predicate.
                let confirmed = snapshot(window, completion)?;
                let confirmed_complete = match completion {
                    Completion::Size(target) => {
                        confirmed.size
                            == normalized_size(target, confirmed.scale, "window")?
                                .to_physical::<u32>(confirmed.scale)
                    }
                    Completion::Bounds(target) => {
                        bounds_match(window, target, confirmed.scale, confirmed.x11)?
                    }
                    _ => confirmed.complete,
                };
                if confirmed_complete {
                    return Ok(());
                }
            }
        } else if !current.mapped {
            // Hidden windows do not paint. Keep polling their GTK metadata until
            // Tao's size request is visible or the same bounded deadline expires.
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        } else {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            if tokio::time::timeout(remaining, after_paint(window))
                .await
                .map_err(|_| AppError::Io("timed out waiting for GTK after-paint".into()))??
            {
                continue;
            }
        }
    }

    Err(AppError::Io(
        "timed out waiting for the Linux window operation".into(),
    ))
}

/// Complete after the requested mapped native maximization is acknowledged.
/// Never-shown/hidden windows retain their native initial-state behavior.
pub(crate) async fn wait_for_maximized(
    window: &tauri::WebviewWindow,
    value: bool,
) -> Result<(), AppError> {
    wait_for(window, Completion::Maximized(value)).await
}

/// Complete after the requested mapped native fullscreen state is acknowledged.
pub(crate) async fn wait_for_fullscreen(
    window: &tauri::WebviewWindow,
    value: bool,
) -> Result<(), AppError> {
    wait_for(window, Completion::Fullscreen(value)).await
}

/// Acknowledge effective client size before issuing an absolute position request.
/// Preserve original minimums and GTK rounding; ignore the current origin.
/// Hidden metadata, cancellation, native errors and the two-second deadline use
/// the same completion contract as bounds. This operation only observes size.
pub(crate) async fn wait_for_size(
    window: &tauri::WebviewWindow,
    size: tauri::PhysicalSize<u32>,
) -> Result<(), AppError> {
    wait_for(window, Completion::Size(size)).await
}

/// Wait for effective physical client bounds, including the original setter minimum
/// and integer GTK scale rounding. Wayland origins are metadata, not global moves.
/// A declined request or destroyed window must error within a bounded interval;
/// cancellation ends polling without retaining a window. Each waiter is independent.
pub(crate) async fn wait_for_bounds(
    window: &tauri::WebviewWindow,
    bounds: WindowBounds,
) -> Result<(), AppError> {
    wait_for(window, Completion::Bounds(bounds)).await
}
