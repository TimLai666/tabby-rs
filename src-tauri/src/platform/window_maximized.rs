//! Bridge maximization follows each native host's fixed-original predicate.
use crate::error::AppError;

pub(crate) fn read(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    #[cfg(target_os = "linux")]
    return crate::platform::window_state_linux::maximized(window);
    #[cfg(target_os = "macos")]
    return crate::platform::window_state_macos::maximized(window);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    window
        .is_maximized()
        .map_err(|error| AppError::Io(error.to_string()))
}
