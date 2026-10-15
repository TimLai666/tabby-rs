//! Owned native hit regions. Visual controls are supplied by the Tauri renderer.
use super::WindowsCaptionSnapshot;
use crate::error::AppError;

pub fn prepare_caption_controls(_window: &tauri::WebviewWindow) -> Result<(), AppError> {
    Err(AppError::Unsupported(
        "Windows caption preparation is not implemented".into(),
    ))
}

pub fn caption_state(
    _window: &tauri::WebviewWindow,
) -> Result<Option<WindowsCaptionSnapshot>, AppError> {
    Err(AppError::Unsupported(
        "Windows caption state is not implemented".into(),
    ))
}

pub fn set_caption_frame(_window: &tauri::WebviewWindow, _frame: &str) -> Result<(), AppError> {
    Err(AppError::Unsupported(
        "Windows caption input is not implemented".into(),
    ))
}
