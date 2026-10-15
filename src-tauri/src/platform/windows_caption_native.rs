//! Per-window, non-painting native input for the Windows caption overlay.
use super::{
    events, layout, CaptionButton, CaptionLayout, CaptionMetrics, CaptionPointerEvent,
    CaptionPointerState, WindowsCaptionSnapshot,
};
use crate::error::AppError;
use std::{
    cell::RefCell,
    ffi::c_void,
    io,
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc::sync_channel,
        Arc, OnceLock,
    },
    time::Duration,
};
use tauri::Emitter;
use windows::{core::IInspectable, Foundation::TypedEventHandler, UI::ViewManagement::UISettings};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{BeginPaint, ClientToScreen, EndPaint, ScreenToClient},
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Input::KeyboardAndMouse::{
            GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT,
            TRACKMOUSEEVENT,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetForegroundWindow,
            GetMenuState, GetParent, GetPropW, GetSystemMenu, GetWindowLongPtrW, IsIconic,
            IsWindow, IsZoomed, PostMessageW, RegisterClassW, RemovePropW, SetPropW, SetWindowPos,
            ShowWindow, WindowFromPoint, GWLP_HINSTANCE, GWL_STYLE, HTCLOSE, HTMAXBUTTON,
            HTMINBUTTON, HTTRANSPARENT, HWND_TOP, MF_BYCOMMAND, MF_DISABLED, MF_GRAYED, SC_CLOSE,
            SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYCAPTION,
            SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_ACTIVATE, WM_APP, WM_CANCELMODE,
            WM_CAPTURECHANGED, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
            WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WM_SETTINGCHANGE,
            WM_SHOWWINDOW, WM_SIZE, WM_STYLECHANGED, WM_SYSCOMMAND, WM_WINDOWPOSCHANGED,
            WS_CAPTION, WS_CHILD, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        },
    },
};

const EVENT_NAME: &str = "desktop:windowCaptionControls";
const SUBCLASS_ID: usize = 0x5442_4341;
const REFRESH_MESSAGE: u32 = WM_APP + 0x42;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameMode {
    Native,
    Thin,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OverlayRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

struct Context {
    child: HWND,
    generation: usize,
    active: Arc<AtomicBool>,
    mode: FrameMode,
    transitioning: bool,
    settings: Option<UISettings>,
    text_scale_cookie: Option<i64>,
    snapshot: WindowsCaptionSnapshot,
    pointer: CaptionPointerState,
    updates: events::Sender,
    overlay_rect: Option<OverlayRect>,
    overlay_shown: bool,
}

fn property_name() -> &'static [u16] {
    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    NAME.get_or_init(|| {
        "TabbyRs.WindowsCaption.Context"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    })
}

fn class_name() -> &'static [u16] {
    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    NAME.get_or_init(|| {
        "TabbyRs.WindowsCaption.Overlay"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    })
}

fn class_instance() -> &'static OnceLock<Result<usize, String>> {
    static INSTANCE: OnceLock<Result<usize, String>> = OnceLock::new();
    &INSTANCE
}

fn win32_error(operation: &str) -> AppError {
    AppError::Io(format!("{operation}: {}", io::Error::last_os_error()))
}

fn main_thread<T: Send + 'static>(
    window: &tauri::WebviewWindow,
    task: impl FnOnce(tauri::WebviewWindow) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let (sender, receiver) = sync_channel(1);
    let owned = window.clone();
    // 0 pending, 1 running, 2 cancelled before starting.
    let dispatch = Arc::new(AtomicU8::new(0));
    let scheduled = Arc::clone(&dispatch);
    window
        .run_on_main_thread(move || {
            if scheduled
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let _ = sender.send(task(owned));
            }
        })
        .map_err(|error| AppError::Io(format!("schedule native main-thread task: {error}")))?;
    match receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(result) => result,
        Err(error) => {
            let cancelled = dispatch
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
            let state = if cancelled {
                "cancelled before starting"
            } else {
                "already started; completion is unknown"
            };
            Err(AppError::Io(format!(
                "native main-thread wait: {error}; {state}"
            )))
        }
    }
}

fn window_hwnd(window: &tauri::WebviewWindow) -> Result<HWND, AppError> {
    let hwnd = window
        .hwnd()
        .map_err(|error| AppError::Io(format!("get native window handle: {error}")))?
        .0;
    if hwnd.is_null() {
        return Err(AppError::Io("native window handle is null".into()));
    }
    Ok(hwnd)
}

unsafe fn context_ptr(parent: HWND) -> Option<*mut RefCell<Context>> {
    let value = unsafe { GetPropW(parent, property_name().as_ptr()) };
    (!value.is_null()).then_some(value.cast())
}

fn attached_context(parent: HWND, generation: usize) -> Result<*mut RefCell<Context>, AppError> {
    let pointer = unsafe { context_ptr(parent) }.ok_or_else(|| {
        AppError::Io("caption context disappeared during native operation".into())
    })?;
    let valid = {
        let context = unsafe { (*pointer).borrow() };
        context.generation == generation && context.active.load(Ordering::Acquire)
    };
    if !valid {
        return Err(AppError::Io(
            "caption context changed during native operation".into(),
        ));
    }
    Ok(pointer)
}

fn ensure_overlay_class(parent: HWND) -> Result<*mut c_void, AppError> {
    let instance = unsafe { GetWindowLongPtrW(parent, GWLP_HINSTANCE) } as *mut c_void;
    if instance.is_null() {
        return Err(win32_error("GetWindowLongPtrW(GWLP_HINSTANCE)"));
    }

    let registered = class_instance().get_or_init(|| {
        let mut definition: windows_sys::Win32::UI::WindowsAndMessaging::WNDCLASSW =
            unsafe { std::mem::zeroed() };
        definition.lpfnWndProc = Some(overlay_wnd_proc);
        definition.hInstance = instance;
        definition.lpszClassName = class_name().as_ptr();
        if unsafe { RegisterClassW(&definition) } == 0 {
            Err(format!(
                "RegisterClassW(caption overlay): {}",
                io::Error::last_os_error()
            ))
        } else {
            Ok(instance as usize)
        }
    });

    match registered {
        Ok(registered) if *registered == instance as usize => Ok(instance),
        Ok(_) => Err(AppError::Io(
            "caption overlay class belongs to a different module instance".into(),
        )),
        Err(error) => Err(AppError::Io(error.clone())),
    }
}

fn next_generation() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed).max(1)
}

fn read_text_scale(settings: Option<&UISettings>) -> f32 {
    settings
        .and_then(|settings| settings.TextScaleFactor().ok())
        .map(|scale| scale as f32)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0)
}

fn register_text_scale_listener(
    parent: HWND,
    generation: usize,
    active: &Arc<AtomicBool>,
    settings: Option<&UISettings>,
) -> Result<Option<i64>, AppError> {
    let Some(settings) = settings else {
        return Ok(None);
    };
    let active = Arc::clone(active);
    let parent_addr = parent as usize;
    let handler = TypedEventHandler::<UISettings, IInspectable>::new(move |_, _| {
        if active.load(Ordering::Acquire) {
            unsafe {
                PostMessageW(parent_addr as HWND, REFRESH_MESSAGE, generation, 0);
            }
        }
        Ok(())
    });
    settings
        .TextScaleFactorChanged(&handler)
        .map(Some)
        .map_err(|error| AppError::Io(format!("register text scale listener: {error}")))
}

fn remove_text_scale_listener(
    settings: Option<&UISettings>,
    cookie: Option<i64>,
) -> Option<String> {
    match (settings, cookie) {
        (Some(settings), Some(cookie)) => settings
            .RemoveTextScaleFactorChanged(cookie)
            .err()
            .map(|error| format!("remove text scale listener: {error}")),
        _ => None,
    }
}

pub fn prepare_caption_controls(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    main_thread(window, |window| {
        let parent = window_hwnd(&window)?;
        if let Some(pointer) = unsafe { context_ptr(parent) } {
            let active = unsafe { (*pointer).borrow().active.load(Ordering::Acquire) };
            if active {
                return refresh(parent);
            }
            return Err(AppError::Io(
                "caption controls are still being removed from this window".into(),
            ));
        }

        let instance = ensure_overlay_class(parent)?;
        let child = unsafe {
            CreateWindowExW(
                0,
                class_name().as_ptr(),
                std::ptr::null(),
                WS_CHILD,
                0,
                0,
                0,
                0,
                parent,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if child.is_null() {
            return Err(win32_error("CreateWindowExW(caption overlay)"));
        }

        // Text scaling is optional when WinRT activation is unavailable. If it
        // activates but event registration fails, the whole installation rolls back.
        let settings = UISettings::new().ok();
        let generation = next_generation();
        let active = Arc::new(AtomicBool::new(true));
        let cookie =
            match register_text_scale_listener(parent, generation, &active, settings.as_ref()) {
                Ok(cookie) => cookie,
                Err(error) => {
                    let cleanup = unsafe { DestroyWindow(child) };
                    return Err(append_cleanup_error(
                        error,
                        (cleanup == 0).then(|| win32_error("DestroyWindow(caption overlay)")),
                    ));
                }
            };

        let initial = WindowsCaptionSnapshot::default();
        let (updates, receiver) = events::channel();
        let context = Context {
            child,
            generation,
            active,
            mode: FrameMode::Native,
            transitioning: false,
            settings,
            text_scale_cookie: cookie,
            snapshot: initial,
            pointer: CaptionPointerState::default(),
            updates,
            overlay_rect: None,
            overlay_shown: false,
        };
        let pointer = Box::into_raw(Box::new(RefCell::new(context)));
        if unsafe { SetPropW(parent, property_name().as_ptr(), pointer.cast()) } == 0 {
            let primary_error = win32_error("SetPropW(caption context)");
            let context = unsafe { Box::from_raw(pointer) };
            context.borrow().active.store(false, Ordering::Release);
            let remove_error = remove_text_scale_listener(
                context.borrow().settings.as_ref(),
                context.borrow().text_scale_cookie,
            );
            let cleanup = unsafe { DestroyWindow(child) };
            let mut error = primary_error;
            if let Some(remove_error) = remove_error {
                error = append_cleanup_error(error, Some(AppError::Io(remove_error)));
            }
            if cleanup == 0 {
                error = append_cleanup_error(
                    error,
                    Some(win32_error("DestroyWindow(caption overlay)")),
                );
            }
            drop(context);
            return Err(error);
        }
        let generation = unsafe { (*pointer).borrow().generation };
        if unsafe { SetWindowSubclass(parent, Some(parent_subclass_proc), SUBCLASS_ID, generation) }
            == 0
        {
            let primary_error = win32_error("SetWindowSubclass(caption parent)");
            let context = unsafe { Box::from_raw(pointer) };
            context.borrow().active.store(false, Ordering::Release);
            unsafe { RemovePropW(parent, property_name().as_ptr()) };
            let remove_error = remove_text_scale_listener(
                context.borrow().settings.as_ref(),
                context.borrow().text_scale_cookie,
            );
            let cleanup = unsafe { DestroyWindow(child) };
            let mut error = primary_error;
            if let Some(remove_error) = remove_error {
                error = append_cleanup_error(error, Some(AppError::Io(remove_error)));
            }
            if cleanup == 0 {
                error = append_cleanup_error(
                    error,
                    Some(win32_error("DestroyWindow(caption overlay)")),
                );
            }
            drop(context);
            return Err(error);
        }

        if let Err(error) = refresh(parent) {
            let cleanup = unsafe { detach_context(parent, generation, true) };
            return Err(append_cleanup_error(error, cleanup.err().map(AppError::Io)));
        }

        let pointer = attached_context(parent, generation)?;
        let active = unsafe { Arc::clone(&(*pointer).borrow().active) };
        start_emitter(window, receiver, active);
        Ok(())
    })
}

pub fn caption_state(
    window: &tauri::WebviewWindow,
) -> Result<Option<WindowsCaptionSnapshot>, AppError> {
    main_thread(window, |window| {
        let parent = window_hwnd(&window)?;
        let Some(pointer) = (unsafe { context_ptr(parent) }) else {
            return Ok(None);
        };
        if !unsafe { (*pointer).borrow().active.load(Ordering::Acquire) } {
            return Ok(None);
        }
        refresh(parent)?;
        let pointer = unsafe { context_ptr(parent) }
            .ok_or_else(|| AppError::Io("caption context disappeared during refresh".into()))?;
        Ok(Some(unsafe { (*pointer).borrow().snapshot.clone() }))
    })
}

pub fn set_caption_frame(window: &tauri::WebviewWindow, frame: &str) -> Result<(), AppError> {
    let mode = match frame {
        "native" => FrameMode::Native,
        "thin" => FrameMode::Thin,
        "full" => FrameMode::Full,
        _ => {
            return Err(AppError::InvalidArgument(
                "window frame must be native, thin, or full".into(),
            ));
        }
    };

    main_thread(window, move |window| {
        let parent = window_hwnd(&window)?;
        let old_decorations = window
            .is_decorated()
            .map_err(|error| AppError::Io(format!("read window decorations: {error}")))?;
        let previous = unsafe { context_ptr(parent) }.map(|pointer| {
            let context = unsafe { (*pointer).borrow() };
            (context.generation, context.mode)
        });
        if mode != FrameMode::Native && previous.is_none() {
            return Err(AppError::Io(
                "prepare Windows caption controls before selecting thin or full frame".into(),
            ));
        }
        if let Some((generation, _)) = previous {
            let pointer = attached_context(parent, generation)?;
            unsafe { (*pointer).borrow_mut().transitioning = true };
        }
        if let Err(error) = window.set_decorations(mode == FrameMode::Native) {
            let rollback = restore_frame(&window, parent, previous, old_decorations);
            return Err(AppError::Io(format!(
                "set window decorations: {error}{}",
                rollback.map_or_else(String::new, |detail| format!("; rollback: {detail}"))
            )));
        }
        if let Some((generation, _)) = previous {
            let pointer = attached_context(parent, generation)?;
            let mut context = unsafe { (*pointer).borrow_mut() };
            context.mode = mode;
            context.transitioning = false;
        }
        if let Err(error) = refresh(parent) {
            let rollback = restore_frame(&window, parent, previous, old_decorations);
            return Err(AppError::Io(format!(
                "refresh caption after changing frame: {error}{}",
                rollback.map_or_else(String::new, |detail| format!("; rollback: {detail}"))
            )));
        }
        Ok(())
    })
}

fn restore_frame(
    window: &tauri::WebviewWindow,
    parent: HWND,
    previous: Option<(usize, FrameMode)>,
    decorations: bool,
) -> Option<String> {
    if let Some((generation, _)) = previous {
        let pointer = match attached_context(parent, generation) {
            Ok(pointer) => pointer,
            Err(error) => return Some(error.to_string()),
        };
        unsafe { (*pointer).borrow_mut().transitioning = true };
    }
    let mut errors = Vec::new();
    if let Err(error) = window.set_decorations(decorations) {
        errors.push(format!("restore window decorations: {error}"));
    }
    if let Some((generation, mode)) = previous {
        let pointer = match attached_context(parent, generation) {
            Ok(pointer) => pointer,
            Err(error) => {
                errors.push(error.to_string());
                return Some(errors.join("; "));
            }
        };
        {
            let mut context = unsafe { (*pointer).borrow_mut() };
            context.mode = mode;
            context.transitioning = false;
        }
        if let Err(error) = refresh(parent) {
            errors.push(format!("refresh restored caption: {error}"));
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

fn append_cleanup_error(primary: AppError, cleanup: Option<AppError>) -> AppError {
    match cleanup {
        Some(cleanup) => AppError::Io(format!("{primary}; cleanup failed: {cleanup}")),
        None => primary,
    }
}

fn start_emitter(
    window: tauri::WebviewWindow,
    receiver: events::Receiver,
    active: Arc<AtomicBool>,
) {
    let label = window.label().to_owned();
    tauri::async_runtime::spawn(events::deliver(receiver, active, move |snapshot| {
        let _ = window.emit_to(&label, EVENT_NAME, snapshot);
    }));
}

fn refresh(parent: HWND) -> Result<(), AppError> {
    let Some(pointer) = (unsafe { context_ptr(parent) }) else {
        return Ok(());
    };
    let (child, settings, generation) = {
        let context = unsafe { (*pointer).borrow() };
        if !context.active.load(Ordering::Acquire) || context.transitioning {
            return Ok(());
        }
        (context.child, context.settings.clone(), context.generation)
    };

    let mut client: RECT = unsafe { std::mem::zeroed() };
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(parent, &mut client) }
        == 0
    {
        return Err(win32_error("GetClientRect(caption parent)"));
    }
    let dpi = unsafe { GetDpiForWindow(parent) };
    if dpi == 0 {
        return Err(win32_error("GetDpiForWindow(caption parent)"));
    }
    let style = unsafe { GetWindowLongPtrW(parent, GWL_STYLE) } as u32;
    let maximized = unsafe { IsZoomed(parent) } != 0;
    let minimized = unsafe { IsIconic(parent) } != 0;
    let system_menu = unsafe { GetSystemMenu(parent, 0) };
    if system_menu.is_null() {
        return Err(win32_error("GetSystemMenu(caption parent)"));
    }
    let close_state = unsafe { GetMenuState(system_menu, SC_CLOSE, MF_BYCOMMAND) };
    if close_state == u32::MAX {
        return Err(win32_error("GetMenuState(SC_CLOSE)"));
    }

    let dpi_scale = dpi as f32 / 96.0f32;
    let scale = dpi_scale * read_text_scale(settings.as_ref());
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let mut border_physical = unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) }
        .saturating_add(unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) });
    if style & WS_CAPTION == 0 {
        border_physical = border_physical.saturating_sub(1);
    }
    let border_dip = ((border_physical as f32) / scale).floor() as i32;
    let caption_height_dip = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, 96) };
    let client_width = client.right.saturating_sub(client.left).max(0) as u32;
    let layout = layout(CaptionMetrics {
        client_width,
        scale: f64::from(scale),
        caption_height_dip,
        border_thickness_dip: border_dip,
        maximized,
        fullscreen: style & WS_CAPTION == 0,
        minimizable: style & WS_MINIMIZEBOX != 0,
        maximizable: style & WS_MAXIMIZEBOX != 0,
        closable: close_state & (MF_DISABLED | MF_GRAYED) == 0,
    })?;
    let foreground = unsafe { GetForegroundWindow() } == parent;

    let pointer = attached_context(parent, generation)?;
    let (mode, old_rect, old_shown) = {
        let context = unsafe { (*pointer).borrow() };
        if !context.active.load(Ordering::Acquire) || context.transitioning {
            return Ok(());
        }
        (context.mode, context.overlay_rect, context.overlay_shown)
    };
    let visible = mode != FrameMode::Native
        && !minimized
        && layout.clip.width > 0.0
        && layout.clip.height > 0.0;
    let desired_rect = visible.then(|| overlay_rect(&layout)).flatten();
    let visible = visible && desired_rect.is_some();

    if visible {
        let desired_rect = desired_rect.expect("visible caption layout has an overlay rectangle");
        if old_rect != Some(desired_rect) || !old_shown {
            // Reserve before SetWindowPos so any reentrant parent notification
            // observes the target geometry without a live RefCell borrow.
            unsafe {
                let mut context = (*pointer).borrow_mut();
                context.overlay_rect = Some(desired_rect);
                context.overlay_shown = true;
            }
            let moved = unsafe {
                SetWindowPos(
                    child,
                    HWND_TOP,
                    desired_rect.x,
                    desired_rect.y,
                    desired_rect.width,
                    desired_rect.height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
            let move_error = (moved == 0).then(|| win32_error("SetWindowPos(caption overlay)"));
            let pointer = attached_context(parent, generation)?;
            if let Some(error) = move_error {
                unsafe {
                    let mut context = (*pointer).borrow_mut();
                    if context.overlay_rect == Some(desired_rect) {
                        context.overlay_rect = old_rect;
                        context.overlay_shown = old_shown;
                    }
                }
                return Err(error);
            }
        }
    } else if old_shown {
        unsafe {
            let mut context = (*pointer).borrow_mut();
            context.overlay_shown = false;
        }
        unsafe { ShowWindow(child, SW_HIDE) };
    }

    let hovered = if visible {
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } != 0
            && (unsafe { WindowFromPoint(point) } == child || unsafe { GetCapture() } == child)
            && unsafe { ScreenToClient(parent, &mut point) } != 0
        {
            layout.hit_test(point.x, point.y).filter(|button| {
                layout
                    .buttons
                    .iter()
                    .any(|item| item.button == *button && item.enabled)
            })
        } else {
            None
        }
    } else {
        None
    };
    let pointer = attached_context(parent, generation)?;
    let (changed, cancel_capture, updates) = {
        let mut context = unsafe { (*pointer).borrow_mut() };
        if !context.active.load(Ordering::Acquire) || context.transitioning {
            return Ok(());
        }
        let old = &context.snapshot;
        let pointer_state = if visible {
            CaptionPointerState {
                hovered,
                ..context.pointer
            }
        } else {
            CaptionPointerState::default()
        };
        let changed = old.visible != visible
            || old.scale != f64::from(scale)
            || old.layout != layout
            || old.pointer != pointer_state
            || old.maximized != maximized
            || old.active != foreground;
        let cancel_capture = !visible && context.pointer.pressed.is_some();
        context.pointer = pointer_state;
        if changed {
            let next = WindowsCaptionSnapshot {
                revision: context.snapshot.revision.wrapping_add(1),
                visible,
                scale: f64::from(scale),
                layout,
                pointer: context.pointer,
                maximized,
                active: foreground,
            };
            context.snapshot = next.clone();
            (Some(next), cancel_capture, Some(context.updates.clone()))
        } else {
            (None, cancel_capture, None)
        }
    };

    if cancel_capture && unsafe { GetCapture() } == child {
        unsafe { ReleaseCapture() };
    }
    if let (Some(snapshot), Some(updates)) = (changed, updates) {
        let _ = events::publish(&updates, snapshot);
    }
    Ok(())
}

fn overlay_rect(layout: &CaptionLayout) -> Option<OverlayRect> {
    let left = layout.clip.x.ceil();
    let top = layout.clip.y.ceil();
    let right = (layout.clip.x + layout.clip.width).ceil();
    let bottom = (layout.clip.y + layout.clip.height).ceil();
    if ![left, top, right, bottom]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    let width = right - left;
    let height = bottom - top;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(OverlayRect {
        x: left.clamp(i32::MIN as f64, i32::MAX as f64) as i32,
        y: top.clamp(i32::MIN as f64, i32::MAX as f64) as i32,
        width: width.clamp(1.0, i32::MAX as f64) as i32,
        height: height.clamp(1.0, i32::MAX as f64) as i32,
    })
}

fn update_pointer(parent: HWND, event: CaptionPointerEvent) -> Option<CaptionButton> {
    let pointer = unsafe { context_ptr(parent) }?;
    let (activation, changed) = {
        let mut context = unsafe { (*pointer).borrow_mut() };
        if !context.active.load(Ordering::Acquire) {
            return None;
        }
        let before = context.pointer;
        let activation = context.pointer.update(event);
        if before == context.pointer {
            (activation, None)
        } else {
            let mut snapshot = context.snapshot.clone();
            snapshot.revision = snapshot.revision.wrapping_add(1);
            snapshot.pointer = context.pointer;
            context.snapshot = snapshot.clone();
            (activation, Some((context.updates.clone(), snapshot)))
        }
    };
    if let Some((updates, snapshot)) = changed {
        let _ = events::publish(&updates, snapshot);
    }
    activation
}

fn layout_at(parent: HWND, point: POINT) -> Option<(CaptionButton, bool)> {
    let pointer = unsafe { context_ptr(parent) }?;
    let layout = {
        let context = unsafe { (*pointer).borrow() };
        if !context.active.load(Ordering::Acquire) {
            return None;
        }
        context.snapshot.layout.clone()
    };
    let button = layout.hit_test(point.x, point.y)?;
    let enabled = layout
        .buttons
        .iter()
        .find(|item| item.button == button)
        .is_some_and(|item| item.enabled);
    Some((button, enabled))
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let packed = lparam as u32;
    POINT {
        x: (packed as u16 as i16) as i32,
        y: ((packed >> 16) as u16 as i16) as i32,
    }
}

fn screen_point_in_parent(parent: HWND, point: POINT) -> Option<POINT> {
    let mut point = point;
    (unsafe { ScreenToClient(parent, &mut point) } != 0).then_some(point)
}

fn child_point_in_parent(child: HWND, point: POINT) -> Option<POINT> {
    let parent = unsafe { GetParent(child) };
    if parent.is_null() {
        return None;
    }
    let mut point = point;
    if unsafe { ClientToScreen(child, &mut point) } == 0
        || unsafe { ScreenToClient(parent, &mut point) } == 0
    {
        return None;
    }
    Some(point)
}

fn button_hit(button: CaptionButton) -> LRESULT {
    match button {
        CaptionButton::Minimize => HTMINBUTTON as LRESULT,
        CaptionButton::Maximize => HTMAXBUTTON as LRESULT,
        CaptionButton::Close => HTCLOSE as LRESULT,
    }
}

fn track_leave(child: HWND, non_client: bool) {
    let mut tracking = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | if non_client { TME_NONCLIENT } else { 0 },
        hwndTrack: child,
        dwHoverTime: 0,
    };
    unsafe { TrackMouseEvent(&mut tracking) };
}

fn press(parent: HWND, child: HWND, point: POINT) {
    let Some((button, true)) = layout_at(parent, point) else {
        return;
    };
    update_pointer(parent, CaptionPointerEvent::Press(button));
    unsafe { SetCapture(child) };
    if unsafe { GetCapture() } != child {
        update_pointer(parent, CaptionPointerEvent::Cancel);
    }
}

fn release(parent: HWND, child: HWND, point: POINT) {
    let button = layout_at(parent, point)
        .filter(|(_, enabled)| *enabled)
        .map(|(button, _)| button);
    let activation = update_pointer(parent, CaptionPointerEvent::Release(button));
    if unsafe { GetCapture() } == child {
        unsafe { ReleaseCapture() };
    }
    if let Some(button) = activation {
        let command = match button {
            CaptionButton::Minimize => SC_MINIMIZE,
            CaptionButton::Maximize => {
                if unsafe { IsZoomed(parent) } != 0 {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                }
            }
            CaptionButton::Close => SC_CLOSE,
        };
        unsafe { PostMessageW(parent, WM_SYSCOMMAND, command as WPARAM, 0) };
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    child: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => {
            let parent = unsafe { GetParent(child) };
            if parent.is_null() {
                return HTTRANSPARENT as LRESULT;
            }
            let Some(point) = screen_point_in_parent(parent, point_from_lparam(lparam)) else {
                return HTTRANSPARENT as LRESULT;
            };
            layout_at(parent, point)
                .map(|(button, _)| button_hit(button))
                .unwrap_or(HTTRANSPARENT as LRESULT)
        }
        WM_NCMOUSEMOVE => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                let point = screen_point_in_parent(parent, point_from_lparam(lparam));
                update_pointer(
                    parent,
                    CaptionPointerEvent::Move(
                        point.and_then(|point| layout_at(parent, point).map(|hit| hit.0)),
                    ),
                );
                track_leave(child, true);
            }
            0
        }
        WM_MOUSEMOVE => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                let point = child_point_in_parent(child, point_from_lparam(lparam));
                update_pointer(
                    parent,
                    CaptionPointerEvent::Move(
                        point.and_then(|point| layout_at(parent, point).map(|hit| hit.0)),
                    ),
                );
                track_leave(child, false);
            }
            0
        }
        WM_NCMOUSELEAVE | WM_MOUSELEAVE => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                update_pointer(parent, CaptionPointerEvent::Move(None));
            }
            0
        }
        // Windows delivers the second down as a non-client double-click even
        // without CS_DBLCLKS. It still starts an ordinary caption press.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                if let Some(point) = screen_point_in_parent(parent, point_from_lparam(lparam)) {
                    press(parent, child, point);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                if let Some(point) = child_point_in_parent(child, point_from_lparam(lparam)) {
                    press(parent, child, point);
                }
            }
            0
        }
        WM_NCLBUTTONUP => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                if let Some(point) = screen_point_in_parent(parent, point_from_lparam(lparam)) {
                    release(parent, child, point);
                }
            }
            0
        }
        WM_LBUTTONUP => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                if let Some(point) = child_point_in_parent(child, point_from_lparam(lparam)) {
                    release(parent, child, point);
                }
            }
            0
        }
        WM_CAPTURECHANGED => {
            let parent = unsafe { GetParent(child) };
            if let Some(pointer) = unsafe { context_ptr(parent) } {
                let pressed = unsafe { (*pointer).borrow().pointer.pressed.is_some() };
                if pressed {
                    update_pointer(parent, CaptionPointerEvent::Cancel);
                }
            }
            0
        }
        WM_CANCELMODE => {
            let parent = unsafe { GetParent(child) };
            if !parent.is_null() {
                update_pointer(parent, CaptionPointerEvent::Cancel);
                if unsafe { GetCapture() } == child {
                    unsafe { ReleaseCapture() };
                }
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT =
                unsafe { std::mem::zeroed() };
            unsafe {
                BeginPaint(child, &mut paint);
                EndPaint(child, &paint);
            }
            0
        }
        _ => unsafe { DefWindowProcW(child, message, wparam, lparam) },
    }
}

unsafe fn detach_context(
    parent: HWND,
    generation: usize,
    destroy_child: bool,
) -> Result<(), String> {
    let Some(pointer) = (unsafe { context_ptr(parent) }) else {
        return Ok(());
    };
    let (child, active, settings, cookie, pressed) = {
        let mut context = unsafe { (*pointer).borrow_mut() };
        if context.generation != generation {
            return Ok(());
        }
        context.active.store(false, Ordering::Release);
        let pressed = context.pointer.pressed.is_some();
        context.pointer.update(CaptionPointerEvent::Cancel);
        context.snapshot.pointer = CaptionPointerState::default();
        (
            context.child,
            Arc::clone(&context.active),
            context.settings.clone(),
            context.text_scale_cookie,
            pressed,
        )
    };
    active.store(false, Ordering::Release);
    let mut errors = Vec::new();
    if unsafe { RemoveWindowSubclass(parent, Some(parent_subclass_proc), SUBCLASS_ID) } == 0 {
        errors.push("RemoveWindowSubclass(caption parent) failed".to_owned());
    }
    let removed = unsafe { RemovePropW(parent, property_name().as_ptr()) } == pointer.cast();
    if !removed {
        errors.push("RemovePropW(caption context) did not return its context".to_owned());
    }
    if pressed && unsafe { GetCapture() } == child {
        unsafe { ReleaseCapture() };
    }
    if let Some(error) = remove_text_scale_listener(settings.as_ref(), cookie) {
        errors.push(error);
    }
    if destroy_child && unsafe { IsWindow(child) } != 0 && unsafe { DestroyWindow(child) } == 0 {
        errors.push(format!(
            "DestroyWindow(caption overlay): {}",
            io::Error::last_os_error()
        ));
    }
    if removed {
        unsafe { drop(Box::from_raw(pointer)) };
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

unsafe extern "system" fn parent_subclass_proc(
    parent: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    generation: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        let _ = unsafe { detach_context(parent, generation, true) };
        return unsafe { DefSubclassProc(parent, message, wparam, lparam) };
    }

    let Some(pointer) = (unsafe { context_ptr(parent) }) else {
        return unsafe { DefSubclassProc(parent, message, wparam, lparam) };
    };
    let valid = {
        let context = unsafe { (*pointer).borrow() };
        context.generation == generation && context.active.load(Ordering::Acquire)
    };
    if !valid {
        return unsafe { DefSubclassProc(parent, message, wparam, lparam) };
    }
    if message == REFRESH_MESSAGE {
        if wparam == generation {
            let _ = refresh(parent);
        }
        return 0;
    }

    let result = unsafe { DefSubclassProc(parent, message, wparam, lparam) };
    if matches!(
        message,
        WM_SIZE
            | WM_WINDOWPOSCHANGED
            | WM_STYLECHANGED
            | WM_DPICHANGED
            | WM_ACTIVATE
            | WM_SETTINGCHANGE
            | WM_SHOWWINDOW
    ) {
        let still_attached = unsafe { context_ptr(parent) }
            .is_some_and(|pointer| unsafe { (*pointer).borrow().generation == generation });
        if still_attached {
            let _ = refresh(parent);
        }
    }
    result
}
