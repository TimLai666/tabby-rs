use crate::error::AppError;

use std::ffi::c_void;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS};
use windows_sys::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, EqualRgn, GetWindowRgn, SetWindowRgn, ERROR, RGN_DIFF,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetParent, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, GWL_STYLE, WM_DPICHANGED_AFTERPARENT, WM_NCDESTROY, WM_SHOWWINDOW,
    WM_WINDOWPOSCHANGED, WS_CAPTION,
};

const CHILD_CLASS: &[u16] = &[
    b'W' as u16,
    b'R' as u16,
    b'Y' as u16,
    b'_' as u16,
    b'W' as u16,
    b'E' as u16,
    b'B' as u16,
    b'V' as u16,
    b'I' as u16,
    b'E' as u16,
    b'W' as u16,
    0,
];
const REGION_SUBCLASS_ID: usize = 0x5441_4252;

pub(super) fn install(parent: HWND) -> Result<(), AppError> {
    let child = find_child(parent)?;
    let mut ref_data = 0usize;
    if unsafe {
        GetWindowSubclass(
            child,
            Some(region_subclass_proc),
            REGION_SUBCLASS_ID,
            &mut ref_data,
        )
    } != 0
    {
        if ref_data != parent as usize {
            return Err(io_error(
                "the Wry child has a conflicting caption-region subclass",
            ));
        }
        return refresh(parent);
    }

    if unsafe {
        SetWindowSubclass(
            child,
            Some(region_subclass_proc),
            REGION_SUBCLASS_ID,
            parent as usize,
        )
    } == 0
    {
        return Err(io_error(
            "could not install the Wry child caption-region subclass",
        ));
    }

    if let Err(error) = refresh(parent) {
        let mut failures = vec![error.to_string()];
        if unsafe { RemoveWindowSubclass(child, Some(region_subclass_proc), REGION_SUBCLASS_ID) }
            == 0
        {
            failures.push("could not remove the newly installed subclass".into());
        }
        if unsafe { IsWindow(child) } != 0
            && unsafe { SetWindowRgn(child, std::ptr::null_mut(), 1) } == 0
        {
            failures.push("could not clear the Wry child region during rollback".into());
        }
        return Err(io_error(failures.join("; ")));
    }
    Ok(())
}

pub(super) fn refresh(parent: HWND) -> Result<(), AppError> {
    let child = find_child(parent)?;
    let mut ref_data = 0usize;
    if unsafe {
        GetWindowSubclass(
            child,
            Some(region_subclass_proc),
            REGION_SUBCLASS_ID,
            &mut ref_data,
        )
    } == 0
    {
        return Err(io_error(
            "the owned Wry child subclass is missing or changed",
        ));
    }
    if ref_data == 0 {
        return Ok(());
    }
    if ref_data != parent as usize {
        return Err(io_error("the Wry child subclass belongs to another window"));
    }

    if unsafe { SetWindowSubclass(child, Some(region_subclass_proc), REGION_SUBCLASS_ID, 0) } == 0 {
        return Err(io_error(
            "could not enter the caption-region reentrancy guard",
        ));
    }

    let result = refresh_guarded(parent, child);
    let mut current = 0usize;
    if unsafe { IsWindow(child) } != 0
        && find_child(parent).ok() == Some(child)
        && unsafe {
            GetWindowSubclass(
                child,
                Some(region_subclass_proc),
                REGION_SUBCLASS_ID,
                &mut current,
            )
        } != 0
        && current == 0
    {
        if unsafe {
            SetWindowSubclass(
                child,
                Some(region_subclass_proc),
                REGION_SUBCLASS_ID,
                parent as usize,
            )
        } == 0
        {
            return match result {
                Ok(()) => Err(io_error(
                    "could not leave the caption-region reentrancy guard",
                )),
                Err(error) => Err(io_error(format!(
                    "{error}; could not leave the caption-region reentrancy guard"
                ))),
            };
        }
    } else {
        return Err(io_error(match result {
            Ok(()) => "the caption-region context changed during refresh".to_string(),
            Err(error) => format!("{error}; the caption-region context changed during refresh"),
        }));
    }
    result
}

pub(super) fn remove(parent: HWND) -> Result<(), AppError> {
    let child = match find_child(parent) {
        Ok(child) => child,
        Err(AppError::NotFound(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut ref_data = 0usize;
    if unsafe {
        GetWindowSubclass(
            child,
            Some(region_subclass_proc),
            REGION_SUBCLASS_ID,
            &mut ref_data,
        )
    } == 0
    {
        return Ok(());
    }
    if ref_data != parent as usize {
        return Err(io_error("refusing to remove a foreign Wry child subclass"));
    }

    if unsafe { RemoveWindowSubclass(child, Some(region_subclass_proc), REGION_SUBCLASS_ID) } == 0 {
        return Err(io_error("could not remove the owned Wry child subclass"));
    }
    if unsafe { SetWindowRgn(child, std::ptr::null_mut(), 1) } != 0 {
        return Ok(());
    }

    let original_error = "could not clear the owned Wry child region";
    let rollback = if unsafe {
        SetWindowSubclass(
            child,
            Some(region_subclass_proc),
            REGION_SUBCLASS_ID,
            parent as usize,
        )
    } != 0
    {
        ""
    } else {
        "; rollback could not restore the owned subclass"
    };
    Err(io_error(format!("{original_error}{rollback}")))
}

fn refresh_guarded(parent: HWND, child: HWND) -> Result<(), AppError> {
    if unsafe { IsIconic(parent) } != 0
        || unsafe { IsWindowVisible(parent) } == 0
        || unsafe { IsIconic(child) } != 0
        || unsafe { IsWindowVisible(child) } == 0
    {
        return Ok(());
    }

    let mut parent_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut child_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(parent, &mut parent_rect) } == 0
        || unsafe { GetWindowRect(child, &mut child_rect) } == 0
    {
        return Err(io_error(
            "could not read the parent and Wry child window bounds",
        ));
    }
    let parent_width = i64::from(parent_rect.right) - i64::from(parent_rect.left);
    let parent_height = i64::from(parent_rect.bottom) - i64::from(parent_rect.top);
    let width = i64::from(child_rect.right) - i64::from(child_rect.left);
    let height = i64::from(child_rect.bottom) - i64::from(child_rect.top);
    if parent_width <= 0 || parent_height <= 0 || width <= 0 || height <= 0 {
        return Ok(());
    }
    let width = i32::try_from(width)
        .map_err(|_| io_error("the Wry child width exceeds GDI region coordinates"))?;
    let height = i32::try_from(height)
        .map_err(|_| io_error("the Wry child height exceeds GDI region coordinates"))?;

    let style = unsafe { GetWindowLongPtrW(parent, GWL_STYLE) } as u32;
    if style & WS_CAPTION == 0 {
        return set_region(child, std::ptr::null_mut());
    }

    let mut buttons = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let hr = unsafe {
        DwmGetWindowAttribute(
            parent,
            DWMWA_CAPTION_BUTTON_BOUNDS as u32,
            &mut buttons as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if hr < 0 || buttons.right <= buttons.left || buttons.bottom <= buttons.top {
        return Err(io_error(format!(
            "DWM returned invalid caption-button bounds (HRESULT 0x{:08x})",
            hr as u32
        )));
    }

    let left = i64::from(parent_rect.left) + i64::from(buttons.left) - i64::from(child_rect.left);
    let top = i64::from(parent_rect.top) + i64::from(buttons.top) - i64::from(child_rect.top);
    let right = i64::from(parent_rect.left) + i64::from(buttons.right) - i64::from(child_rect.left);
    let bottom = i64::from(parent_rect.top) + i64::from(buttons.bottom) - i64::from(child_rect.top);
    let hole = RECT {
        left: left.max(0).min(i64::from(width)) as i32,
        top: top.max(0).min(i64::from(height)) as i32,
        right: right.max(0).min(i64::from(width)) as i32,
        bottom: bottom.max(0).min(i64::from(height)) as i32,
    };
    if hole.right <= hole.left || hole.bottom <= hole.top {
        return set_region(child, std::ptr::null_mut());
    }

    let full = unsafe { CreateRectRgn(0, 0, width, height) };
    if full.is_null() {
        return Err(io_error("could not allocate the Wry child region"));
    }
    let hole_region = unsafe { CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom) };
    if hole_region.is_null() {
        unsafe { DeleteObject(full) };
        return Err(io_error("could not allocate the caption-button region"));
    }
    if unsafe { CombineRgn(full, full, hole_region, RGN_DIFF) } == ERROR {
        unsafe {
            DeleteObject(full);
            DeleteObject(hole_region);
        }
        return Err(io_error("could not subtract the caption-button region"));
    }
    unsafe { DeleteObject(hole_region) };
    set_region(child, full)
}

fn set_region(
    child: HWND,
    region: windows_sys::Win32::Graphics::Gdi::HRGN,
) -> Result<(), AppError> {
    if !region.is_null() {
        let current = unsafe { CreateRectRgn(0, 0, 0, 0) };
        if !current.is_null() {
            let kind = unsafe { GetWindowRgn(child, current) };
            if kind != ERROR && unsafe { EqualRgn(current, region) } != 0 {
                unsafe { DeleteObject(current) };
                unsafe { DeleteObject(region) };
                return Ok(());
            }
            unsafe { DeleteObject(current) };
        }
    }
    if unsafe { SetWindowRgn(child, region, 1) } == 0 {
        if !region.is_null() {
            unsafe { DeleteObject(region) };
        }
        return Err(io_error("SetWindowRgn failed for the Wry child"));
    }
    Ok(())
}

fn find_child(parent: HWND) -> Result<HWND, AppError> {
    if parent.is_null() || unsafe { IsWindow(parent) } == 0 {
        return Err(io_error("the parent window is invalid or destroyed"));
    }
    let first = unsafe {
        FindWindowExW(
            parent,
            std::ptr::null_mut(),
            CHILD_CLASS.as_ptr(),
            std::ptr::null(),
        )
    };
    if first.is_null() {
        return Err(AppError::NotFound(
            "direct WRY_WEBVIEW child was not found".into(),
        ));
    }
    let second = unsafe { FindWindowExW(parent, first, CHILD_CLASS.as_ptr(), std::ptr::null()) };
    if !second.is_null() {
        return Err(io_error("multiple direct WRY_WEBVIEW children were found"));
    }
    if unsafe { IsWindow(first) } == 0 || unsafe { GetParent(first) } != parent {
        return Err(io_error(
            "the Wry child is invalid or is not a direct child",
        ));
    }
    let mut parent_process = 0u32;
    let mut child_process = 0u32;
    let parent_thread = unsafe { GetWindowThreadProcessId(parent, &mut parent_process) };
    let child_thread = unsafe { GetWindowThreadProcessId(first, &mut child_process) };
    if parent_thread == 0
        || child_thread == 0
        || parent_thread != child_thread
        || parent_process != child_process
        || parent_thread != unsafe { GetCurrentThreadId() }
        || parent_process != unsafe { GetCurrentProcessId() }
    {
        return Err(io_error(
            "the Wry child is not on the parent's UI thread and process",
        ));
    }
    Ok(first)
}

unsafe extern "system" fn region_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    ref_data: usize,
) -> isize {
    if subclass_id == REGION_SUBCLASS_ID && message == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(hwnd, Some(region_subclass_proc), REGION_SUBCLASS_ID) };
    }
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if subclass_id == REGION_SUBCLASS_ID
        && ref_data != 0
        && matches!(
            message,
            WM_WINDOWPOSCHANGED | WM_DPICHANGED_AFTERPARENT | WM_SHOWWINDOW
        )
    {
        let refresh_result = catch_unwind(AssertUnwindSafe(|| {
            if let Err(error) = refresh(ref_data as HWND) {
                log_callback_error(&error.to_string());
            }
        }));
        if refresh_result.is_err() {
            log_callback_error("caption-region refresh panicked");
        }
    }
    result
}

fn log_callback_error(error: &str) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _ = writeln!(
            std::io::stderr().lock(),
            "Windows caption region refresh failed: {error}"
        );
    }));
}

fn io_error(message: impl Into<String>) -> AppError {
    AppError::Io(message.into())
}
