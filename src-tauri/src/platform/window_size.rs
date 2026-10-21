use crate::error::AppError;
use std::{ptr, sync::mpsc::sync_channel};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageW, SetWindowPos, MINMAXINFO, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
    WM_GETMINMAXINFO,
};

pub fn set_window_outer_size(
    window: &tauri::WebviewWindow,
    size: tauri::PhysicalSize<u32>,
) -> Result<(), AppError> {
    let width = i32::try_from(size.width).map_err(|_| {
        AppError::InvalidArgument("window width exceeds the native size limit".into())
    })?;
    let height = i32::try_from(size.height).map_err(|_| {
        AppError::InvalidArgument("window height exceeds the native size limit".into())
    })?;
    let (sender, receiver) = sync_channel(1);
    let main = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                // Match Tao's resize behavior before writing physical outer bounds.
                main.unmaximize()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                let hwnd = main
                    .hwnd()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                // Tao consumes WM_WINDOWPOSCHANGING without the default size
                // validation. Query the live outer minimum after unmaximizing;
                // SetWindowPos alone accepts even 1x1 on the real Wry host.
                let mut limits: MINMAXINFO = unsafe { std::mem::zeroed() };
                unsafe {
                    SendMessageW(hwnd.0, WM_GETMINMAXINFO, 0, &mut limits as *mut _ as isize);
                }
                let width = width.max(limits.ptMinTrackSize.x);
                let height = height.max(limits.ptMinTrackSize.y);
                // Tao's set_size adds native frame margins even when the custom
                // caption supplies WM_NCCALCSIZE. SetWindowPos takes outer pixels.
                if unsafe {
                    SetWindowPos(
                        hwnd.0,
                        ptr::null_mut(),
                        0,
                        0,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
                    )
                } == 0
                {
                    return Err(AppError::Io(std::io::Error::last_os_error().to_string()));
                }
                Ok(())
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}
