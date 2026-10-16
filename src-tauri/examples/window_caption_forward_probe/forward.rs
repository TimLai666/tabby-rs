// Disposable experiment slot. No product code includes this file.
use std::{cell::Cell, sync::Arc};
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
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetClassInfoW, GetClientRect,
            GetParent, GetWindowThreadProcessId, IsWindow, RegisterClassW, SetWindowPos,
            HTMAXBUTTON, HTTRANSPARENT, HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW, WM_NCDESTROY,
            WM_NCHITTEST, WNDCLASSW, WS_CHILD, WS_EX_NOACTIVATE, WS_VISIBLE,
        },
    },
};

const CHILD_ID: usize = 0x5441_4241;
const CONTAINER_ID: usize = 0x5441_4242;
const ROOT_ID: usize = 0x5441_4243;
const CLASS: &[u16] = &[
    84, 97, 98, 98, 121, 67, 97, 112, 116, 105, 111, 110, 79, 118, 101, 114, 108, 97, 121, 80, 114,
    111, 98, 101, 0,
];

struct State {
    parent: HWND,
    container: HWND,
    child: Cell<HWND>,
    rect: RECT,
    closing: Cell<bool>,
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
    _hwnd: HWND,
    _message: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
    _data: usize,
) -> Option<LRESULT> {
    None
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

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        return HTMAXBUTTON as LRESULT;
    }
    DefWindowProcW(hwnd, msg, w, l)
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
    let mut class: WNDCLASSW = std::mem::zeroed();
    class.lpfnWndProc = Some(window_proc);
    class.hInstance = module;
    class.lpszClassName = CLASS.as_ptr();
    if RegisterClassW(&class) == 0 {
        let mut existing: WNDCLASSW = std::mem::zeroed();
        if GetClassInfoW(module, CLASS.as_ptr(), &mut existing) == 0
            || existing.lpfnWndProc != Some(window_proc)
        {
            return Err(fail("overlay window class registration failed"));
        }
    }
    let child = CreateWindowExW(
        WS_EX_NOACTIVATE,
        CLASS.as_ptr(),
        CLASS.as_ptr(),
        WS_CHILD | WS_VISIBLE,
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
