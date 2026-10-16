use crate::desktop::{WindowCaptionButton, WindowCaptionButtonKind, WindowCaptionState};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        GetMonitorInfoW, GetSysColor, MonitorFromWindow, ScreenToClient, COLOR_WINDOW, MONITORINFO,
        MONITOR_DEFAULTTONEAREST,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::WM_MOUSELEAVE,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            GetFocus, SetFocus, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
        },
        Shell::{
            DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
            SUBCLASSPROC,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetClientRect, GetCursorPos, GetParent,
            GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect,
            GetWindowThreadProcessId, IsChild, IsWindow, IsZoomed, PostMessageW, SendMessageW,
            SetWindowPos, SetWindowTextW, ShowWindow, SystemParametersInfoW, BM_GETSTATE,
            BST_PUSHED, BS_NOTIFY, BS_OWNERDRAW, GWL_STYLE, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
            HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
            HWND_TOP, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME, SPI_GETHIGHCONTRAST,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
            SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CANCELMODE, WM_CAPTURECHANGED, WM_CLOSE,
            WM_COMMAND, WM_DPICHANGED, WM_DRAWITEM, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
            WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCMOUSELEAVE,
            WM_NCMOUSEMOVE, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE, WM_THEMECHANGED,
            WS_CHILD, WS_VISIBLE,
        },
    },
};

pub(super) type CaptionObserver = Box<dyn Fn(Option<WindowCaptionState>) + Send>;
const ROOT_ID: usize = 0x5441_4341;
const WEBVIEW_ID: usize = 0x5441_4345;
const IDS: [usize; 3] = [0x5441_4342, 0x5441_4343, 0x5441_4344];
thread_local! { static OWNED: RefCell<HashMap<isize, Rc<State>>> = RefCell::new(HashMap::new()); }
struct State {
    root: HWND,
    webview: HWND,
    buttons: [HWND; 3],
    observer: CaptionObserver,
    closing: Cell<bool>,
    updating: Cell<bool>,
    maximized: Cell<bool>,
    fullscreen: Cell<bool>,
    scale: Cell<f64>,
    rects: RefCell<[RECT; 3]>,
    hovered: Cell<Option<usize>>,
    pressed: Cell<Option<usize>>,
    dying_buttons: Cell<u8>,
    hooked_buttons: Cell<u8>,
    relaying: Cell<bool>,
    hover_color: RefCell<String>,
    saved_focus: Cell<HWND>,
}
#[derive(Default)]
struct Dispatch {
    base_called: bool,
    base_result: LRESULT,
    handled: bool,
    handled_result: LRESULT,
}
impl Dispatch {
    fn handled(&mut self, result: LRESULT) {
        self.handled = true;
        self.handled_result = result;
    }
}
unsafe fn def(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM, dispatch: &mut Dispatch) -> LRESULT {
    let result = DefSubclassProc(hwnd, msg, w, l);
    dispatch.base_called = true;
    dispatch.base_result = result;
    result
}
unsafe fn panic_fallback(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    dispatch: Dispatch,
) -> LRESULT {
    if dispatch.base_called {
        dispatch.base_result
    } else if dispatch.handled {
        dispatch.handled_result
    } else {
        DefSubclassProc(hwnd, msg, w, l)
    }
}
unsafe fn live(hwnd: HWND) -> bool {
    if hwnd.is_null() || IsWindow(hwnd) == 0 {
        return false;
    }
    let mut pid = 0;
    let tid = GetWindowThreadProcessId(hwnd, &mut pid);
    tid != 0 && tid == GetCurrentThreadId() && pid == std::process::id()
}
fn owns_hwnd(state: &State, hwnd: HWND, id: usize) -> bool {
    if id == ROOT_ID {
        state.root == hwnd
    } else if id == WEBVIEW_ID {
        state.webview == hwnd
    } else {
        IDS.iter()
            .enumerate()
            .any(|(i, button_id)| *button_id == id && state.buttons[i] == hwnd)
    }
}
fn registered(hwnd: HWND, id: usize, p: usize) -> bool {
    OWNED.with(|map| {
        map.borrow()
            .values()
            .any(|state| Rc::as_ptr(state) as usize == p && owns_hwnd(state, hwnd, id))
    })
}
fn registered_state(hwnd: HWND, id: usize, p: usize) -> Option<Rc<State>> {
    OWNED.with(|map| {
        map.borrow()
            .values()
            .find(|state| Rc::as_ptr(state) as usize == p && owns_hwnd(state, hwnd, id))
            .cloned()
    })
}
unsafe fn own(hwnd: HWND, proc: SUBCLASSPROC, id: usize) -> Option<Rc<State>> {
    if !live(hwnd) {
        return None;
    }
    let mut p = 0usize;
    if GetWindowSubclass(hwnd, proc, id, &mut p) == 0 || p == 0 {
        return None;
    }
    // Registry is the sole long-lived owner. Treat subclass refdata as an
    // opaque key and only clone the matching registry entry.
    registered_state(hwnd, id, p).filter(|state| !state.closing.get())
}
fn snap(s: &State) -> Option<WindowCaptionState> {
    if s.closing.get() || s.fullscreen.get() {
        return None;
    }
    Some(WindowCaptionState {
        scale_factor: s.scale.get(),
        maximized: s.maximized.get(),
        hover_color: s.hover_color.borrow().clone(),
        buttons: s
            .rects
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, r)| WindowCaptionButton {
                kind: [
                    WindowCaptionButtonKind::Minimize,
                    WindowCaptionButtonKind::Maximize,
                    WindowCaptionButtonKind::Close,
                ][i],
                x: r.left,
                y: r.top,
                width: (r.right - r.left).max(0) as u32,
                height: (r.bottom - r.top).max(0) as u32,
                hovered: s.hovered.get() == Some(i),
                pressed: s.pressed.get() == Some(i),
            })
            .collect(),
    })
}
fn emit(s: &State) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (s.observer)(snap(s)))).is_err() {
        eprintln!("Windows caption observer panicked");
    }
}
fn emit_if_changed(s: &State, before: Option<WindowCaptionState>) {
    if before != snap(s) {
        emit(s);
    }
}
unsafe fn apps_hover() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let k: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let n: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut v = 1u32;
    let mut z = 4u32;
    let mut hc: HIGHCONTRASTW = std::mem::zeroed();
    hc.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
    if SystemParametersInfoW(
        SPI_GETHIGHCONTRAST,
        hc.cbSize,
        (&mut hc as *mut HIGHCONTRASTW).cast(),
        0,
    ) != 0
        && hc.dwFlags & HCF_HIGHCONTRASTON != 0
    {
        let color = GetSysColor(COLOR_WINDOW);
        let r = (color & 255) as f64;
        let g = ((color >> 8) & 255) as f64;
        let b = ((color >> 16) & 255) as f64;
        return if 0.25 * r + 0.625 * g + 0.125 * b <= 128.0 {
            "#ffffff".into()
        } else {
            "#000000".into()
        };
    }
    if RegGetValueW(
        HKEY_CURRENT_USER,
        k.as_ptr(),
        n.as_ptr(),
        RRF_RT_REG_DWORD,
        std::ptr::null_mut(),
        (&mut v as *mut u32).cast(),
        &mut z,
    ) == 0
        && v == 0
    {
        // Chromium maps AppsUseLightTheme=0 to dark UI; native non-close hover
        // uses a white translucent overlay in dark mode.
        "#ffffff".into()
    } else {
        "#000000".into()
    }
}
unsafe fn refresh(s: &State, notify: bool) -> Result<(), String> {
    if s.closing.get() || !live(s.root) {
        return Err("caption root is no longer live on its UI thread".into());
    }
    if s.updating.replace(true) {
        return Ok(());
    }
    let before = snap(s);
    let max = IsZoomed(s.root) != 0;
    let scale = GetDpiForWindow(s.root).max(96) as f64 / 96.0;
    let mut c: RECT = std::mem::zeroed();
    if GetClientRect(s.root, &mut c) == 0 {
        s.updating.set(false);
        return Err("GetClientRect failed".into());
    }
    let widths = [45., 46., 46.];
    let right_gap = if max { 0. } else { 1. };
    let top = if max { 0. } else { 1. };
    let h = if max { 32. } else { 31. };
    let mut x = c.right - (right_gap * scale).round() as i32;
    let mut rs = [std::mem::zeroed(); 3];
    for i in (0..3).rev() {
        let w = (widths[i] * scale).round() as i32;
        x -= w;
        rs[i] = RECT {
            left: x,
            top: (top * scale).round() as i32,
            right: x + w,
            bottom: ((top + h) * scale).round() as i32,
        };
    }
    let old_rects = *s.rects.borrow();
    for (i, r) in rs.iter().enumerate() {
        if SetWindowPos(
            s.buttons[i],
            HWND_TOP,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        ) == 0
        {
            for (j, old) in old_rects.iter().enumerate() {
                if SetWindowPos(
                    s.buttons[j],
                    HWND_TOP,
                    old.left,
                    old.top,
                    old.right - old.left,
                    old.bottom - old.top,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                ) == 0
                {
                    eprintln!("caption button geometry rollback failed for button {j}");
                }
            }
            s.updating.set(false);
            return Err(format!("SetWindowPos failed for caption button {i}"));
        }
    }
    if max != s.maximized.get() {
        let text: Vec<u16> = if max {
            "Restore\0".encode_utf16().collect()
        } else {
            "Maximize\0".encode_utf16().collect()
        };
        if SetWindowTextW(s.buttons[1], text.as_ptr()) == 0 {
            for (j, old) in old_rects.iter().enumerate() {
                if SetWindowPos(
                    s.buttons[j],
                    HWND_TOP,
                    old.left,
                    old.top,
                    old.right - old.left,
                    old.bottom - old.top,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                ) == 0
                {
                    eprintln!("caption button geometry rollback failed for button {j}");
                }
            }
            let old_text: Vec<u16> = if s.maximized.get() {
                "Restore\0".encode_utf16().collect()
            } else {
                "Maximize\0".encode_utf16().collect()
            };
            if SetWindowTextW(s.buttons[1], old_text.as_ptr()) == 0 {
                eprintln!("caption maximize accessible-name rollback failed");
            }
            s.updating.set(false);
            return Err("SetWindowTextW failed for caption maximize accessible name".into());
        }
    }
    s.rects.replace(rs);
    s.maximized.set(max);
    s.scale.set(scale);
    s.fullscreen
        .set((GetWindowLongPtrW(s.root, GWL_STYLE) as u32 & 0x00C00000) == 0);
    for b in s.buttons {
        ShowWindow(b, if s.fullscreen.get() { 0 } else { 5 });
    }
    s.updating.set(false);
    if notify && before != snap(s) {
        emit(s)
    }
    Ok(())
}
unsafe fn resize_hit(s: &State, l: LPARAM) -> Option<u32> {
    if IsZoomed(s.root) != 0 || s.fullscreen.get() {
        return None;
    }
    let mut bounds: RECT = std::mem::zeroed();
    if GetWindowRect(s.root, &mut bounds) == 0 {
        return None;
    }
    let scale = GetDpiForWindow(s.root).max(96) as f64 / 96.0;
    let bx = ((GetSystemMetrics(SM_CXSIZEFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER)) as f64
        * scale)
        .round() as i32;
    let by = ((GetSystemMetrics(SM_CYSIZEFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER)) as f64
        * scale)
        .round() as i32;
    let x = l as u16 as i16 as i32;
    let y = (l >> 16) as u16 as i16 as i32;
    let left = x < bounds.left + bx;
    let right = x >= bounds.right - bx;
    let top = y < bounds.top + by;
    let bottom = y >= bounds.bottom - by;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(HTTOPLEFT),
        (_, true, true, _) => Some(HTTOPRIGHT),
        (true, _, _, true) => Some(HTBOTTOMLEFT),
        (_, true, _, true) => Some(HTBOTTOMRIGHT),
        (true, _, _, _) => Some(HTLEFT),
        (_, true, _, _) => Some(HTRIGHT),
        (_, _, true, _) => Some(HTTOP),
        (_, _, _, true) => Some(HTBOTTOM),
        _ => None,
    }
}
unsafe fn button_proc_impl(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
    dispatch: &mut Dispatch,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let state = registered_state(hwnd, id, data);
        if let Some(state) = state.as_ref() {
            if let Some(index) = IDS.iter().position(|button_id| *button_id == id) {
                if state.buttons[index] == hwnd
                    && GetParent(hwnd) == state.root
                    && !state.closing.get()
                    && state.dying_buttons.get() & (1 << index) == 0
                {
                    state
                        .dying_buttons
                        .set(state.dying_buttons.get() | (1 << index));
                    if state.hovered.get() == Some(index) {
                        state.hovered.set(None);
                    }
                    if state.pressed.get() == Some(index) {
                        state.pressed.set(None);
                    }
                    // Remove this binding before the window can finish
                    // destruction and its handle can be recycled.
                    if RemoveWindowSubclass(hwnd, Some(button_proc), id) == 0 {
                        eprintln!("could not remove destroyed caption button subclass");
                    }
                    if let Err(error) = remove(state.root) {
                        eprintln!(
                            "could not detach caption adapter after button destruction: {error}"
                        );
                    }
                    return def(hwnd, msg, w, l, dispatch);
                }
            }
        }
        let result = def(hwnd, msg, w, l, dispatch);
        for button_id in IDS {
            let mut p = 0;
            if GetWindowSubclass(hwnd, Some(button_proc), button_id, &mut p) != 0
                && p == data
                && registered(hwnd, button_id, p)
            {
                if RemoveWindowSubclass(hwnd, Some(button_proc), button_id) == 0 {
                    eprintln!("could not remove caption button subclass during destruction");
                }
                break;
            }
        }
        return result;
    }
    let mut found = None;
    for (i, id) in IDS.iter().copied().enumerate() {
        if let Some(s) = own(hwnd, Some(button_proc), id) {
            if s.buttons[i] == hwnd {
                found = Some((s, i));
                break;
            }
        }
    }
    if let Some((s, i)) = found {
        let before = snap(&s);
        if msg == WM_LBUTTONDOWN {
            s.saved_focus.set(GetFocus());
        }
        if msg == WM_MOUSEMOVE {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            if TrackMouseEvent(&mut tracking) == 0 {
                eprintln!("could not track caption button mouse leave");
            }
        }
        let ret = def(hwnd, msg, w, l, dispatch);
        let mut pointer_inside = false;
        if matches!(
            msg,
            WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_MOUSEMOVE
                | WM_MOUSELEAVE
                | WM_CANCELMODE
                | WM_CAPTURECHANGED
        ) {
            let pushed =
                live(hwnd) && (SendMessageW(hwnd, BM_GETSTATE, 0, 0) as u32 & BST_PUSHED) != 0;
            let inside = if msg == WM_MOUSELEAVE {
                false
            } else if matches!(msg, WM_CANCELMODE | WM_CAPTURECHANGED) {
                let mut point = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 {
                    false
                } else {
                    let mut r: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut r) != 0
                        && point.x >= r.left
                        && point.x < r.right
                        && point.y >= r.top
                        && point.y < r.bottom
                }
            } else {
                let x = l as u16 as i16 as i32;
                let y = (l >> 16) as u16 as i16 as i32;
                let mut r: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut r) != 0
                    && x >= r.left
                    && x < r.right
                    && y >= r.top
                    && y < r.bottom
            };
            pointer_inside = inside;
            s.pressed.set(if pushed && inside { Some(i) } else { None });
            if inside {
                s.hovered.set(Some(i));
            } else if s.hovered.get() == Some(i) {
                s.hovered.set(None);
            }
            emit_if_changed(&s, before);
        }
        if msg == WM_MOUSEMOVE && pointer_inside {
            if !s.relaying.replace(true) {
                let mut point = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut point) != 0 {
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: s.root,
                        dwHoverTime: 0,
                    };
                    if TrackMouseEvent(&mut tracking) == 0 {
                        eprintln!("could not track caption root non-client mouse leave");
                    }
                    let packed = ((point.y as u16 as isize) << 16) | (point.x as u16 as isize);
                    SendMessageW(
                        s.root,
                        WM_NCMOUSEMOVE,
                        [HTMINBUTTON, HTMAXBUTTON, HTCLOSE][i] as usize,
                        packed,
                    );
                }
                s.relaying.set(false);
            }
        }
        return ret;
    }
    def(hwnd, msg, w, l, dispatch)
}
unsafe extern "system" fn button_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let mut dispatch = Dispatch::default();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        button_proc_impl(hwnd, msg, w, l, id, data, &mut dispatch)
    })) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("Windows caption button callback panicked for message {msg:#x}");
            unsafe { panic_fallback(hwnd, msg, w, l, dispatch) }
        }
    }
}
unsafe fn root_proc_impl(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
    dispatch: &mut Dispatch,
) -> LRESULT {
    let Some(s) = own(hwnd, Some(root_proc), ROOT_ID) else {
        return def(hwnd, msg, w, l, dispatch);
    };
    if msg == WM_NCDESTROY {
        s.closing.set(true);
        let ret = def(hwnd, msg, w, l, dispatch);
        let mut p = 0;
        if GetWindowSubclass(hwnd, Some(root_proc), ROOT_ID, &mut p) != 0 && p == data {
            if RemoveWindowSubclass(hwnd, Some(root_proc), ROOT_ID) == 0 {
                eprintln!("could not remove caption root subclass during destruction");
            }
        }
        for (i, id) in IDS.iter().copied().enumerate() {
            let b = s.buttons[i];
            let mut q = 0;
            if live(b)
                && GetParent(b) == hwnd
                && GetWindowSubclass(b, Some(button_proc), id, &mut q) != 0
                && q == data
                && registered(b, id, q)
            {
                if RemoveWindowSubclass(b, Some(button_proc), id) != 0 {
                    DestroyWindow(b);
                } else {
                    eprintln!("could not remove caption button subclass during root destruction");
                }
            }
        }
        let mut child_data = 0;
        if live(s.webview)
            && GetWindowSubclass(s.webview, Some(webview_proc), WEBVIEW_ID, &mut child_data) != 0
            && child_data == data
            && registered(s.webview, WEBVIEW_ID, child_data)
            && RemoveWindowSubclass(s.webview, Some(webview_proc), WEBVIEW_ID) == 0
        {
            eprintln!("could not remove Wry child subclass during root destruction");
        }
        OWNED.with(|m| {
            m.borrow_mut().remove(&(hwnd as isize));
        });
        return ret;
    }
    if msg == WM_NCCALCSIZE && l != 0 {
        // The first proposed RECT is the outer window rect in both the
        // NCCALCSIZE_PARAMS and FALSE forms.
        let original = *(l as *const RECT);
        let ret = def(hwnd, msg, w, l, dispatch);
        let p = l as *mut RECT;
        if IsZoomed(hwnd) != 0 {
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut mi) != 0 {
                *p = mi.rcWork
            } else {
                eprintln!("GetMonitorInfoW failed while applying maximized caption bounds");
            }
        } else {
            *p = original
        }
        return ret;
    }
    if msg == WM_DRAWITEM && l != 0 {
        let d = &*(l as *const windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT);
        if s.buttons.contains(&d.hwndItem) {
            dispatch.handled(1);
            return 1;
        }
    }
    if msg == WM_COMMAND && ((w >> 16) & 0xffff) == 0 {
        let child = l as HWND;
        if let Some(i) = s.buttons.iter().position(|b| *b == child) {
            if live(child) && GetParent(child) == hwnd {
                dispatch.handled(0);
                let saved = s.saved_focus.get();
                let f = if saved.is_null() { GetFocus() } else { saved };
                match i {
                    0 => {
                        ShowWindow(hwnd, SW_MINIMIZE);
                    }
                    1 => {
                        ShowWindow(
                            hwnd,
                            if IsZoomed(hwnd) != 0 {
                                SW_RESTORE
                            } else {
                                SW_MAXIMIZE
                            },
                        );
                    }
                    2 => {
                        if PostMessageW(hwnd, WM_CLOSE, 0, 0) == 0 {
                            eprintln!("could not post WM_CLOSE for caption close button");
                        }
                    }
                    _ => {}
                }
                if live(f) && (f == s.webview || IsChild(s.webview, f) != 0) {
                    SetFocus(f);
                }
                return 0;
            }
        }
    }
    if msg == WM_NCHITTEST && !s.fullscreen.get() {
        let mut p = POINT {
            x: l as u16 as i16 as i32,
            y: (l >> 16) as u16 as i16 as i32,
        };
        if ScreenToClient(hwnd, &mut p) != 0 {
            for (i, r) in s.rects.borrow().iter().enumerate() {
                if p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom {
                    let result = [HTMINBUTTON, HTMAXBUTTON, HTCLOSE][i] as LRESULT;
                    dispatch.handled(result);
                    return result;
                }
            }
        }
        // The WebView child receives hits over the full-client surface, so
        // return native resize zones here as well as on the root HWND.
        if let Some(hit) = resize_hit(&s, l) {
            dispatch.handled(hit as LRESULT);
            return hit as LRESULT;
        }
    }
    if msg == WM_NCLBUTTONDOWN {
        if let Some(i) = match w as u32 {
            HTMINBUTTON => Some(0),
            HTMAXBUTTON => Some(1),
            HTCLOSE => Some(2),
            _ => None,
        } {
            dispatch.handled(0);
            s.saved_focus.set(GetFocus());
            let mut p = POINT {
                x: l as u16 as i16 as i32,
                y: (l >> 16) as u16 as i16 as i32,
            };
            ScreenToClient(s.buttons[i], &mut p);
            let lp = ((p.y as u16 as isize) << 16) | (p.x as u16 as isize);
            SendMessageW(s.buttons[i], WM_LBUTTONDOWN, 1, lp);
            return 0;
        }
    }
    if msg == WM_NCMOUSEMOVE {
        if let Some(i) = match w as u32 {
            HTMINBUTTON => Some(0),
            HTMAXBUTTON => Some(1),
            HTCLOSE => Some(2),
            _ => None,
        } {
            if s.hovered.replace(Some(i)) != Some(i) {
                emit(&s)
            }
        }
    }
    if msg == WM_NCMOUSELEAVE && s.hovered.get().is_some() {
        s.hovered.set(None);
        emit(&s);
    }
    if matches!(
        msg,
        WM_SIZE | WM_DPICHANGED | WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE
    ) {
        if matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE) {
            s.hover_color.replace(apps_hover());
        }
        if let Err(error) = refresh(&s, true) {
            eprintln!("Windows caption refresh failed: {error}");
        }
    }
    def(hwnd, msg, w, l, dispatch)
}

unsafe extern "system" fn root_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let mut dispatch = Dispatch::default();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        root_proc_impl(hwnd, msg, w, l, id, data, &mut dispatch)
    })) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("Windows caption root callback panicked for message {msg:#x}");
            unsafe { panic_fallback(hwnd, msg, w, l, dispatch) }
        }
    }
}

unsafe fn webview_proc_impl(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
    dispatch: &mut Dispatch,
) -> LRESULT {
    if msg == WM_NCHITTEST {
        if let Some(state) = own(hwnd, Some(webview_proc), WEBVIEW_ID) {
            if let Some(hit) = resize_hit(&state, l) {
                dispatch.handled(hit as LRESULT);
                return hit as LRESULT;
            }
        }
    }
    if msg == WM_NCLBUTTONDOWN {
        if matches!(
            w as u32,
            HTLEFT
                | HTRIGHT
                | HTTOP
                | HTBOTTOM
                | HTTOPLEFT
                | HTTOPRIGHT
                | HTBOTTOMLEFT
                | HTBOTTOMRIGHT
        ) {
            if let Some(state) = own(hwnd, Some(webview_proc), WEBVIEW_ID) {
                dispatch.handled(0);
                SendMessageW(state.root, WM_NCLBUTTONDOWN, w, l);
                return 0;
            }
        }
    }
    if msg != WM_NCDESTROY {
        return def(hwnd, msg, w, l, dispatch);
    }
    // Hold this hook's registry-owned state through the reentrant destruction path.
    let state = own(hwnd, Some(webview_proc), WEBVIEW_ID);
    let result = def(hwnd, msg, w, l, dispatch);
    let mut raw = 0;
    if GetWindowSubclass(hwnd, Some(webview_proc), WEBVIEW_ID, &mut raw) != 0
        && raw == data
        && registered(hwnd, WEBVIEW_ID, raw)
    {
        if RemoveWindowSubclass(hwnd, Some(webview_proc), WEBVIEW_ID) == 0 {
            eprintln!("could not remove Wry child subclass during destruction");
        }
    }
    if let Some(state) = state {
        let _ = remove(state.root);
    }
    result
}
unsafe extern "system" fn webview_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let mut dispatch = Dispatch::default();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        webview_proc_impl(hwnd, msg, w, l, id, data, &mut dispatch)
    })) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("Windows caption Wry-child callback panicked for message {msg:#x}");
            unsafe { panic_fallback(hwnd, msg, w, l, dispatch) }
        }
    }
}
unsafe fn make_button(root: HWND, label: &str) -> Result<HWND, String> {
    let m = GetModuleHandleW(std::ptr::null());
    if m.is_null() {
        return Err("GetModuleHandleW failed".into());
    }
    let class: Vec<u16> = "BUTTON\0".encode_utf16().collect();
    let text: Vec<u16> = format!("{label}\0").encode_utf16().collect();
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        text.as_ptr(),
        WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32 | BS_NOTIFY as u32,
        0,
        0,
        1,
        1,
        root,
        std::ptr::null_mut(),
        m,
        std::ptr::null(),
    );
    if h.is_null() {
        Err(format!("could not create {label} caption button"))
    } else {
        Ok(h)
    }
}
pub(super) fn install(hwnd: HWND, observer: CaptionObserver) -> Result<(), String> {
    unsafe {
        if !live(hwnd) {
            return Err("caption root is not live on this process UI thread".into());
        }
        if let Some(existing) = OWNED.with(|m| m.borrow().get(&(hwnd as isize)).cloned()) {
            // Reapplying the same frame updates the existing adapter; it never
            // creates duplicate buttons or subclass ownership references.
            refresh(&existing, true)?;
            return Ok(());
        }
        let web = GetWindow(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::GW_CHILD);
        if web.is_null() || !live(web) || GetParent(web) != hwnd {
            return Err("owned Wry child is unavailable".into());
        }
        let mut existing_data = 0;
        if GetWindowSubclass(hwnd, Some(root_proc), ROOT_ID, &mut existing_data) != 0
            || GetWindowSubclass(web, Some(webview_proc), WEBVIEW_ID, &mut existing_data) != 0
        {
            return Err(
                "caption adapter subclass binding already exists without owned state".into(),
            );
        }
        let mut bs = [std::ptr::null_mut(); 3];
        let initially_maximized = IsZoomed(hwnd) != 0;
        let labels = [
            "Minimize",
            if initially_maximized {
                "Restore"
            } else {
                "Maximize"
            },
            "Close",
        ];
        for i in 0..3 {
            match make_button(hwnd, labels[i]) {
                Ok(b) => bs[i] = b,
                Err(e) => {
                    for b in bs {
                        if !b.is_null() {
                            DestroyWindow(b);
                        }
                    }
                    return Err(e);
                }
            }
        }
        let s = Rc::new(State {
            root: hwnd,
            webview: web,
            buttons: bs,
            observer,
            closing: Cell::new(false),
            updating: Cell::new(false),
            maximized: Cell::new(IsZoomed(hwnd) != 0),
            fullscreen: Cell::new(false),
            scale: Cell::new(GetDpiForWindow(hwnd).max(96) as f64 / 96.),
            rects: RefCell::new([std::mem::zeroed(); 3]),
            hovered: Cell::new(None),
            pressed: Cell::new(None),
            dying_buttons: Cell::new(0),
            hooked_buttons: Cell::new(0),
            relaying: Cell::new(false),
            hover_color: RefCell::new(apps_hover()),
            saved_focus: Cell::new(std::ptr::null_mut()),
        });
        // Publish the owning Rc before installing hooks so every later
        // installation failure can use the same complete rollback path.
        OWNED.with(|m| {
            m.borrow_mut().insert(hwnd as isize, Rc::clone(&s));
        });
        let p = Rc::as_ptr(&s) as usize;
        if SetWindowSubclass(hwnd, Some(root_proc), ROOT_ID, p) == 0 {
            let _ = remove(hwnd);
            return Err("root subclass install failed".into());
        }
        for (i, b) in bs.iter().copied().enumerate() {
            if SetWindowSubclass(b, Some(button_proc), IDS[i], p) == 0 {
                let _ = remove(hwnd);
                return Err("button subclass install failed; owned hooks rolled back".into());
            }
            s.hooked_buttons.set(s.hooked_buttons.get() | (1 << i));
        }
        if SetWindowSubclass(web, Some(webview_proc), WEBVIEW_ID, p) == 0 {
            let _ = remove(hwnd);
            return Err(
                "owned Wry child subclass installation failed; own hooks rolled back".into(),
            );
        }
        if let Err(e) = refresh(&s, false) {
            let _ = remove(hwnd);
            return Err(e);
        }
        // Recalculate the non-client area after every hook is live. The
        // no-move/no-size flags retain the caller's exact outer bounds.
        if SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
        ) == 0
        {
            let _ = remove(hwnd);
            return Err("SetWindowPos failed while applying the client-area caption frame".into());
        }
        if let Err(e) = refresh(&s, false) {
            let _ = remove(hwnd);
            return Err(e);
        }
        emit(&s);
        Ok(())
    }
}
pub(super) fn remove(hwnd: HWND) -> Result<(), String> {
    unsafe {
        if !live(hwnd) {
            return Err("caption root is not live on this process UI thread".into());
        }
        let s = OWNED.with(|m| m.borrow().get(&(hwnd as isize)).cloned());
        let Some(s) = s else { return Ok(()) };
        if s.closing.replace(true) {
            return Ok(());
        }
        let expected = Rc::as_ptr(&s) as usize;
        let mut active = 0;
        if GetWindowSubclass(hwnd, Some(root_proc), ROOT_ID, &mut active) != 0 && active != expected
        {
            eprintln!("caption root subclass binding changed; leaving foreign binding untouched");
        }
        for (i, id) in IDS.iter().copied().enumerate() {
            if s.dying_buttons.get() & (1 << i) != 0 {
                continue;
            }
            if live(s.buttons[i])
                && GetWindowSubclass(s.buttons[i], Some(button_proc), id, &mut active) != 0
                && active != expected
            {
                eprintln!("caption button {i} subclass binding changed; leaving it untouched");
            }
        }
        if live(s.webview)
            && GetWindowSubclass(s.webview, Some(webview_proc), WEBVIEW_ID, &mut active) != 0
            && active != expected
        {
            eprintln!("Wry child subclass binding changed; leaving foreign binding untouched");
        }
        let mut p = 0;
        if GetWindowSubclass(hwnd, Some(root_proc), ROOT_ID, &mut p) != 0
            && p == expected
            && registered(hwnd, ROOT_ID, p)
            && RemoveWindowSubclass(hwnd, Some(root_proc), ROOT_ID) == 0
        {
            eprintln!("could not remove caption root subclass");
        }
        for (i, id) in IDS.iter().copied().enumerate() {
            let b = s.buttons[i];
            if s.dying_buttons.get() & (1 << i) != 0 {
                continue;
            }
            let mut q = 0;
            let parent_is_root = live(b) && GetParent(b) == hwnd;
            let has_own_hook = parent_is_root
                && GetWindowSubclass(b, Some(button_proc), id, &mut q) != 0
                && q == expected
                && registered(b, id, q);
            let created_but_unhooked = parent_is_root
                && s.hooked_buttons.get() & (1 << i) == 0
                && GetWindowSubclass(b, Some(button_proc), id, &mut q) == 0;
            if has_own_hook {
                if RemoveWindowSubclass(b, Some(button_proc), id) != 0 {
                    DestroyWindow(b);
                } else {
                    eprintln!("could not remove caption button subclass {i}");
                }
            } else if created_but_unhooked {
                // This can only be a child created by this install attempt
                // before its subclass was successfully attached.
                DestroyWindow(b);
            }
        }
        if live(s.webview) {
            let mut child_data = 0;
            if GetWindowSubclass(s.webview, Some(webview_proc), WEBVIEW_ID, &mut child_data) != 0
                && child_data == expected
                && registered(s.webview, WEBVIEW_ID, child_data)
                && RemoveWindowSubclass(s.webview, Some(webview_proc), WEBVIEW_ID) == 0
            {
                eprintln!("could not remove Wry child subclass");
            }
        }
        OWNED.with(|m| {
            m.borrow_mut().remove(&(hwnd as isize));
        });
        // Clear the browser-rendered caption as soon as native removal has
        // detached the adapter, even if restoring the OS frame later fails.
        emit(&s);
        if SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
        ) == 0
        {
            return Err("SetWindowPos failed while restoring the native frame".into());
        }
        Ok(())
    }
}
pub(super) fn snapshot(hwnd: HWND) -> Result<Option<WindowCaptionState>, String> {
    unsafe {
        if !live(hwnd) {
            return Err("caption root is not live on this process UI thread".into());
        }
        let s = OWNED.with(|m| m.borrow().get(&(hwnd as isize)).cloned());
        let Some(s) = s else { return Ok(None) };
        refresh(&s, false)?;
        Ok(snap(&s))
    }
}
