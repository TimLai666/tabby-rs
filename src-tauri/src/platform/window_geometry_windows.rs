use crate::error::AppError;
use std::{ptr, sync::mpsc::sync_channel};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        HiDpi::GetDpiForWindow,
        Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetWindowThreadProcessId, IsWindow, SetWindowPos, MINMAXINFO, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOZORDER, WM_GETMINMAXINFO, WM_NCDESTROY,
        },
    },
};

const SUBCLASS_ID: usize = 0x5442_474d;
const MARKER: usize = 0x5747_454f;
const INITIALIZING: usize = MARKER + 2;
const INITIALIZED: usize = MARKER;

pub(crate) fn initialize(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    let (sender, receiver) = sync_channel(1);
    let main = window.clone();
    window
        .run_on_main_thread(move || {
            let result = main
                .hwnd()
                .map_err(|error| AppError::Io(error.to_string()))
                .and_then(|hwnd| initialize_hwnd(hwnd.0));
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

fn initialize_hwnd(hwnd: HWND) -> Result<(), AppError> {
    unsafe {
        if hwnd.is_null() || IsWindow(hwnd) == 0 {
            return Err(AppError::InvalidArgument(
                "invalid or destroyed window handle".into(),
            ));
        }
        if GetWindowThreadProcessId(hwnd, ptr::null_mut()) != GetCurrentThreadId() {
            return Err(AppError::InvalidArgument(
                "window must be initialized on its owning thread".into(),
            ));
        }

        let mut existing = 0usize;
        if GetWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID, &mut existing) != 0 {
            return if existing == INITIALIZED {
                Ok(())
            } else {
                Err(AppError::Io(
                    "window geometry subclass is already owned by another initializer".into(),
                ))
            };
        }

        let dpi = GetDpiForWindow(hwnd);
        let width = scaled(800, dpi).ok_or_else(|| {
            AppError::InvalidArgument("invalid DPI or scaled window width".into())
        })?;
        let height = scaled(600, dpi).ok_or_else(|| {
            AppError::InvalidArgument("invalid DPI or scaled window height".into())
        })?;
        if SetWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID, INITIALIZING) == 0 {
            return Err(native_error());
        }
        if !has_binding(hwnd, INITIALIZING) {
            return Err(AppError::Io(
                "window geometry subclass ownership changed during initialization".into(),
            ));
        }

        if SetWindowPos(
            hwnd,
            ptr::null_mut(),
            0,
            0,
            width,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        ) == 0
        {
            let error = native_error();
            remove_if_owned(hwnd, INITIALIZING);
            return Err(error);
        }

        if !has_binding(hwnd, INITIALIZING) {
            return Err(AppError::Io(
                "window geometry subclass ownership changed during resize".into(),
            ));
        }
        if SetWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID, INITIALIZED) == 0 {
            let error = native_error();
            remove_if_owned(hwnd, INITIALIZING);
            return Err(error);
        }
        if !has_binding(hwnd, INITIALIZED) {
            return Err(AppError::Io(
                "window geometry subclass ownership changed after resize".into(),
            ));
        }
        Ok(())
    }
}

fn scaled(dip: u32, dpi: u32) -> Option<i32> {
    if dpi == 0 {
        return None;
    }
    let numerator = u64::from(dip).checked_mul(u64::from(dpi))?;
    let rounded = numerator.checked_add(48)? / 96;
    i32::try_from(rounded).ok()
}

unsafe fn has_binding(hwnd: HWND, expected: usize) -> bool {
    let mut data = 0usize;
    GetWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID, &mut data) != 0 && data == expected
}

unsafe fn remove_if_owned(hwnd: HWND, expected: usize) {
    if has_binding(hwnd, expected) {
        RemoveWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID);
    }
}

fn native_error() -> AppError {
    AppError::Io(std::io::Error::last_os_error().to_string())
}

unsafe extern "system" fn minimum_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        if data == INITIALIZING || data == INITIALIZED {
            remove_if_owned(hwnd, data);
        }
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }

    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    if message == WM_GETMINMAXINFO
        && (data == INITIALIZING || data == INITIALIZED)
        && has_binding(hwnd, data)
        && lparam != 0
    {
        let dpi = GetDpiForWindow(hwnd);
        if let (Some(width), Some(height)) = (scaled(400, dpi), scaled(300, dpi)) {
            let info = &mut *(lparam as *mut MINMAXINFO);
            info.ptMinTrackSize.x = width;
            info.ptMinTrackSize.y = height;
        }
    }
    result
}

#[cfg(test)]
#[path = "window_geometry_windows_tests.rs"]
mod tests;
