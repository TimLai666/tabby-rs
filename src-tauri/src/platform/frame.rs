use crate::error::AppError;

pub fn set_window_frame(window: &tauri::WebviewWindow, frame: &str) -> Result<(), AppError> {
    let native = match frame {
        "native" => true,
        "thin" | "full" => false,
        _ => {
            return Err(AppError::InvalidArgument(
                "window frame must be native, thin, or full".into(),
            ));
        }
    };

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSWindow, NSWindowStyleMask, NSWindowTitleVisibility};
        use std::sync::mpsc::sync_channel;

        let (sender, receiver) = sync_channel(1);
        let window = window.clone();
        let main_window = window.clone();
        window
            .run_on_main_thread(move || {
                let result = (|| {
                    let native_window = main_window
                        .ns_window()
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    let native_window: &NSWindow = unsafe { &*native_window.cast() };

                    let current_mask = native_window.styleMask();
                    let full_size_mask = NSWindowStyleMask::FullSizeContentView;
                    let updated_mask = if native {
                        current_mask & !full_size_mask
                    } else {
                        current_mask | full_size_mask
                    };

                    if updated_mask != current_mask {
                        let frame = native_window.frame();
                        let alpha = native_window.alphaValue();
                        let opaque = native_window.isOpaque();
                        let first_responder = native_window.firstResponder();

                        native_window.setStyleMask(updated_mask);
                        if native_window.frame() != frame {
                            native_window.setFrame_display(frame, false);
                        }
                        native_window.setAlphaValue(alpha);
                        native_window.setOpaque(opaque);
                        if let Some(responder) = first_responder.as_deref() {
                            if !native_window.makeFirstResponder(Some(responder)) {
                                return Err(AppError::Io(
                                    "could not restore the window first responder".into(),
                                ));
                            }
                        }
                    }

                    native_window.setTitleVisibility(if native {
                        NSWindowTitleVisibility::Visible
                    } else {
                        NSWindowTitleVisibility::Hidden
                    });
                    native_window.setTitlebarAppearsTransparent(!native);
                    Ok(())
                })();
                let _ = sender.send(result);
            })
            .map_err(|error| AppError::Io(error.to_string()))?;

        return receiver
            .recv()
            .map_err(|error| AppError::Io(error.to_string()))?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, native);
        Err(AppError::Unsupported(
            "window frame is available only on macOS".into(),
        ))
    }
}
