use crate::error::AppError;
use objc2_app_kit::NSWindow;
use objc2_foundation::NSRect;

fn target_outer_frame(
    native_window: &NSWindow,
    size: tauri::PhysicalSize<u32>,
) -> Result<NSRect, AppError> {
    let scale = native_window.backingScaleFactor();
    if !scale.is_finite() || scale <= 0.0 {
        return Err(AppError::Io(
            "window has an invalid backing scale factor".into(),
        ));
    }

    let mut frame = native_window.frame();
    let minimum = native_window.minSize();
    let height = (size.height as f64 / scale).max(minimum.height);
    frame.origin.y += frame.size.height - height;
    frame.size.width = (size.width as f64 / scale).max(minimum.width);
    frame.size.height = height;
    let content = native_window.contentRectForFrameRect(frame);
    if content.size.width < 0.0 || content.size.height < 0.0 {
        return Err(AppError::InvalidArgument(
            "requested outer size cannot contain the window content area".into(),
        ));
    }
    Ok(frame)
}

pub fn set_window_outer_size(
    window: &tauri::WebviewWindow,
    size: tauri::PhysicalSize<u32>,
) -> Result<(), AppError> {
    if size.width > i32::MAX as u32 || size.height > i32::MAX as u32 {
        return Err(AppError::InvalidArgument(
            "window dimensions exceed the supported size limit".into(),
        ));
    }

    use std::sync::mpsc::sync_channel;

    let (sender, receiver) = sync_channel(1);
    queue_window_outer_size(window, size, Some(sender), None)?;

    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))??;

    Ok(())
}

pub async fn set_window_outer_size_async(
    window: &tauri::WebviewWindow,
    size: tauri::PhysicalSize<u32>,
) -> Result<(), AppError> {
    if size.width > i32::MAX as u32 || size.height > i32::MAX as u32 {
        return Err(AppError::InvalidArgument(
            "window dimensions exceed the supported size limit".into(),
        ));
    }

    let (sender, receiver) = tokio::sync::oneshot::channel();
    queue_window_outer_size(window, size, None, Some(sender))?;
    receiver
        .await
        .map_err(|error| AppError::Io(error.to_string()))?
}

pub async fn wait_for_window_operations(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            dispatch2::DispatchQueue::main().exec_async(move || {
                let _ = sender.send(Ok(()));
            });
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .await
        .map_err(|error| AppError::Io(error.to_string()))?
}

fn queue_window_outer_size(
    window: &tauri::WebviewWindow,
    size: tauri::PhysicalSize<u32>,
    mut queued_completion: Option<std::sync::mpsc::SyncSender<Result<(), AppError>>>,
    mut applied_completion: Option<tokio::sync::oneshot::Sender<Result<(), AppError>>>,
) -> Result<(), AppError> {
    let window = window.clone();
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let preflight = (|| {
                let native_window = main_window
                    .ns_window()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                // SAFETY: Tauri owns this live NSWindow and this callback runs on AppKit's main thread.
                let native_window: &NSWindow = unsafe { &*native_window.cast() };
                target_outer_frame(native_window, size).map(|_| ())
            })();
            if let Err(error) = preflight {
                if let Some(sender) = queued_completion.take() {
                    let _ = sender.send(Err(error));
                } else if let Some(sender) = applied_completion.take() {
                    let _ = sender.send(Err(error));
                }
                return;
            }

            // Re-read AppKit state in FIFO order so queued zoom or ready-style changes run first.
            dispatch2::DispatchQueue::main().exec_async(move || {
                let result = (|| {
                    let native_window = main_window
                        .ns_window()
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    // SAFETY: This block runs on the main GCD queue and Tauri owns the live NSWindow.
                    let native_window: &NSWindow = unsafe { &*native_window.cast() };
                    let frame = target_outer_frame(native_window, size)?;
                    native_window.setFrame_display(frame, false);
                    Ok(())
                })();
                if let Some(sender) = applied_completion.take() {
                    let _ = sender.send(result);
                } else if let Err(error) = result {
                    eprintln!("could not set macOS window outer frame: {error}");
                }
            });
            if let Some(sender) = queued_completion.take() {
                let _ = sender.send(Ok(()));
            }
        })
        .map_err(|error| AppError::Io(error.to_string()))
}
