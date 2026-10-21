use super::{initialize_hwnd, minimum_proc, scaled, MARKER, SUBCLASS_ID};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Shell::{GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

thread_local! {
    static REBIND_ON_POSITION: Cell<usize> = const { Cell::new(0) };
    static POSITIONS: RefCell<HashMap<isize, Vec<u32>>> = RefCell::new(HashMap::new());
}
static NEXT_CLASS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn fixture_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_WINDOWPOSCHANGING && lparam != 0 {
        let flags = (*(lparam as *const WINDOWPOS)).flags;
        POSITIONS.with(|positions| {
            positions
                .borrow_mut()
                .entry(hwnd as isize)
                .or_default()
                .push(flags)
        });
        let rebind = REBIND_ON_POSITION.with(|target| {
            if target.get() == hwnd as usize {
                target.set(0);
                true
            } else {
                false
            }
        });
        if rebind {
            SetWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID, MARKER + 1);
        }
    }
    let result = DefWindowProcW(hwnd, message, wparam, lparam);
    if message == WM_GETMINMAXINFO && lparam != 0 {
        let constraints = &mut *(lparam as *mut MINMAXINFO);
        constraints.ptMinTrackSize.x = 700;
        constraints.ptMinTrackSize.y = 500;
        constraints.ptMaxTrackSize.x = 9000;
        constraints.ptMaxTrackSize.y = 8000;
        return 0;
    }
    result
}

struct Fixture {
    hwnd: HWND,
    class: Vec<u16>,
}

impl Fixture {
    fn new() -> Self {
        let class: Vec<_> = format!(
            "TabbyGeometry-{}-{}",
            std::process::id(),
            NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        unsafe {
            let module = GetModuleHandleW(ptr::null());
            let mut definition: WNDCLASSW = std::mem::zeroed();
            definition.lpfnWndProc = Some(fixture_proc);
            definition.hInstance = module;
            definition.lpszClassName = class.as_ptr();
            assert_ne!(RegisterClassW(&definition), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                WS_OVERLAPPEDWINDOW,
                40,
                60,
                1100,
                720,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                ptr::null(),
            );
            assert!(!hwnd.is_null());
            POSITIONS.with(|positions| positions.borrow_mut().insert(hwnd as isize, Vec::new()));
            Self { hwnd, class }
        }
    }

    fn rect(&self) -> RECT {
        let mut rect = unsafe { std::mem::zeroed() };
        assert_ne!(unsafe { GetWindowRect(self.hwnd, &mut rect) }, 0);
        rect
    }

    fn constraints(&self) -> MINMAXINFO {
        let mut constraints = unsafe { std::mem::zeroed() };
        unsafe {
            SendMessageW(
                self.hwnd,
                WM_GETMINMAXINFO,
                0,
                &mut constraints as *mut _ as LPARAM,
            )
        };
        constraints
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
            POSITIONS.with(|positions| positions.borrow_mut().remove(&(self.hwnd as isize)));
            UnregisterClassW(self.class.as_ptr(), GetModuleHandleW(ptr::null()));
        }
    }
}

#[test]
fn logical_sizes_round_at_the_live_dpi_and_reject_invalid_or_overflowing_values() {
    for (dpi, width, height) in [
        (96, 800, 600),
        (120, 1000, 750),
        (144, 1200, 900),
        (192, 1600, 1200),
        (97, 808, 606),
    ] {
        assert_eq!(scaled(800, dpi), Some(width));
        assert_eq!(scaled(600, dpi), Some(height));
    }
    assert_eq!(scaled(800, 0), None);
    assert_eq!(scaled(800, u32::MAX), None);
}

#[test]
fn hidden_window_initializes_original_outer_size_without_moving_or_activating_it() {
    let fixture = Fixture::new();
    let before = fixture.rect();
    let dpi = unsafe { GetDpiForWindow(fixture.hwnd) };
    assert!(dpi > 0);
    assert_eq!(unsafe { IsWindowVisible(fixture.hwnd) }, 0);
    assert_eq!(unsafe { IsZoomed(fixture.hwnd) }, 0);
    initialize_hwnd(fixture.hwnd).unwrap();
    let after = fixture.rect();
    assert_eq!((after.left, after.top), (before.left, before.top));
    assert_eq!(
        (after.right - after.left, after.bottom - after.top),
        (scaled(800, dpi).unwrap(), scaled(600, dpi).unwrap())
    );
    assert_eq!(unsafe { IsWindowVisible(fixture.hwnd) }, 0);
    assert_eq!(unsafe { IsZoomed(fixture.hwnd) }, 0);
    let expected = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
    POSITIONS.with(|positions| {
        assert!(positions.borrow()[&(fixture.hwnd as isize)]
            .iter()
            .any(|flags| flags & expected == expected))
    });
    let minimum = fixture.constraints();
    assert_eq!(
        (minimum.ptMinTrackSize.x, minimum.ptMinTrackSize.y),
        (scaled(400, dpi).unwrap(), scaled(300, dpi).unwrap())
    );
    assert_eq!(
        (minimum.ptMaxTrackSize.x, minimum.ptMaxTrackSize.y),
        (9000, 8000)
    );
}

#[test]
fn repeated_initialization_preserves_user_bounds_and_existing_minimum_registration() {
    let fixture = Fixture::new();
    initialize_hwnd(fixture.hwnd).unwrap();
    assert_ne!(
        unsafe {
            SetWindowPos(
                fixture.hwnd,
                ptr::null_mut(),
                0,
                0,
                1000,
                700,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        },
        0
    );
    let before = fixture.rect();
    initialize_hwnd(fixture.hwnd).unwrap();
    let after = fixture.rect();
    assert_eq!(
        (before.left, before.top, before.right, before.bottom),
        (after.left, after.top, after.right, after.bottom)
    );
    let mut marker = 0;
    assert_ne!(
        unsafe { GetWindowSubclass(fixture.hwnd, Some(minimum_proc), SUBCLASS_ID, &mut marker) },
        0
    );
    assert_eq!(marker, MARKER);
}

#[test]
fn foreign_subclass_data_is_never_overwritten_or_removed() {
    let fixture = Fixture::new();
    assert_ne!(
        unsafe { SetWindowSubclass(fixture.hwnd, Some(minimum_proc), SUBCLASS_ID, MARKER + 1) },
        0
    );
    let before = fixture.rect();
    assert!(initialize_hwnd(fixture.hwnd).is_err());
    let after = fixture.rect();
    assert_eq!(
        (before.left, before.top, before.right, before.bottom),
        (after.left, after.top, after.right, after.bottom)
    );
    let mut marker = 0;
    assert_ne!(
        unsafe { GetWindowSubclass(fixture.hwnd, Some(minimum_proc), SUBCLASS_ID, &mut marker) },
        0
    );
    assert_eq!(marker, MARKER + 1);
    let constraints = fixture.constraints();
    assert_eq!(
        (constraints.ptMinTrackSize.x, constraints.ptMinTrackSize.y),
        (700, 500)
    );
    assert_ne!(
        unsafe { RemoveWindowSubclass(fixture.hwnd, Some(minimum_proc), SUBCLASS_ID) },
        0
    );
}

#[test]
fn invalid_and_destroyed_handles_return_errors() {
    assert!(initialize_hwnd(ptr::null_mut()).is_err());
    let mut fixture = Fixture::new();
    let hwnd = fixture.hwnd;
    initialize_hwnd(hwnd).unwrap();
    assert_ne!(unsafe { DestroyWindow(hwnd) }, 0);
    POSITIONS.with(|positions| positions.borrow_mut().remove(&(hwnd as isize)));
    fixture.hwnd = ptr::null_mut();
    let mut marker = 0;
    assert_eq!(
        unsafe { GetWindowSubclass(hwnd, Some(minimum_proc), SUBCLASS_ID, &mut marker) },
        0
    );
    assert!(initialize_hwnd(hwnd).is_err());
}

#[test]
fn initialization_keeps_a_second_window_and_its_constraints_unchanged() {
    let first = Fixture::new();
    let second = Fixture::new();
    let before = second.rect();
    initialize_hwnd(first.hwnd).unwrap();
    let after = second.rect();
    assert_eq!(
        (before.left, before.top, before.right, before.bottom),
        (after.left, after.top, after.right, after.bottom)
    );
    let untouched = second.constraints();
    assert_eq!(
        (untouched.ptMinTrackSize.x, untouched.ptMinTrackSize.y),
        (700, 500)
    );
}

#[test]
fn initialization_rejects_a_window_owned_by_another_thread() {
    let fixture = Fixture::new();
    let before = fixture.rect();
    let raw = fixture.hwnd as usize;
    assert!(
        std::thread::spawn(move || initialize_hwnd(raw as HWND).is_err())
            .join()
            .unwrap()
    );
    let after = fixture.rect();
    assert_eq!(
        (before.left, before.top, before.right, before.bottom),
        (after.left, after.top, after.right, after.bottom)
    );
}

#[test]
fn a_subclass_replaced_during_initialization_is_preserved_and_reported() {
    let fixture = Fixture::new();
    REBIND_ON_POSITION.with(|target| target.set(fixture.hwnd as usize));
    assert!(initialize_hwnd(fixture.hwnd).is_err());
    let mut marker = 0;
    assert_ne!(
        unsafe { GetWindowSubclass(fixture.hwnd, Some(minimum_proc), SUBCLASS_ID, &mut marker) },
        0
    );
    assert_eq!(marker, MARKER + 1);
    let constraints = fixture.constraints();
    assert_eq!(
        (constraints.ptMinTrackSize.x, constraints.ptMinTrackSize.y),
        (700, 500)
    );
    assert_ne!(
        unsafe { RemoveWindowSubclass(fixture.hwnd, Some(minimum_proc), SUBCLASS_ID) },
        0
    );
}
