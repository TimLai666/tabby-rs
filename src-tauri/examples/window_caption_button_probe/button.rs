// Disposable experiment slot. No product code includes this file.
use std::{cell::Cell, sync::Arc};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Shell::{
            DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
            SUBCLASSPROC,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetClientRect, GetParent, GetWindowThreadProcessId,
            IsWindow, SendMessageW, SetWindowPos, BS_OWNERDRAW, HTMAXBUTTON, HTTRANSPARENT,
            HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW, WM_DRAWITEM, WM_ERASEBKGND, WM_NCDESTROY,
            WM_NCHITTEST, WM_NCMOUSEMOVE, WS_CHILD, WS_EX_NOACTIVATE, WS_VISIBLE,
        },
    },
};

const CHILD_ID: usize = 0x5441_4241;
const CONTAINER_ID: usize = 0x5441_4242;
const ROOT_ID: usize = 0x5441_4243;
struct State {
    parent: HWND,
    container: HWND,
    child: Cell<HWND>,
    rect: RECT,
    closing: Cell<bool>,
    relaying: Cell<bool>,
    symbol_color: Cell<u32>,
}

fn inside(state: &State, lparam: LPARAM) -> bool {
    let mut point = POINT {
        x: lparam as u16 as i16 as i32,
        y: (lparam >> 16) as u16 as i16 as i32,
    };
    unsafe {
        ScreenToClient(state.parent, &mut point) != 0
            && point.x >= state.rect.left
            && point.x < state.rect.right
            && point.y >= state.rect.top
            && point.y < state.rect.bottom
    }
}

// B slot: relay only an actual child non-client mouse move to its fixed owned root.
// Preserve packed screen coordinates, retain state across reentrant destruction,
// and return Some(0) after handling. Other messages return None. Never synthesize
// hover from hit-test, setup, timers or observers. No mouse-down belongs here.
unsafe fn relay_child_input(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    data: usize,
) -> Option<LRESULT> {
    if message != WM_NCMOUSEMOVE || wparam != HTMAXBUTTON as usize || data == 0 {
        return None;
    }
    let state_ptr = data as *const State;
    Arc::increment_strong_count(state_ptr);
    let state = Arc::from_raw(state_ptr);
    if state.closing.get()
        || hwnd != state.child.get()
        || state.parent.is_null()
        || IsWindow(hwnd) == 0
        || IsWindow(state.parent) == 0
        || GetParent(hwnd) != state.parent
    {
        return None;
    }
    let mut child_pid = 0;
    let mut parent_pid = 0;
    let child_tid = GetWindowThreadProcessId(hwnd, &mut child_pid);
    let parent_tid = GetWindowThreadProcessId(state.parent, &mut parent_pid);
    let current_tid = GetCurrentThreadId();
    if child_pid != std::process::id()
        || parent_pid != child_pid
        || child_tid == 0
        || child_tid != parent_tid
        || child_tid != current_tid
    {
        return None;
    }
    let mut active_child_data = 0;
    let mut active_root_data = 0;
    if GetWindowSubclass(hwnd, Some(child_proc), CHILD_ID, &mut active_child_data) == 0
        || active_child_data != data
        || GetWindowSubclass(
            state.parent,
            Some(root_proc),
            ROOT_ID,
            &mut active_root_data,
        ) == 0
        || active_root_data != data
        || !inside(&state, lparam)
    {
        return None;
    }
    if state.relaying.replace(true) {
        return Some(0);
    }
    SendMessageW(state.parent, message, wparam, lparam);
    state.relaying.set(false);
    Some(0)
}

unsafe extern "system" fn child_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    if let Some(result) = relay_child_input(hwnd, msg, w, l, data) {
        return result;
    }
    if msg == WM_NCDESTROY {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(child_proc), CHILD_ID, &mut data) != 0 {
            let state = &*(data as *const State);
            if !state.closing.replace(true) {
                detach(state.parent, Some(root_proc), ROOT_ID);
                detach(state.container, Some(container_proc), CONTAINER_ID);
            }
            let removed = RemoveWindowSubclass(hwnd, Some(child_proc), CHILD_ID);
            let result = DefSubclassProc(hwnd, msg, w, l);
            if removed != 0 {
                drop(Arc::from_raw(data as *const State));
            }
            return result;
        }
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    if msg == WM_NCHITTEST {
        return HTMAXBUTTON as LRESULT;
    }
    DefSubclassProc(hwnd, msg, w, l)
}

unsafe extern "system" fn container_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let state = &*(data as *const State);
    if msg == WM_NCDESTROY {
        let removed = RemoveWindowSubclass(hwnd, Some(container_proc), CONTAINER_ID);
        let result = DefSubclassProc(hwnd, msg, w, l);
        detach(state.parent, Some(root_proc), ROOT_ID);
        if !state.closing.replace(true) {
            detach(state.child.get(), Some(child_proc), CHILD_ID);
        }
        if removed != 0 {
            drop(Arc::from_raw(data as *const State));
        }
        return result;
    }
    if msg == WM_NCHITTEST && !state.closing.get() && inside(state, l) {
        return HTTRANSPARENT as LRESULT;
    }
    DefSubclassProc(hwnd, msg, w, l)
}

unsafe extern "system" fn root_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let state = &*(data as *const State);
    if msg == WM_DRAWITEM && l != 0 {
        let item = &*(l as *const DRAWITEMSTRUCT);
        if item.hwndItem == state.child.get() {
            paint_symbol(item.hDC, item.rcItem, state.symbol_color.get());
            return 1;
        }
    }
    if msg == WM_NCDESTROY {
        state.closing.set(true);
        detach(state.container, Some(container_proc), CONTAINER_ID);
        let child = state.child.get();
        if !child.is_null() {
            DestroyWindow(child);
        }
        let removed = RemoveWindowSubclass(hwnd, Some(root_proc), ROOT_ID);
        let result = DefSubclassProc(hwnd, msg, w, l);
        if removed != 0 {
            drop(Arc::from_raw(data as *const State));
        }
        return result;
    }
    if msg == WM_NCHITTEST && !state.closing.get() && inside(state, l) {
        return HTMAXBUTTON as LRESULT;
    }
    DefSubclassProc(hwnd, msg, w, l)
}

unsafe fn detach(hwnd: HWND, proc: SUBCLASSPROC, id: usize) {
    if hwnd.is_null() {
        return;
    }
    let mut data = 0;
    if GetWindowSubclass(hwnd, proc, id, &mut data) != 0
        && RemoveWindowSubclass(hwnd, proc, id) != 0
    {
        drop(Arc::from_raw(data as *const State));
    }
}

pub unsafe fn install(parent: HWND, container: HWND, rect: RECT) -> Result<HWND, String> {
    let fail = |s: &str| s.to_string();
    if parent.is_null() || container.is_null() || IsWindow(parent) == 0 || IsWindow(container) == 0
    {
        return Err(fail("parent/container is not a live window"));
    }
    let pid = std::process::id();
    let mut pp = 0;
    let mut cp = 0;
    let pt = GetWindowThreadProcessId(parent, &mut pp);
    let ct = GetWindowThreadProcessId(container, &mut cp);
    if pp != pid || cp != pid || pt == 0 || pt != ct || pt != GetCurrentThreadId() {
        return Err(fail(
            "parent/container must belong to this process and UI thread",
        ));
    }
    if GetParent(container) != parent {
        return Err(fail("container is not a direct child of parent"));
    }
    if GetWindowSubclass(parent, Some(root_proc), ROOT_ID, std::ptr::null_mut()) != 0
        || GetWindowSubclass(
            container,
            Some(container_proc),
            CONTAINER_ID,
            std::ptr::null_mut(),
        ) != 0
    {
        return Err(fail("caption overlay hooks are already installed"));
    }
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if GetClientRect(parent, &mut client) == 0
        || rect.left < client.left
        || rect.top < client.top
        || rect.right > client.right
        || rect.bottom > client.bottom
        || rect.left >= rect.right
        || rect.top >= rect.bottom
    {
        return Err(fail("rect is empty or outside parent client area"));
    }
    let module = GetModuleHandleW(std::ptr::null());
    if module.is_null() {
        return Err(fail("GetModuleHandleW failed"));
    }
    let child = CreateWindowExW(
        WS_EX_NOACTIVATE,
        "BUTTON\0".encode_utf16().collect::<Vec<_>>().as_ptr(),
        "Maximize\0".encode_utf16().collect::<Vec<_>>().as_ptr(),
        WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        parent,
        std::ptr::null_mut(),
        module,
        std::ptr::null(),
    );
    if child.is_null() {
        return Err(fail("CreateWindowExW failed"));
    }
    let state = Arc::new(State {
        parent,
        container,
        child: Cell::new(child),
        rect,
        closing: Cell::new(false),
        relaying: Cell::new(false),
        symbol_color: Cell::new(0x00ffffff),
    });
    let child_data = Arc::into_raw(Arc::clone(&state)) as usize;
    if SetWindowSubclass(child, Some(child_proc), CHILD_ID, child_data) == 0 {
        drop(Arc::from_raw(child_data as *const State));
        DestroyWindow(child);
        return Err(fail("overlay child subclass install failed"));
    }
    let container_data = Arc::into_raw(Arc::clone(&state)) as usize;
    if SetWindowSubclass(
        container,
        Some(container_proc),
        CONTAINER_ID,
        container_data,
    ) == 0
    {
        drop(Arc::from_raw(container_data as *const State));
        state.closing.set(true);
        DestroyWindow(child);
        return Err(fail("container subclass install failed"));
    }
    let root_data = Arc::into_raw(Arc::clone(&state)) as usize;
    if SetWindowSubclass(parent, Some(root_proc), ROOT_ID, root_data) == 0 {
        drop(Arc::from_raw(root_data as *const State));
        state.closing.set(true);
        detach(container, Some(container_proc), CONTAINER_ID);
        DestroyWindow(child);
        return Err(fail("root subclass install failed"));
    }
    if SetWindowPos(
        child,
        HWND_TOP,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    ) == 0
    {
        state.closing.set(true);
        detach(parent, Some(root_proc), ROOT_ID);
        detach(container, Some(container_proc), CONTAINER_ID);
        DestroyWindow(child);
        return Err(fail("SetWindowPos failed while placing overlay"));
    }
    drop(state);
    Ok(child)
}

// C drawing slot: paint one centered 10x10 outline with the supplied COLORREF.
// Transparent pixels must keep the underlying HTML compositor visible. The slot
// was initially empty; actual run38950004028 fails its independent white pixel check.
unsafe fn paint_symbol(dc: windows_sys::Win32::Graphics::Gdi::HDC, rect: RECT, color: u32) {
    eprintln!(
        "caption_button_probe draw entry: dc={dc:?} rect={},{},{},{} color={color:#x}",
        rect.left, rect.top, rect.right, rect.bottom
    );
    if dc.is_null() || rect.right - rect.left < 10 || rect.bottom - rect.top < 10 {
        return;
    }
    let left = (rect.left + rect.right) / 2 - 5;
    let top = (rect.top + rect.bottom) / 2 - 5;
    // Write only the outline. No brush, selected GDI object, background copy,
    // layered alpha or persistent DC is needed for this bounded feasibility slot.
    for offset in 0..10 {
        for (x, y) in [
            (left + offset, top),
            (left + offset, top + 9),
            (left, top + offset),
            (left + 9, top + offset),
        ] {
            if windows_sys::Win32::Graphics::Gdi::SetPixelV(dc, x, y, color) == 0 {
                eprintln!(
                    "caption_button_probe native glyph pixel failed: {}",
                    std::io::Error::last_os_error()
                );
                return;
            }
        }
    }
    eprintln!(
        "caption_button_probe draw readback: glyph={:#x} center={:#x}",
        windows_sys::Win32::Graphics::Gdi::GetPixel(dc, left, top),
        windows_sys::Win32::Graphics::Gdi::GetPixel(dc, left + 5, top + 5)
    );
}

pub unsafe fn set_symbol_color(child: HWND, color: u32) -> Result<(), String> {
    let mut data = 0;
    if GetWindowSubclass(child, Some(child_proc), CHILD_ID, &mut data) == 0 || data == 0 {
        return Err("owned button subclass is absent".into());
    }
    let state = &*(data as *const State);
    if state.closing.get()
        || state.child.get() != child
        || GetCurrentThreadId() != GetWindowThreadProcessId(child, std::ptr::null_mut())
    {
        return Err("button is closing or is on another UI thread".into());
    }
    state.symbol_color.set(color & 0x00ffffff);
    if InvalidateRect(child, std::ptr::null(), 0) == 0 || UpdateWindow(child) == 0 {
        return Err("button repaint request failed".into());
    }
    Ok(())
}
