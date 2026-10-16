// Disposable experiment slot. No product code includes this file.
use windows_sys::Win32::Foundation::{HWND, RECT};

pub unsafe fn install(_parent: HWND, _container: HWND, _rect: RECT) -> Result<HWND, String> {
    Ok(std::ptr::null_mut())
}
