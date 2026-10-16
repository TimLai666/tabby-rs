use crate::{desktop::WindowCaptionState, error::AppError};
use std::sync::mpsc::sync_channel;
use tauri::Emitter;
#[path = "frame_windows_buttons.rs"]
mod buttons;

fn app_error(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

pub(super) fn set_window_frame(
    window: &tauri::WebviewWindow,
    native: bool,
) -> Result<(), AppError> {
    let (tx, rx) = sync_channel(1);
    let main = window.clone();
    let emitter = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let hwnd = main.hwnd().map_err(app_error)?.0;
                if native {
                    buttons::remove(hwnd).map_err(app_error)?;
                } else {
                    let label = emitter.label().to_owned();
                    let window_for_event = emitter.clone();
                    buttons::install(
                        hwnd,
                        Box::new(move |state| {
                            if let Err(error) = window_for_event.emit_to(
                                &label,
                                "desktop:windowCaptionChanged",
                                state,
                            ) {
                                eprintln!("could not emit desktop:windowCaptionChanged to {label}: {error}");
                            }
                        }),
                    )
                    .map_err(app_error)?;
                }
                Ok(())
            })();
            let _ = tx.send(result);
        })
        .map_err(app_error)?;
    rx.recv().map_err(app_error)?
}

pub(super) fn caption_controls(
    window: &tauri::WebviewWindow,
) -> Result<Option<WindowCaptionState>, AppError> {
    let (tx, rx) = sync_channel(1);
    let main = window.clone();
    window
        .run_on_main_thread(move || {
            let result = main
                .hwnd()
                .map_err(app_error)
                .and_then(|hwnd| buttons::snapshot(hwnd.0).map_err(app_error));
            let _ = tx.send(result);
        })
        .map_err(app_error)?;
    rx.recv().map_err(app_error)?
}
