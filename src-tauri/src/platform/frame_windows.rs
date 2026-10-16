use crate::error::AppError;

use std::sync::mpsc::sync_channel;

#[path = "frame_windows_region.rs"]
mod region;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmDefWindowProc, DwmExtendFrameIntoClientArea};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindow, IsZoomed, SetWindowPos,
    GWL_STYLE, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, SM_CYCAPTION, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, TITLEBARINFOEX, WM_ACTIVATE, WM_DPICHANGED,
    WM_DWMCOMPOSITIONCHANGED, WM_GETTITLEBARINFOEX, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST,
    WM_SHOWWINDOW, WM_STYLECHANGED, WM_WINDOWPOSCHANGED, WS_CAPTION,
};

const FRAME_SUBCLASS_ID: usize = 0x5441_4242;
const FRAME_SUBCLASS_ACTIVE: usize = 1;

pub(super) fn set_window_frame(
    window: &tauri::WebviewWindow,
    native: bool,
) -> Result<(), AppError> {
    let (sender, receiver) = sync_channel(1);
    let window = window.clone();
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let hwnd = main_window
                    .hwnd()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                unsafe { set_window_frame_hwnd(hwnd.0, native) }
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;

    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

unsafe fn set_window_frame_hwnd(hwnd: HWND, native: bool) -> Result<(), AppError> {
    if unsafe { IsWindow(hwnd) } == 0 {
        return Err(io_error(
            "window handle is invalid or the window was destroyed",
        ));
    }

    let mut outer_before = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(hwnd, &mut outer_before) } == 0 {
        return Err(io_error("could not read the window bounds"));
    }

    let mut old_ref_data = 0usize;
    let had_subclass = unsafe {
        GetWindowSubclass(
            hwnd,
            Some(frame_subclass_proc),
            FRAME_SUBCLASS_ID,
            &mut old_ref_data,
        ) != 0
    };
    let was_active = had_subclass && old_ref_data == FRAME_SUBCLASS_ACTIVE;

    if native && !had_subclass {
        return region::remove(hwnd);
    }

    let margins = if native {
        MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 0,
        }
    } else {
        custom_margins(hwnd).map_err(io_error)?
    };

    if !native {
        region::install(hwnd)?;
    }

    if !native
        && unsafe {
            SetWindowSubclass(
                hwnd,
                Some(frame_subclass_proc),
                FRAME_SUBCLASS_ID,
                FRAME_SUBCLASS_ACTIVE,
            )
        } == 0
    {
        let rollback = if had_subclass {
            Ok(())
        } else {
            region::remove(hwnd).map_err(|error| error.to_string())
        };
        return Err(with_rollback(
            "could not install the window frame subclass",
            rollback,
        ));
    }

    if !native && !extend_frame(hwnd, &margins) {
        let rollback = restore_original_frame(hwnd, had_subclass, was_active);
        return Err(with_rollback(
            "DWM could not extend the frame into the caption area",
            rollback,
        ));
    }

    if native {
        if !extend_frame(hwnd, &margins) {
            return Err(with_rollback(
                "DWM could not restore the native frame margins",
                restore_custom_frame(hwnd),
            ));
        }
        if let Err(error) = region::remove(hwnd) {
            return Err(with_rollback(
                &format!("could not restore the native WebView region: {error}"),
                restore_custom_frame(hwnd),
            ));
        }
        if unsafe { RemoveWindowSubclass(hwnd, Some(frame_subclass_proc), FRAME_SUBCLASS_ID) } == 0
        {
            let rollback = restore_custom_frame(hwnd);
            return Err(with_rollback(
                "could not remove the window frame subclass",
                rollback,
            ));
        }
    }

    if unsafe { frame_changed(hwnd) } == 0 {
        let rollback = if native {
            restore_custom_frame(hwnd)
        } else {
            restore_original_frame(hwnd, had_subclass, was_active)
        };
        return Err(with_rollback(
            "could not recalculate the window frame",
            rollback,
        ));
    }

    if !native {
        if let Err(error) = region::refresh(hwnd) {
            return Err(with_rollback(
                &format!("could not expose the native caption controls: {error}"),
                restore_original_frame(hwnd, had_subclass, was_active),
            ));
        }
    }

    let mut outer_after = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(hwnd, &mut outer_after) } == 0 {
        let rollback = if native {
            restore_custom_frame(hwnd)
        } else {
            restore_original_frame(hwnd, had_subclass, was_active)
        };
        return Err(with_rollback(
            "could not verify that the window bounds were preserved",
            rollback,
        ));
    }
    if outer_after.left != outer_before.left
        || outer_after.top != outer_before.top
        || outer_after.right != outer_before.right
        || outer_after.bottom != outer_before.bottom
    {
        let rollback = if native {
            restore_custom_frame(hwnd)
        } else {
            restore_original_frame(hwnd, had_subclass, was_active)
        };
        return Err(with_rollback(
            "recalculating the frame changed the outer window bounds",
            rollback,
        ));
    }

    Ok(())
}

unsafe extern "system" fn frame_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    ref_data: usize,
) -> isize {
    let mut dwm_result = 0;
    let dwm_handled =
        unsafe { DwmDefWindowProc(hwnd, message, wparam, lparam, &mut dwm_result) != 0 };

    if message == WM_NCDESTROY {
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(frame_subclass_proc), subclass_id) };
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }

    if ref_data == FRAME_SUBCLASS_ACTIVE && message == WM_NCCALCSIZE {
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        if style & WS_CAPTION != 0 && lparam != 0 {
            let outer_top = if wparam != 0 {
                let params =
                    lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::NCCALCSIZE_PARAMS;
                unsafe { (*params).rgrc[0].top }
            } else {
                let rect = lparam as *const RECT;
                unsafe { (*rect).top }
            };

            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            let client_top = if unsafe { IsZoomed(hwnd) } != 0 {
                let native_top = if wparam != 0 {
                    let params = lparam
                        as *const windows_sys::Win32::UI::WindowsAndMessaging::NCCALCSIZE_PARAMS;
                    unsafe { (*params).rgrc[0].top }
                } else {
                    unsafe { (*(lparam as *const RECT)).top }
                };
                match caption_height(hwnd) {
                    Ok(height) => native_top.saturating_sub(height),
                    Err(error) => {
                        log_refresh_error(&error);
                        native_top
                    }
                }
            } else {
                outer_top
            };

            if wparam != 0 {
                let params =
                    lparam as *mut windows_sys::Win32::UI::WindowsAndMessaging::NCCALCSIZE_PARAMS;
                unsafe { (*params).rgrc[0].top = client_top };
            } else {
                let rect = lparam as *mut RECT;
                unsafe { (*rect).top = client_top };
            }
            return result;
        }
    }

    // With a maximized top inset, DWM can leave its caption buttons visible
    // but decline hit testing. Keep the native action and current system rects.
    if ref_data == FRAME_SUBCLASS_ACTIVE
        && message == WM_NCHITTEST
        && !dwm_handled
        && unsafe { IsZoomed(hwnd) } != 0
        && unsafe { IsIconic(hwnd) } == 0
        && unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32 & WS_CAPTION == WS_CAPTION
    {
        if let Some(hit) = native_caption_hit(hwnd, lparam) {
            return hit;
        }
    }

    let result = if message != WM_NCCALCSIZE && dwm_handled {
        dwm_result
    } else {
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    };
    if ref_data == FRAME_SUBCLASS_ACTIVE
        && matches!(
            message,
            WM_ACTIVATE
                | WM_DPICHANGED
                | WM_DWMCOMPOSITIONCHANGED
                | WM_SHOWWINDOW
                | WM_STYLECHANGED
                | WM_WINDOWPOSCHANGED
        )
    {
        if matches!(
            message,
            WM_ACTIVATE
                | WM_DPICHANGED
                | WM_DWMCOMPOSITIONCHANGED
                | WM_SHOWWINDOW
                | WM_STYLECHANGED
        ) {
            if let Err(error) = extend_custom_frame(hwnd) {
                log_refresh_error(&error);
            }
        }
        if let Err(error) = region::refresh(hwnd) {
            log_refresh_error(&error.to_string());
        }
    }
    result
}

fn native_caption_hit(hwnd: HWND, lparam: isize) -> Option<isize> {
    const INACTIVE_BUTTON_STATES: u32 = 0x0000_0001 | 0x0000_8000 | 0x0001_0000;
    let mut info: TITLEBARINFOEX = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<TITLEBARINFOEX>() as u32;
    unsafe {
        DefWindowProcW(
            hwnd,
            WM_GETTITLEBARINFOEX,
            0,
            &mut info as *mut TITLEBARINFOEX as isize,
        );
    }
    let x = (lparam as u16) as i16 as i32;
    let y = ((lparam as usize >> 16) as u16) as i16 as i32;
    for (index, hit) in [(2, HTMINBUTTON), (3, HTMAXBUTTON), (5, HTCLOSE)] {
        let rect = info.rgrect[index];
        if info.rgstate[index] & INACTIVE_BUTTON_STATES == 0
            && rect.right > rect.left
            && rect.bottom > rect.top
            && x >= rect.left
            && x < rect.right
            && y >= rect.top
            && y < rect.bottom
        {
            return Some(hit as isize);
        }
    }
    None
}

fn log_refresh_error(error: &str) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr().lock(),
        "Windows frame refresh failed: {error}"
    );
}

unsafe fn frame_changed(hwnd: HWND) -> i32 {
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}

fn extend_frame(hwnd: HWND, margins: &MARGINS) -> bool {
    unsafe { DwmExtendFrameIntoClientArea(hwnd, margins) >= 0 }
}

fn extend_custom_frame(hwnd: HWND) -> Result<(), String> {
    let margins = custom_margins(hwnd)?;
    if extend_frame(hwnd, &margins) {
        Ok(())
    } else {
        Err("could not restore the caption DWM margin".into())
    }
}

fn restore_subclass(hwnd: HWND, had_subclass: bool, was_active: bool) -> Result<(), String> {
    if had_subclass {
        if unsafe {
            SetWindowSubclass(
                hwnd,
                Some(frame_subclass_proc),
                FRAME_SUBCLASS_ID,
                if was_active { FRAME_SUBCLASS_ACTIVE } else { 0 },
            )
        } != 0
        {
            Ok(())
        } else {
            Err("could not restore the previous window frame subclass state".into())
        }
    } else if unsafe { RemoveWindowSubclass(hwnd, Some(frame_subclass_proc), FRAME_SUBCLASS_ID) }
        != 0
    {
        Ok(())
    } else {
        Err("could not remove the newly installed window frame subclass".into())
    }
}

fn restore_original_frame(hwnd: HWND, had_subclass: bool, was_active: bool) -> Result<(), String> {
    let margins = if was_active {
        match custom_margins(hwnd) {
            Ok(margins) => margins,
            Err(error) => return Err(error),
        }
    } else {
        MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 0,
        }
    };
    let mut errors: Vec<String> = Vec::new();
    if !extend_frame(hwnd, &margins) {
        errors.push("could not restore the previous DWM frame margins".into());
    }
    if let Err(error) = restore_subclass(hwnd, had_subclass, was_active) {
        errors.push(error);
    }
    if unsafe { frame_changed(hwnd) } == 0 {
        errors.push("could not recalculate the previous window frame".into());
    }
    let region_result = if was_active {
        region::install(hwnd).and_then(|()| region::refresh(hwnd))
    } else {
        region::remove(hwnd)
    };
    if let Err(error) = region_result {
        errors.push(error.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn restore_custom_frame(hwnd: HWND) -> Result<(), String> {
    let region_result = region::install(hwnd);
    let subclass_ok = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(frame_subclass_proc),
            FRAME_SUBCLASS_ID,
            FRAME_SUBCLASS_ACTIVE,
        ) != 0
    };
    let margins_result = extend_custom_frame(hwnd);
    let mut errors: Vec<String> = Vec::new();
    if let Err(error) = region_result {
        errors.push(error.to_string());
    }
    if !subclass_ok {
        errors.push("could not restore the active window frame subclass".into());
    }
    if let Err(error) = margins_result {
        errors.push(error);
    }
    if unsafe { frame_changed(hwnd) } == 0 {
        errors.push("could not recalculate the restored window frame".into());
    }
    if let Err(error) = region::refresh(hwnd) {
        errors.push(error.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn custom_margins(hwnd: HWND) -> Result<MARGINS, String> {
    if unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32 & WS_CAPTION == 0 {
        return Ok(MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 0,
        });
    }
    Ok(MARGINS {
        cxLeftWidth: 0,
        cxRightWidth: 0,
        cyTopHeight: caption_height(hwnd)?,
        cyBottomHeight: 0,
    })
}

fn caption_height(hwnd: HWND) -> Result<i32, String> {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        return Err("could not determine the window DPI".into());
    }
    let height = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, dpi) };
    if height <= 0 {
        return Err("could not determine the caption height".into());
    }
    Ok(height)
}

fn with_rollback(message: &str, rollback: Result<(), String>) -> AppError {
    match rollback {
        Ok(()) => io_error(message),
        Err(error) => io_error(format!("{message}; rollback also failed: {error}")),
    }
}

fn io_error(message: impl Into<String>) -> AppError {
    AppError::Io(message.into())
}
