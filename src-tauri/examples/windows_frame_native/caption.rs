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

fn hit_test(handle: HWND, x: i32, y: i32, deadline: Instant) -> Result<isize, String> {
    if Instant::now() >= deadline || i16::try_from(x).is_err() || i16::try_from(y).is_err() {
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
    Ok(hit as isize)
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
            let hit = hit_test(handle, x, y, deadline)?;
            let relative = x - bounds.left;
            if let Some(last) = runs.last_mut().filter(|last| last["hit"] == hit as isize) {
                last["end"] = json!(relative + 1);
            } else {
                runs.push(json!({"start":relative,"end":relative+1,"hit":hit as isize}));
            }
        }
        rows.push(json!({"outerLogicalY":logical_y,"screenY":y,"runs":runs}));
    }
    let middle = rows
        .iter()
        .find(|row| row["outerLogicalY"] == 15)
        .and_then(|row| row["runs"].as_array())
        .ok_or("caption middle row is missing")?;
    let buttons = middle
        .iter()
        .filter(|run| matches!(run["hit"].as_i64(), Some(8 | 9 | 20)))
        .collect::<Vec<_>>();
    let left = buttons
        .iter()
        .filter_map(|run| run["start"].as_i64())
        .min()
        .ok_or("caption button interval is missing")?;
    let right = buttons
        .iter()
        .filter_map(|run| run["end"].as_i64())
        .max()
        .ok_or("caption button interval is missing")?;
    let minimize = buttons
        .iter()
        .find(|run| run["hit"] == HTMINBUTTON)
        .ok_or("minimize interval is missing")?;
    let center_x = bounds.left
        + ((minimize["start"].as_i64().ok_or("minimize start missing")?
            + minimize["end"].as_i64().ok_or("minimize end missing")?)
            / 2) as i32;
    let mut vertical = Vec::new();
    let scan_height = (64.0 * scale).ceil() as i32;
    for relative_y in 0..scan_height {
        if hit_test(handle, center_x, bounds.top + relative_y, deadline)? == HTMINBUTTON as isize {
            if vertical
                .last()
                .is_some_and(|previous| relative_y != previous + 1)
            {
                return Err("caption button has a discontinuous vertical interval".into());
            }
            vertical.push(relative_y);
        }
    }
    let top = *vertical
        .first()
        .ok_or("caption button vertical interval is missing")?;
    let bottom = *vertical
        .last()
        .ok_or("caption button vertical interval is missing")?
        + 1;
    if bottom == scan_height {
        return Err("caption button extends beyond vertical scan".into());
    }
    owned(handle, process)?;
    Ok(json!({"dpi":dpi,"maximized":unsafe{IsZoomed(handle)}!=0,
        "outerBounds":[bounds.left,bounds.top,bounds.right,bounds.bottom],"rows":rows,
        "nativeButtonClip":{"x":left,"y":top,"width":right-left,"height":bottom-top}}))
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

fn capture_tooltip(
    parent: HWND,
    process: u32,
    cursor: POINT,
    path: &Path,
) -> Result<Value, String> {
    struct Search {
        parent: HWND,
        process: u32,
        cursor: POINT,
        candidates: Vec<(usize, Value)>,
        observed: Vec<Value>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: isize) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
        if process != search.process
            || hwnd == search.parent
            || unsafe { IsWindowVisible(hwnd) } == 0
        {
            return 1;
        }
        let mut bounds: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut bounds) } == 0 {
            return 1;
        }
        let width = i64::from(bounds.right) - i64::from(bounds.left);
        let height = i64::from(bounds.bottom) - i64::from(bounds.top);
        let root_owner = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
        if search.observed.len() < 32 {
            search
                .observed
                .push(json!({"hwnd":hwnd as usize,"rootOwner":root_owner as usize,
                "owner":unsafe{GetWindow(hwnd,GW_OWNER)} as usize,
                "screenBounds":[bounds.left,bounds.top,bounds.right,bounds.bottom]}));
        }
        if root_owner != search.parent
            || !(18..=800).contains(&width)
            || !(10..=96).contains(&height)
            || (i64::from(bounds.top) - i64::from(search.cursor.y)).abs() > 96
            || (i64::from(bounds.left) - i64::from(search.cursor.x)).abs() > 800
        {
            return 1;
        }
        let mut class = [0u16; 128];
        let length =
            unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) }.max(0) as usize;
        search.candidates.push((
            hwnd as usize,
            json!({"hwnd":hwnd as usize,"processId":process,
            "owner":unsafe{GetWindow(hwnd,GW_OWNER)} as usize,"rootOwner":search.parent as usize,
            "class":String::from_utf16_lossy(&class[..length]),"dpi":unsafe{GetDpiForWindow(hwnd)},
            "screenBounds":[bounds.left,bounds.top,bounds.right,bounds.bottom]}),
        ));
        1
    }
    let mut search = Search {
        parent,
        process,
        cursor,
        candidates: Vec::new(),
        observed: Vec::new(),
    };
    if unsafe { EnumWindows(Some(visit), (&mut search as *mut Search) as isize) } == 0 {
        return Err(format!(
            "EnumWindows(owned tooltip): {}",
            std::io::Error::last_os_error()
        ));
    }
    if search.candidates.len() != 1 {
        return Err(format!(
            "expected one visible owned tooltip candidate: {}",
            json!({"candidates":search.candidates,"observedOwnedVisibleWindows":search.observed})
        ));
    }
    let (hwnd, window) = search
        .candidates
        .pop()
        .expect("one owned tooltip candidate");
    let image = native::capture_owned(hwnd, process, path, true)?;
    Ok(
        json!({"window":window,"capture":path,"width":image.width,"height":image.height,
        "completePopupCapture":true,"productionTooltipVerified":false}),
    )
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
    thread::sleep(Duration::from_millis(1000));
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
    let tooltip_path = path.with_file_name(format!(
        "{}-tooltip.png",
        path.file_stem()
            .and_then(|name| name.to_str())
            .ok_or("hover capture has no file stem")?
    ));
    let tooltip = capture_tooltip(handle, process, before, &tooltip_path);
    let after = hover_readback(handle, POINT { x, y })?;
    Ok(
        json!({"screenPoint":[x,y],"actualTargetHwnd":target as usize,"actualTargetPid":target_process,
        "cursorBeforeCapture":[before.x,before.y],"cursorAfterCapture":[after.x,after.y],"foregroundOwned":true,
        "width":image.width,"height":image.height,"capture":path,"tooltip":tooltip}),
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
