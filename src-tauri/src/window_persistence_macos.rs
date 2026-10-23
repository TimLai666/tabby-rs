//! Construction-time saved-frame restoration; presentation and close are separate operations.
use crate::{error::AppError, platform::window_state_macos, state::AppState};

pub(crate) fn restore_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    let saved = state.begin_window_state(window.label())?;
    if let Err(error) = window_state_macos::restore_startup(window, saved) {
        state.remove_window_state(window.label());
        return Err(error);
    }
    Ok(())
}

/// Drain construction events before allowing subsequent movement to update the cache.
pub(crate) async fn enable_capture(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    crate::platform::wait_for_window_operations(window).await?;
    state.ready_window_state(window.label());
    Ok(())
}

/// Move/resize events sample native logical bounds without writing to disk.
pub(crate) fn capture_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    state.record_window_state(window.label(), window_state_macos::read(window)?);
    Ok(())
}
