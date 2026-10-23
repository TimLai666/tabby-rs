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
                    } else {
                        initialize_wayland_decoration(&native)?;
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

/// Keep native decoration changes from altering an acknowledged content resize.
/// Preserve theme styles, global animation preferences, and unrelated windows.
fn initialize_wayland_decoration(window: &gtk::ApplicationWindow) -> Result<(), AppError> {
    let provider = gtk::CssProvider::new();
    // GTK adds the currently animated shadow extents to resize requests. An
    // unmaximize frame can therefore acknowledge normal state while a later
    // decoration frame changes the requested content dimensions. Keep the
    // theme's final decoration style without that geometry-changing transition.
    provider
        .load_from_data(b"decoration { transition: none; }")
        .map_err(|error| {
            AppError::Io(format!(
                "could not initialize Wayland window geometry: {error}"
            ))
        })?;
    window
        .style_context()
        .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    Ok(())
}
