//! First-finished-page Linux presentation and saved maximization.
use crate::{error::AppError, state::AppState};

/// Run once after restoration/capture is ready. Serialize with geometry and drain
/// dispatched native work. The fixed original leaves Linux native theme unchanged.
/// Hidden main startup must not maximize/show/focus. Otherwise apply the label's
/// saved maximization before showing/focusing. Recheck removal/approved close
/// after each awaited boundary; propagate native errors without fallback.
pub(crate) async fn present_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    let _guard = state.lock_window_operations().await;
    let label = window.label();
    if state.startup_window_maximized(label).is_none() || state.launches().is_closing(label) {
        return Ok(());
    }

    crate::platform::wait_for_window_operations(window).await?;
    if state.startup_window_hidden(label)
        || state.launches().is_closing(label)
        || state.startup_window_maximized(label).is_none()
    {
        return Ok(());
    }

    if state.startup_window_maximized(label) == Some(true) {
        window
            .maximize()
            .map_err(|error| AppError::Io(error.to_string()))?;
        crate::platform::wait_for_window_operations(window).await?;
    }

    if state.launches().is_closing(label) || state.startup_window_maximized(label).is_none() {
        return Ok(());
    }
    window
        .show()
        .map_err(|error| AppError::Io(error.to_string()))?;
    window
        .set_focus()
        .map_err(|error| AppError::Io(error.to_string()))?;
    crate::platform::wait_for_window_operations(window).await?;
    Ok(())
}
