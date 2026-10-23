//! Linux always-on-top adapter; the native-window checks define its contract.
use crate::error::AppError;

pub(crate) fn read(window: &tauri::WebviewWindow) -> Result<bool, AppError> {
    window
        .is_always_on_top()
        .map_err(|error| AppError::Io(error.to_string()))
}

pub(crate) fn set(window: &tauri::WebviewWindow, value: bool) -> Result<(), AppError> {
    crate::platform::window_state_linux::on_main_thread(window, move |window| {
        // Resolve the live native window before dispatching the setter.
        window
            .gtk_window()
            .map_err(|error| AppError::Io(error.to_string()))?;
        window
            .set_always_on_top(value)
            .map_err(|error| AppError::Io(error.to_string()))
    })
}
