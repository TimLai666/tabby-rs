//! Linux saved-frame restoration and logical sampling; disk writes belong to approved close.
use crate::{error::AppError, state::AppState, window_geometry::persistence::LogicalBounds};

/// Register the label from current durable state, then restore native constructor
/// options. Return the raw constructor sample without saving it. On native error,
/// remove only this registration. Duplicate begin must preserve the existing one.
pub(crate) fn restore_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
    initial_origin: Option<(i32, i32)>,
) -> Result<LogicalBounds, AppError> {
    let saved = state.begin_window_state(window.label())?;
    match crate::platform::window_state_linux::restore_startup(window, saved, initial_origin) {
        Ok(constructor) => Ok(constructor),
        Err(error) => {
            state.remove_window_state(window.label());
            Err(error)
        }
    }
}

/// Await native work, then select the owning backend's first-sample policy.
/// X11 records first-display events; Wayland suppresses unchanged startup samples.
/// Arming either path must not itself promote the constructor into saved bounds.
/// Error/cancellation must not arm capture or affect a peer window.
pub(crate) async fn enable_capture(
    window: &tauri::WebviewWindow,
    state: &AppState,
    constructor: LogicalBounds,
) -> Result<(), AppError> {
    crate::platform::wait_for_window_operations(window).await?;
    if crate::platform::supports_absolute_positioning(window)? {
        state.ready_window_state_with_origin(window.label(), constructor);
    } else {
        state.ready_window_state_with_sample(window.label(), constructor);
    }
    Ok(())
}

/// Sample with the latest logical origin, retaining Wayland metadata. Unknown,
/// removed, and origin-less unready labels are no-ops. Never write the state file.
/// Propagate native read errors without changing tracked bounds.
pub(crate) fn capture_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    let Some(origin) = state.window_sampling_bounds(window.label()) else {
        return Ok(());
    };
    if let Some(sample) = crate::platform::window_state_linux::read_for_capture(window, origin)? {
        state.record_window_state(window.label(), sample);
    }
    Ok(())
}
