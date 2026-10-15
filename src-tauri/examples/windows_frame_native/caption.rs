//! Measurements of the pinned Electron native overlay, not production acceptance.
use super::native;
use serde_json::{json, Value};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

fn owned(handle: HWND, process: u32) -> Result<(), String> {
    let mut actual = 0;
    unsafe {
        GetWindowThreadProcessId(handle, &mut actual);
    }
    if handle.is_null() || unsafe { IsWindow(handle) } == 0 || actual != process {
        return Err("reference HWND no longer belongs to its owned child".into());
    }
    Ok(())
}

fn geometry(handle: HWND, process: u32) -> Result<Value, String> {
    owned(handle, process)?;
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(handle, &mut bounds) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let dpi = unsafe { GetDpiForWindow(handle) };
    if !(96..=384).contains(&dpi) {
        return Err(format!("unsupported probe DPI: {dpi}"));
    }
    let scale = f64::from(dpi) / 96.0;
    let width = (150.0 * scale).round() as i32;
    if bounds.right - bounds.left < width || bounds.bottom - bounds.top < 40 {
        return Err("reference window is smaller than the caption probe".into());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut rows = Vec::new();
    // Offsets use the outer top, including the restored one-pixel upper border.
    for logical_y in [0, 1, 15, 30, 31, 32, 33] {
        let y = bounds.top + (f64::from(logical_y) * scale).round() as i32;
        let mut runs = Vec::<Value>::new();
        for x in bounds.right - width..bounds.right {
            if Instant::now() >= deadline || i16::try_from(x).is_err() || i16::try_from(y).is_err()
            {
                return Err("caption hit scan exceeded its deadline or Win32 coordinates".into());
            }
            let packed = ((y as u16 as u32) << 16 | x as u16 as u32) as isize;
            let mut hit = 0usize;
            if unsafe {
                SendMessageTimeoutW(
                    handle,
                    WM_NCHITTEST,
                    0,
                    packed,
                    SMTO_ABORTIFHUNG | SMTO_BLOCK,
                    100,
                    &mut hit,
                )
            } == 0
            {
                return Err(format!(
                    "reference WM_NCHITTEST failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let relative = x - bounds.left;
            if let Some(last) = runs.last_mut().filter(|last| last["hit"] == hit as isize) {
                last["end"] = json!(relative + 1);
            } else {
                runs.push(json!({"start":relative,"end":relative+1,"hit":hit as isize}));
            }
        }
        rows.push(json!({"outerLogicalY":logical_y,"screenY":y,"runs":runs}));
    }
    owned(handle, process)?;
    Ok(json!({"dpi":dpi,"maximized":unsafe{IsZoomed(handle)}!=0,
        "outerBounds":[bounds.left,bounds.top,bounds.right,bounds.bottom],"rows":rows}))
}

struct Cursor(Option<POINT>);
impl Cursor {
    fn restore(&mut self) -> Result<(), String> {
        if let Some(point) = self.0 {
            if unsafe { SetCursorPos(point.x, point.y) } == 0 {
                return Err("could not restore the cursor after native hover".into());
            }
            let mut actual = POINT { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut actual) } == 0
                || actual.x != point.x
                || actual.y != point.y
            {
                return Err("restored cursor position did not match its readback".into());
            }
            self.0 = None;
        }
        Ok(())
    }
}

fn hover_readback(handle: HWND, expected: POINT) -> Result<POINT, String> {
    let mut actual = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut actual) } == 0 {
        return Err("native hover cursor readback failed".into());
    }
    if actual.x != expected.x
        || actual.y != expected.y
        || unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) } != handle
    {
        return Err("native hover lost its cursor position or foreground ownership".into());
    }
    Ok(actual)
}
impl Drop for Cursor {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn hover_maximize(handle: HWND, process: u32, map: &Value, path: &Path) -> Result<Value, String> {
    owned(handle, process)?;
    let row = map["rows"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["outerLogicalY"] == 15))
        .ok_or("caption scan has no middle row")?;
    let run = row["runs"]
        .as_array()
        .and_then(|runs| runs.iter().find(|run| run["hit"] == HTMAXBUTTON))
        .ok_or("caption scan has no maximize/restore hit interval")?;
    let x = map["outerBounds"][0].as_i64().ok_or("missing outer x")? as i32
        + ((run["start"].as_i64().ok_or("missing interval start")?
            + run["end"].as_i64().ok_or("missing interval end")?)
            / 2) as i32;
    let y = row["screenY"].as_i64().ok_or("missing scan y")? as i32;
    if unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) } != handle {
        return Err("reference is not the foreground window before native hover".into());
    }
    if unsafe { SetCursorPos(x, y) } == 0 {
        return Err("SetCursorPos failed".into());
    }
    thread::sleep(Duration::from_millis(500));
    owned(handle, process)?;
    let before = hover_readback(handle, POINT { x, y })?;
    let target = unsafe { WindowFromPoint(before) };
    let mut target_process = 0;
    unsafe {
        GetWindowThreadProcessId(target, &mut target_process);
    }
    if unsafe { GetAncestor(target, GA_ROOT) } != handle {
        return Err("native hover is occluded by a window outside the owned reference".into());
    }
    let image = native::capture_owned(handle as usize, process, path, true)?;
    let after = hover_readback(handle, POINT { x, y })?;
    Ok(
        json!({"screenPoint":[x,y],"actualTargetHwnd":target as usize,"actualTargetPid":target_process,
        "cursorBeforeCapture":[before.x,before.y],"cursorAfterCapture":[after.x,after.y],"foregroundOwned":true,
        "width":image.width,"height":image.height,"capture":path}),
    )
}

/// Scan actual native hit boundaries, capture a real hover, and repeat maximized.
/// Maximization here uses WM_SYSCOMMAND, not a mouse click or a production command.
pub fn measure(handle: usize, process: u32, mode: &str, output: &Path) -> Result<Value, String> {
    let handle = handle as HWND;
    owned(handle, process)?;
    let mut original = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut original) } == 0 {
        return Err("GetCursorPos failed".into());
    }
    let mut cursor = Cursor(Some(original));
    let restored = geometry(handle, process)?;
    let restored_hover = hover_maximize(
        handle,
        process,
        &restored,
        &output.join(format!("electron-{mode}-hover-max.png")),
    )?;
    if unsafe { PostMessageW(handle, WM_SYSCOMMAND, SC_MAXIMIZE as usize, 0) } == 0 {
        return Err("owned reference maximize request failed".into());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while unsafe { IsZoomed(handle) } == 0 {
        owned(handle, process)?;
        if Instant::now() >= deadline {
            return Err("reference did not maximize".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(200));
    let maximized = geometry(handle, process)?;
    let maximized_hover = hover_maximize(
        handle,
        process,
        &maximized,
        &output.join(format!("electron-{mode}-hover-restore.png")),
    )?;
    cursor.restore()?;
    Ok(
        json!({"restored":restored,"restoredHover":restored_hover,"maximized":maximized,"maximizedHover":maximized_hover,
        "cursorRestored":true,
        "physicalClickVerified":false,"productionAppVerified":false,"win11SnapVerified":false}),
    )
}
