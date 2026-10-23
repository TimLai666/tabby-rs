//! Renderer-approved close persists the owning window before native destruction.
use crate::{error::AppError, state::AppState};
use tauri::Emitter;

struct CloseGuard<'a> {
    window: &'a tauri::WebviewWindow,
    state: &'a AppState,
    label: &'a str,
    destroyed: bool,
}

impl Drop for CloseGuard<'_> {
    fn drop(&mut self) {
        if self.destroyed {
            return;
        }

        self.state.launches().set_closing(self.label, false);
        if let Err(error) = self.window.emit_to(self.label, "app:launch", ()) {
            eprintln!(
                "failed to resume launches for window {}: {error}",
                self.label
            );
        }
    }
}

/// Serialize with geometry, persist the live maximized state, then destroy.
/// Duplicate requests do nothing; failed or cancelled closes restore launch routing.
pub(crate) async fn close_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    let label = window.label();
    if !state.launches().begin_close(label) {
        return Ok(());
    }

    // Install the rollback before the first await so cancellation restores routing.
    let mut close = CloseGuard {
        window,
        state,
        label,
        destroyed: false,
    };

    let _operations = state.lock_window_operations().await;
    crate::platform::wait_for_window_operations(window).await?;

    #[cfg(not(target_os = "linux"))]
    let snapshot = crate::platform::window_state_macos::read(window);
    #[cfg(target_os = "linux")]
    let snapshot = state
        .window_sampling_bounds(label)
        .ok_or_else(|| AppError::Io(format!("window state is not registered for {label}")))
        .and_then(|origin| crate::platform::window_state_linux::read(window, origin));

    match snapshot {
        Ok(saved) => {
            if let Err(error) = state.save_window_state(label, saved.maximized) {
                eprintln!("failed to save state for closing window {label}: {error:?}");
            }
        }
        Err(error) => {
            eprintln!("failed to read state for closing window {label}: {error:?}");
        }
    }

    window
        .destroy()
        .map_err(|error| AppError::Io(error.to_string()))?;
    close.destroyed = true;
    Ok(())
}
