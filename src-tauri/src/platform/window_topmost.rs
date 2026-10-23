//! Always-on-top reporting follows the owning native window.
use crate::error::AppError;

pub(crate) fn read(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    #[cfg(target_os = "linux")]
    return super::window_topmost_linux::read(window);
    #[cfg(not(target_os = "linux"))]
    window
        .is_always_on_top()
        .map_err(|error| AppError::Io(error.to_string()))
}

pub(crate) fn set(window: &tauri::WebviewWindow, value: bool) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    return super::window_topmost_linux::set(window, value);
    #[cfg(not(target_os = "linux"))]
    window
        .set_always_on_top(value)
        .map_err(|error| AppError::Io(error.to_string()))
}
