//! Live maximized/minimized state read from the native window.
//!
//! Tao caches these flags and updates them after the geometry events that
//! announce a system-initiated maximize (Windows `WM_WINDOWPOSCHANGED`
//! before `WM_SIZE`, GTK `configure-event` before `window-state-event`), so
//! placement capture inside `Moved`/`Resized` handlers must ask the native
//! window instead of the cache.

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowFlags {
    pub maximized: bool,
    pub minimized: bool,
}

/// Returns the live flags, or `None` where the cached Tao values are already
/// live (macOS reads the window frame and miniaturized state directly).
pub(crate) fn live_window_flags(
    window: &tauri::WebviewWindow,
) -> Result<Option<WindowFlags>, AppError> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsZoomed};
        let hwnd = window
            .hwnd()
            .map_err(|error| AppError::Io(error.to_string()))?;
        // SAFETY: the HWND comes from the live Tauri window handle.
        let (maximized, minimized) =
            unsafe { (IsZoomed(hwnd.0 as _) != 0, IsIconic(hwnd.0 as _) != 0) };
        return Ok(Some(WindowFlags {
            maximized,
            minimized,
        }));
    }

    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        use std::sync::mpsc::sync_channel;

        let (sender, receiver) = sync_channel(1);
        let main_window = window.clone();
        window
            .run_on_main_thread(move || {
                let result = main_window
                    .gtk_window()
                    .map_err(|error| AppError::Io(error.to_string()))
                    .map(|native| {
                        native.window().map(|surface| {
                            let state = surface.state();
                            WindowFlags {
                                maximized: state.contains(gtk::gdk::WindowState::MAXIMIZED),
                                minimized: state.contains(gtk::gdk::WindowState::ICONIFIED),
                            }
                        })
                    });
                let _ = sender.send(result);
            })
            .map_err(|error| AppError::Io(error.to_string()))?;
        return receiver
            .recv()
            .map_err(|error| AppError::Io(error.to_string()))?;
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = window;
        Ok(None)
    }
}
