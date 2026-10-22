use crate::error::AppError;
use gtk::prelude::*;
use std::sync::mpsc::sync_channel;

pub(crate) fn initialize(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    let (sender, receiver) = sync_channel(1);
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = main_window
                .gtk_window()
                .map_err(|error| AppError::Io(error.to_string()))
                .and_then(|native| {
                    if native
                        .display()
                        .downcast_ref::<gdkx11::X11Display>()
                        .is_some()
                    {
                        native.set_gravity(gtk::gdk::Gravity::Static);
                    }
                    main_window
                        .default_vbox()
                        .map(|body| {
                            // Window-level geometry hints include GTK client-side decorations.
                            // Constrain the content container so the promised minimum stays usable.
                            let (width, height) = crate::window_geometry::MIN_WINDOW_SIZE;
                            body.set_size_request(width as i32, height as i32);
                        })
                        .map_err(|error| AppError::Io(error.to_string()))
                });
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}
