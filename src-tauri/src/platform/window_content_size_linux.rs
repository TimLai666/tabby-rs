use crate::error::AppError;
use gtk::prelude::*;
use std::sync::mpsc::sync_channel;

pub(crate) fn window_content_size(
    window: &tauri::WebviewWindow,
) -> Result<tauri::PhysicalSize<u32>, AppError> {
    let (sender, receiver) = sync_channel(1);
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = main_window
                .gtk_window()
                .map(|native| {
                    // GTK retains requested content dimensions while hidden; Tao's
                    // configure-event cache can contain a position or an old size.
                    let (width, height) = native.size();
                    tauri::LogicalSize::new(width, height).to_physical(native.scale_factor() as f64)
                })
                .map_err(|error| AppError::Io(error.to_string()));
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}
