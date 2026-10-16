use std::sync::mpsc::sync_channel;
use std::time::Duration;

mod desktop {
    #[derive(Clone)]
    pub struct VibrancyOptions {
        pub enabled: bool,
        pub effect: Option<String>,
    }
}

mod error {
    #[derive(Debug)]
    pub enum AppError {
        InvalidArgument(String),
        Io(String),
        Unsupported(String),
    }

    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidArgument(s) | Self::Io(s) | Self::Unsupported(s) => f.write_str(s),
            }
        }
    }
}

#[path = "../src/platform/mod.rs"]
mod platform;

#[cfg(not(target_os = "windows"))]
fn main() {
    if std::env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!(
            "window-frame-caption-clip-probe: Windows-only owned-child mouse diagnostic. It creates two transparent Wry windows and isolated temporary WebView data; clicks fresh caption-button centers only when WindowFromPoint targets an owned window or its child, then checks native window state. This diagnostic is not full-app or parity acceptance."
        );
    } else {
        eprintln!(
            "window-frame-caption-clip-probe: Windows checks require Windows; no native checks ran"
        );
    }
}

#[cfg(target_os = "windows")]
fn main() {
    use tauri::WebviewUrl;

    if std::env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!(
            "window-frame-caption-clip-probe: diagnostic that clips the exact owned WRY_WEBVIEW container around actual DWM caption-button bounds immediately before each mouse action. Requires the resulting target to be the exact owning Tauri HWND. Creates two isolated Wry windows, changes their state, then closes/destroys them. No production refresh lifecycle, full-app, pixel, CSS drag, PTY, restart, persistence, or parity acceptance."
        );
        return;
    }

    let isolated_data = match tempfile::tempdir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!(
                "window_frame_windows_native: cannot create isolated data directory: {error}"
            );
            std::process::exit(1);
        }
    };
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("window_frame_windows_native watchdog: timed out after 180 seconds");
        std::process::exit(1);
    });

    let mut context = tauri::generate_context!();
    let Some(main_config) = context.config().app.windows.first().cloned() else {
        eprintln!("window_frame_windows_native: generated config has no primary window");
        if let Err(error) = cleanup_owned_data(isolated_data) {
            eprintln!("window_frame_windows_native {error}");
        }
        std::process::exit(1);
    };
    context.config_mut().identifier = format!(
        "io.tabbyrs.tests.window-frame-windows.{}",
        std::process::id()
    );
    let data_directory = isolated_data.path().to_path_buf();
    context.config_mut().app.windows.clear();
    let (completed, completion) = sync_channel(1);
    let (worker_ready, worker_handle) = sync_channel(1);
    let app_result = tauri::Builder::default()
        .setup(move |app| {
            let blank = WebviewUrl::External("about:blank".parse()?);
            let mut first_config = main_config.clone();
            first_config.label = "frameWindowsMain".into();
            first_config.url = blank.clone();
            first_config.title = "Native frame Windows test".into();
            first_config.visible = false;
            first_config.transparent = true;
            let first = tauri::WebviewWindowBuilder::from_config(app, &first_config)?
                .inner_size(640.0, 480.0)
                .position(32.0, 32.0)
                .incognito(true)
                .data_directory(data_directory.clone())
                .build()?;

            let mut second_config = first_config.clone();
            second_config.label = "frameWindowsSecondary".into();
            second_config.url = blank;
            second_config.title = "Native frame Windows secondary".into();
            let second = tauri::WebviewWindowBuilder::from_config(app, &second_config)?
                .inner_size(640.0, 480.0)
                .position(64.0, 64.0)
                .incognito(true)
                .data_directory(data_directory)
                .build()?;
            let first_hwnd = first.hwnd()?.0 as usize;
            let second_hwnd = second.hwnd()?.0 as usize;
            if first_hwnd == 0 || second_hwnd == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Tauri returned a null HWND for an owned test window",
                )
                .into());
            }

            let app_handle = app.handle().clone();
            let worker = std::thread::spawn(move || {
                let mut failures = run_checks(&first, &second, second_hwnd);
                for (window, saved_hwnd, label) in [
                    (&second, second_hwnd, "secondary"),
                    (&first, first_hwnd, "primary"),
                ] {
                    if let Err(error) = destroy_owned_window(window, saved_hwnd, label) {
                        failures.push(format!("final {label} window cleanup failed: {error}"));
                    }
                }
                for failure in &failures {
                    eprintln!("window_frame_windows_native failure: {failure}");
                }
                let exit_code = if failures.is_empty() { 0 } else { 1 };
                eprintln!(
                    "window_frame_windows_native completed: checks={} failures={}",
                    CHECKS.load(std::sync::atomic::Ordering::Relaxed),
                    failures.len()
                );
                let _ = completed.send(exit_code);
                app_handle.exit(exit_code);
            });
            let _ = worker_ready.send(worker);
            Ok(())
        })
        .build(context);
    let app = match app_result {
        Ok(app) => app,
        Err(error) => {
            eprintln!("window_frame_windows_native startup failure: {error}");
            let cleanup_result = cleanup_owned_data(isolated_data);
            if let Err(cleanup_error) = cleanup_result {
                eprintln!("window_frame_windows_native {cleanup_error}");
            }
            std::process::exit(1);
        }
    };
    // setup starts the worker only after the event loop receives Ready.
    let app_exit_code = app.run_return(|_, _| {});
    let mut exit_code = completion
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or(1);
    if app_exit_code != 0 {
        exit_code = app_exit_code;
    }
    match worker_handle.recv_timeout(Duration::from_secs(10)) {
        Ok(worker) => {
            if let Err(error) = join_worker_bounded(worker) {
                eprintln!("window_frame_windows_native {error}");
                exit_code = 1;
            }
        }
        Err(error) => {
            eprintln!("window_frame_windows_native: worker did not start: {error}");
            exit_code = 1;
        }
    }
    if let Err(error) = cleanup_owned_data(isolated_data) {
        eprintln!("window_frame_windows_native {error}");
        exit_code = 1;
    }
    std::process::exit(exit_code);
}

#[cfg(target_os = "windows")]
fn join_worker_bounded(worker: std::thread::JoinHandle<()>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !worker.is_finished() {
        if std::time::Instant::now() >= deadline {
            return Err("worker did not release its window handles before timeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    worker
        .join()
        .map_err(|_| "window-frame worker panicked".to_string())
}

#[cfg(target_os = "windows")]
fn cleanup_owned_data(owned_data: tempfile::TempDir) -> Result<(), String> {
    let owned_path = owned_data.path().to_path_buf();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut last_error = None;
    loop {
        match std::fs::remove_dir_all(&owned_path) {
            Ok(()) => {
                let _ = owned_data.keep();
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let _ = owned_data.keep();
                return Ok(());
            }
            Err(error) => last_error = Some(error),
        }
        if std::time::Instant::now() >= deadline {
            let retained_path = owned_data.keep();
            let error = last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no filesystem error was returned".to_string());
            return Err(format!(
                "temporary data cleanup failed after bounded retries for {}: {} (directory retained)",
                retained_path.display(),
                error
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "windows")]
fn destroy_owned_window(
    window: &tauri::WebviewWindow,
    saved_hwnd: usize,
    label: &str,
) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;

    const TIMEOUT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    let handle = saved_hwnd as windows_sys::Win32::Foundation::HWND;
    if handle.is_null() {
        return Err(format!("{label} saved HWND is null"));
    }
    if unsafe { IsWindow(handle) } == 0 {
        return Ok(());
    }
    let current_hwnd = window
        .hwnd()
        .map_err(|error| format!("{label} live HWND lookup failed: {error}"))?
        .0;
    if current_hwnd != handle {
        return Err(format!(
            "{label} live HWND {current_hwnd:?} no longer matches saved owned HWND {handle:?}"
        ));
    }
    let owned_window = window.clone();
    let (sender, receiver) = sync_channel(1);
    window
        .run_on_main_thread(move || {
            let _ = sender.send(owned_window.destroy().map_err(|error| error.to_string()));
        })
        .map_err(|error| format!("{label} destroy scheduling failed: {error}"))?;
    receiver
        .recv_timeout(TIMEOUT)
        .map_err(|error| format!("{label} destroy operation wait failed: {error}"))??;

    let deadline = std::time::Instant::now() + TIMEOUT;
    while unsafe { IsWindow(handle) } != 0 {
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for owned {label} HWND destruction"
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "windows")]
fn run_checks(
    first: &tauri::WebviewWindow,
    second: &tauri::WebviewWindow,
    second_hwnd: usize,
) -> Vec<String> {
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetClientRect, GetForegroundWindow, GetWindowLongPtrW,
        GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsZoomed, SendMessageTimeoutW,
        SetCursorPos, SetWindowPos, WindowFromPoint, GA_ROOT, GWL_STYLE, SMTO_ABORTIFHUNG,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, TITLEBARINFOEX, WM_GETTITLEBARINFOEX,
        WM_NCHITTEST, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };

    const TIMEOUT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const STATE_SYSTEM_INVISIBLE: u32 = 0x0000_8000;
    const STATE_SYSTEM_UNAVAILABLE: u32 = 0x0000_0001;
    const STATE_SYSTEM_OFFSCREEN: u32 = 0x0001_0000;
    const FRAME_BITS: u32 =
        WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_THICKFRAME;

    #[derive(Clone, Debug, PartialEq)]
    struct Snapshot {
        style: u32,
        outer: (i32, i32, i32, i32),
        client_top: i32,
        minimized: bool,
        maximized: bool,
    }

    fn hwnd(window: &tauri::WebviewWindow) -> Result<windows_sys::Win32::Foundation::HWND, String> {
        window
            .hwnd()
            .map(|handle| handle.0)
            .map_err(|error| error.to_string())
    }

    fn snapshot(window: &tauri::WebviewWindow) -> Result<Snapshot, String> {
        let handle = hwnd(window)?;
        let mut rect = unsafe { std::mem::zeroed::<RECT>() };
        let mut client = unsafe { std::mem::zeroed::<RECT>() };
        let (style, got_rect, got_client, minimized, maximized) = unsafe {
            (
                GetWindowLongPtrW(handle, GWL_STYLE) as u32,
                GetWindowRect(handle, &mut rect),
                GetClientRect(handle, &mut client),
                IsIconic(handle) != 0,
                IsZoomed(handle) != 0,
            )
        };
        if got_rect == 0 || got_client == 0 {
            return Err("GetWindowRect or GetClientRect failed".into());
        }
        let mut client_origin = POINT {
            x: client.left,
            y: client.top,
        };
        if unsafe { ClientToScreen(handle, &mut client_origin) } == 0 {
            return Err("ClientToScreen failed".into());
        }
        Ok(Snapshot {
            style,
            outer: (rect.left, rect.top, rect.right, rect.bottom),
            client_top: client_origin.y,
            minimized,
            maximized,
        })
    }

    fn wait_for_state(
        window: &tauri::WebviewWindow,
        expected: impl Fn(&Snapshot) -> bool,
        label: &str,
    ) -> Result<Snapshot, String> {
        let deadline = Instant::now() + TIMEOUT;
        let mut last = None;
        loop {
            let state = snapshot(window)?;
            if expected(&state) {
                return Ok(state);
            }
            last = Some(state);
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for {label}; last state={last:?}"
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn focus_owned_window(
        window: &tauri::WebviewWindow,
        handle: windows_sys::Win32::Foundation::HWND,
        label: &str,
    ) -> Result<(), String> {
        window
            .set_focus()
            .map_err(|error| format!("{label} focus request failed: {error}"))?;
        let deadline = Instant::now() + TIMEOUT;
        while unsafe { GetForegroundWindow() } != handle {
            if Instant::now() >= deadline {
                return Err(format!(
                    "{label} owned HWND did not become foreground within {TIMEOUT:?}"
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    }

    // Diagnostic only: update the exact owned container region immediately
    // before each caption input. This does not implement production refresh.
    fn clip_owned_container_before_input(
        window: &tauri::WebviewWindow,
        owned_root: windows_sys::Win32::Foundation::HWND,
    ) -> Result<(), String> {
        let (sender, receiver) = sync_channel(1);
        let saved_root = owned_root as usize;
        window.with_webview(move |webview| {
            use windows_sys::Win32::Graphics::Dwm::{
                DwmGetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS,
            };
            use windows_sys::Win32::Graphics::Gdi::{
                CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_DIFF,
            };
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetClassNameW, GetParent, GetWindowRect, IsWindow,
            };
            let result = (|| {
                let root = saved_root as windows_sys::Win32::Foundation::HWND;
                let mut parent = unsafe { std::mem::zeroed() };
                unsafe { webview.controller().ParentWindow(&mut parent) }
                    .map_err(|error| format!("controller parent: {error}"))?;
                let child = parent.0;
                if child.is_null() || unsafe { IsWindow(root) } == 0
                    || unsafe { IsWindow(child) } == 0
                    || unsafe { GetParent(child) } != root {
                    return Err("controller parent is not the exact owned child".to_string());
                }
                let mut name = [0u16; 64];
                let length = unsafe { GetClassNameW(child, name.as_mut_ptr(), name.len() as i32) };
                if length <= 0 || String::from_utf16_lossy(&name[..length as usize]) != "WRY_WEBVIEW" {
                    return Err("controller parent is not WRY_WEBVIEW".to_string());
                }
                let mut root_rect: RECT = unsafe { std::mem::zeroed() };
                let mut child_rect: RECT = unsafe { std::mem::zeroed() };
                let mut buttons: RECT = unsafe { std::mem::zeroed() };
                if unsafe { GetWindowRect(root, &mut root_rect) } == 0
                    || unsafe { GetWindowRect(child, &mut child_rect) } == 0 {
                    return Err("owned window bounds lookup failed".to_string());
                }
                let hr = unsafe {
                    DwmGetWindowAttribute(
                        root, DWMWA_CAPTION_BUTTON_BOUNDS as u32,
                        (&mut buttons as *mut RECT).cast(),
                        std::mem::size_of::<RECT>() as u32,
                    )
                };
                if hr < 0 || buttons.right <= buttons.left || buttons.bottom <= buttons.top {
                    return Err(format!("visible DWM caption bounds unavailable: {hr:#x}"));
                }
                let width = child_rect.right - child_rect.left;
                let height = child_rect.bottom - child_rect.top;
                if width <= 0 || height <= 0 {
                    return Err("owned container has no area".to_string());
                }
                let cut = RECT {
                    left: root_rect.left + buttons.left - child_rect.left,
                    top: root_rect.top + buttons.top - child_rect.top,
                    right: root_rect.left + buttons.right - child_rect.left,
                    bottom: root_rect.top + buttons.bottom - child_rect.top,
                };
                eprintln!(
                    "caption-clip-probe ownedRoot={root:?} child={child:?} size={width}x{height} cut={:?}",
                    (cut.left, cut.top, cut.right, cut.bottom)
                );
                let full = unsafe { CreateRectRgn(0, 0, width, height) };
                if full.is_null() { return Err("CreateRectRgn full failed".to_string()); }
                let hole = unsafe { CreateRectRgn(cut.left, cut.top, cut.right, cut.bottom) };
                if hole.is_null() {
                    unsafe { DeleteObject(full); }
                    return Err("CreateRectRgn hole failed".to_string());
                }
                let combined = unsafe { CombineRgn(full, full, hole, RGN_DIFF) };
                unsafe { DeleteObject(hole); }
                if combined == 0 {
                    unsafe { DeleteObject(full); }
                    return Err("CombineRgn failed".to_string());
                }
                if unsafe { SetWindowRgn(child, full, 1) } == 0 {
                    unsafe { DeleteObject(full); }
                    return Err("SetWindowRgn failed".to_string());
                }
                // The OS now owns full, including its eventual destruction.
                Ok(())
            })();
            let _ = sender.send(result);
        }).map_err(|error| format!("owned container scheduling: {error}"))?;
        receiver
            .recv_timeout(TIMEOUT)
            .map_err(|error| format!("owned container clip wait: {error}"))?
    }

    fn point_targets_owned_tree(
        point: POINT,
        owned_root: windows_sys::Win32::Foundation::HWND,
        label: &str,
        stage: &str,
    ) -> Result<(), String> {
        let target = unsafe { WindowFromPoint(point) };
        let target_root = unsafe { GetAncestor(target, GA_ROOT) };
        let mut target_pid = 0u32;
        let mut root_pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(target, &mut target_pid);
            GetWindowThreadProcessId(owned_root, &mut root_pid);
        }
        let mut class_name = [0u16; 256];
        let class_len =
            unsafe { GetClassNameW(target, class_name.as_mut_ptr(), class_name.len() as i32) };
        let class_name = if class_len > 0 {
            String::from_utf16_lossy(&class_name[..class_len as usize])
        } else {
            String::new()
        };
        eprintln!(
            "window-frame-caption-clip-probe diagnostic: label={label:?} stage={stage:?} targetHWND={target:?} targetClass={class_name:?} targetPID={target_pid} ownedRootHWND={owned_root:?} ownedRootPID={root_pid} targetRootAncestorHWND={target_root:?}"
        );
        if target != owned_root {
            return Err(format!(
                "{label} {stage} target is not the exact owned root after clipping: target={target:?}, root ancestor={target_root:?}, expected root={owned_root:?}"
            ));
        }
        Ok(())
    }

    fn fresh_caption_center(
        window: &tauri::WebviewWindow,
        index: usize,
        label: &str,
    ) -> Result<POINT, String> {
        let handle = hwnd(window)?;
        focus_owned_window(window, handle, label)?;
        let previous_dpi =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous_dpi.is_null() {
            return Err(format!("{label} could not set per-monitor-v2 DPI context"));
        }
        let result = (|| {
            let mut info: TITLEBARINFOEX = unsafe { std::mem::zeroed() };
            info.cbSize = std::mem::size_of::<TITLEBARINFOEX>() as u32;
            let mut message_result = 0usize;
            let sent = unsafe {
                SendMessageTimeoutW(
                    handle,
                    WM_GETTITLEBARINFOEX,
                    0,
                    (&mut info as *mut TITLEBARINFOEX) as isize,
                    SMTO_ABORTIFHUNG,
                    TIMEOUT.as_millis() as u32,
                    &mut message_result,
                )
            };
            if sent == 0 {
                return Err(format!(
                    "WM_GETTITLEBARINFOEX timed out or failed for {label}"
                ));
            }
            let rect = info.rgrect[index];
            let state = info.rgstate[index];
            if rect.right <= rect.left || rect.bottom <= rect.top {
                return Err(format!(
                    "{label} rectangle is empty: {:?}",
                    (rect.left, rect.top, rect.right, rect.bottom)
                ));
            }
            if state & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_UNAVAILABLE | STATE_SYSTEM_OFFSCREEN)
                != 0
            {
                return Err(format!(
                    "{label} is unavailable/offscreen: state={state:#x}"
                ));
            }
            let point = POINT {
                x: rect.left + (rect.right - rect.left) / 2,
                y: rect.top + (rect.bottom - rect.top) / 2,
            };
            clip_owned_container_before_input(window, handle)?;
            point_targets_owned_tree(point, handle, label, "fresh metadata")?;
            if unsafe { GetForegroundWindow() } != handle {
                return Err(format!(
                    "{label} owned HWND lost foreground during metadata read"
                ));
            }
            // Windows hit-test results are diagnostic only; a real mouse input
            // followed by observed window state is the pass condition.
            let packed_point = (((point.y as u16 as u32) << 16) | point.x as u16 as u32) as isize;
            message_result = 0;
            let hit_test_sent = unsafe {
                SendMessageTimeoutW(
                    handle,
                    WM_NCHITTEST,
                    0,
                    packed_point,
                    SMTO_ABORTIFHUNG,
                    TIMEOUT.as_millis() as u32,
                    &mut message_result,
                )
            };
            eprintln!(
                "window-frame-caption-clip-probe diagnostic: {label} WM_NCHITTEST sent={} reply={}",
                hit_test_sent != 0,
                message_result as u32
            );
            Ok(point)
        })();
        let restored = unsafe { SetThreadDpiAwarenessContext(previous_dpi) };
        if restored.is_null() {
            return Err(format!("{label} failed to restore prior DPI context"));
        }
        result
    }

    fn send_left_click(label: &str) -> Result<(), String> {
        let events = [
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTDOWN,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];
        let sent = unsafe {
            SendInput(
                events.len() as u32,
                events.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent == events.len() as u32 {
            return Ok(());
        }
        let release = if sent == 1 {
            let event = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            Some(unsafe { SendInput(1, &event, std::mem::size_of::<INPUT>() as i32) })
        } else {
            None
        };
        Err(format!(
            "{label} SendInput accepted {sent}/2 events; emergency left-button release accepted {release:?}"
        ))
    }

    fn real_caption_click(
        window: &tauri::WebviewWindow,
        index: usize,
        label: &str,
    ) -> Result<(), String> {
        let handle = hwnd(window)?;
        let point = fresh_caption_center(window, index, label)?;
        let previous_dpi =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous_dpi.is_null() {
            return Err(format!("{label} could not set input-thread DPI context"));
        }
        let result = (|| {
            if unsafe { GetForegroundWindow() } != handle {
                return Err(format!("{label} owned HWND lost foreground before input"));
            }
            point_targets_owned_tree(point, handle, label, "before cursor placement")?;
            if unsafe { SetCursorPos(point.x, point.y) } == 0 {
                return Err(format!("{label} SetCursorPos failed"));
            }
            if unsafe { GetForegroundWindow() } != handle {
                return Err(format!(
                    "{label} owned HWND lost foreground immediately before input"
                ));
            }
            point_targets_owned_tree(point, handle, label, "immediately before SendInput")?;
            send_left_click(label)
        })();
        let restored = unsafe { SetThreadDpiAwarenessContext(previous_dpi) };
        if restored.is_null() {
            return Err(format!(
                "{label} failed to restore input-thread DPI context"
            ));
        }
        result
    }

    fn exercise_real_caption_actions(
        window: &tauri::WebviewWindow,
        label: &str,
        original_outer: (i32, i32, i32, i32),
        failures: &mut Vec<String>,
    ) {
        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match real_caption_click(window, 2, &format!("{label} minimize")) {
            Ok(()) => match wait_for_state(window, |state| state.minimized, "actual minimize") {
                Ok(_) => {}
                Err(error) => failures.push(error),
            },
            Err(error) => failures.push(error),
        }
        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match window.unminimize() {
            Ok(()) => {
                if let Err(error) = wait_for_state(
                    window,
                    |state| !state.minimized && !state.maximized,
                    "programmatic unminimize fixture reset",
                ) {
                    failures.push(format!("{label} {error}"));
                }
            }
            Err(error) => failures.push(format!("{label} unminimize reset failed: {error}")),
        }

        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match real_caption_click(window, 3, &format!("{label} maximize")) {
            Ok(()) => match wait_for_state(window, |state| state.maximized, "actual maximize") {
                Ok(_) => {}
                Err(error) => failures.push(error),
            },
            Err(error) => failures.push(error),
        }
        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match real_caption_click(window, 3, &format!("{label} restore")) {
            Ok(()) => match wait_for_state(
                window,
                |state| !state.minimized && !state.maximized && state.outer == original_outer,
                "actual restore to original outer bounds",
            ) {
                Ok(_) => {}
                Err(error) => failures.push(error),
            },
            Err(error) => failures.push(error),
        }
    }

    fn issue(
        task: impl FnOnce() -> Result<(), error::AppError> + Send + 'static,
    ) -> Result<Result<(), error::AppError>, String> {
        let (tx, rx) = sync_channel(1);
        std::thread::spawn(move || {
            let _ = tx.send(task());
        });
        rx.recv_timeout(TIMEOUT)
            .map_err(|error| format!("frame operation wait error: {error}"))
    }

    fn apply_frame(window: &tauri::WebviewWindow, frame: &str) -> Result<(), String> {
        let window = window.clone();
        let frame = frame.to_owned();
        issue(move || platform::set_window_frame(&window, &frame))
            .map_err(|wait_error| wait_error)?
            .map_err(|app_error| format!("inner AppError: {app_error:?}"))
    }

    fn nudge_without_changing_bounds(window: &tauri::WebviewWindow) -> Result<(), String> {
        let handle = hwnd(window)?;
        let ok = unsafe {
            SetWindowPos(
                handle,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if ok == 0 {
            Err("SetWindowPos failed".into())
        } else {
            Ok(())
        }
    }

    let mut failures = Vec::new();
    macro_rules! check {
        ($label:expr, $condition:expr, $details:expr) => {{
            CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if !$condition {
                failures.push(format!("{}: {}", $label, $details));
            }
        }};
    }
    macro_rules! capture {
        ($label:expr, $window:expr) => {{
            CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match snapshot($window) {
                Ok(state) => Some(state),
                Err(error) => {
                    failures.push(format!("{} readback: {}", $label, error));
                    None
                }
            }
        }};
    }

    let _ = first.show();
    let _ = second.show();
    let initial_first = capture!("initial main", first);
    let initial_second = capture!("initial secondary", second);
    check!(
        "initial HWND readback",
        initial_first.is_some() && initial_second.is_some(),
        "both native windows must be readable"
    );

    if let (Some(baseline), Some(untouched)) = (initial_first, initial_second) {
        for frame in ["thin", "full", "thin", "native"] {
            let result = apply_frame(first, frame);
            check!(
                "set_window_frame succeeds",
                result.is_ok(),
                format!("{frame}: {result:?}")
            );
            if let Err(error) = nudge_without_changing_bounds(first) {
                failures.push(format!("frame settling: {error}"));
            }
            if let Some(state) = capture!("main after frame", first) {
                check!(
                    "caption and system controls remain available",
                    state.style & FRAME_BITS == FRAME_BITS,
                    format!(
                        "{frame} expected style bits {FRAME_BITS:#x}, actual {:#x}",
                        state.style
                    )
                );
                check!(
                    "frame preserves outer bounds",
                    state.outer == baseline.outer,
                    format!(
                        "{frame} baseline={:?}, actual={:?}",
                        baseline.outer, state.outer
                    )
                );
                if frame == "thin" || frame == "full" {
                    check!(
                        "thin/full client area reaches the caption region",
                        state.client_top <= state.outer.1 + 1,
                        format!(
                            "{frame} outer top={}, client top={}",
                            state.outer.1, state.client_top
                        )
                    );
                } else {
                    check!(
                        "native frame restores client area below the caption",
                        state.client_top > state.outer.1 + 1,
                        format!(
                            "native outer top={}, client top={}",
                            state.outer.1, state.client_top
                        )
                    );
                }
            }
            exercise_real_caption_actions(first, frame, baseline.outer, &mut failures);
            if let Some(state) = capture!("secondary isolation", second) {
                check!(
                    "other window unchanged",
                    state == untouched,
                    format!("after {frame}: expected {untouched:?}, got {state:?}")
                );
            }
        }

        let main_after_first_pass = capture!("main before secondary frames", first);
        for frame in ["thin", "full"] {
            let result = apply_frame(second, frame);
            check!(
                "set_window_frame on secondary succeeds",
                result.is_ok(),
                format!("{frame}: {result:?}")
            );
            if let (Some(expected), Some(actual)) = (
                main_after_first_pass.as_ref(),
                capture!("main secondary isolation", first),
            ) {
                check!(
                    "secondary frame leaves main window unchanged",
                    actual == *expected,
                    format!("after secondary {frame}: expected {expected:?}, got {actual:?}")
                );
            }
        }
    }

    let destroyed_hwnd = second_hwnd as windows_sys::Win32::Foundation::HWND;
    if !destroyed_hwnd.is_null() {
        let handle = destroyed_hwnd;
        let close_result = real_caption_click(second, 5, "secondary full close");
        check!(
            "actual secondary close-button click",
            close_result.is_ok(),
            format!("close button input: {close_result:?}")
        );
        let close_deadline = Instant::now() + TIMEOUT;
        let mut close_observed = false;
        while Instant::now() < close_deadline {
            if unsafe { IsWindow(handle) } == 0 {
                close_observed = true;
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        if close_result.is_ok() {
            check!(
                "actual close click destroys secondary HWND",
                close_observed,
                "close-button input was accepted but the owned HWND remained alive until timeout"
            );
        } else {
            eprintln!(
                "window-frame-caption-clip-probe diagnostic: secondary close click failed: {close_result:?}; attempting owned-window cleanup"
            );
        }
        let destroyed = if close_observed {
            true
        } else {
            eprintln!(
                "window-frame-caption-clip-probe diagnostic: secondary HWND still alive; using programmatic cleanup fallback"
            );
            match destroy_owned_window(second, second_hwnd, "secondary fallback") {
                Ok(()) => (unsafe { IsWindow(handle) }) == 0,
                Err(error) => {
                    failures.push(format!(
                        "secondary programmatic cleanup fallback failed: {error}"
                    ));
                    false
                }
            }
        };
        if destroyed {
            let after_destroy = {
                let window = second.clone();
                issue(move || platform::set_window_frame(&window, "thin"))
            };
            check!(
                "frame call after destroy returns AppError",
                matches!(after_destroy, Ok(Err(_))),
                format!("{after_destroy:?}")
            );
        } else {
            failures.push(
                "skipped post-destroy frame assertion because the owned secondary HWND remains alive".into(),
            );
        }
    } else {
        failures.push("could not save secondary HWND before destroy".into());
        if let Err(error) =
            destroy_owned_window(second, second_hwnd, "secondary without saved HWND")
        {
            failures.push(format!("secondary fallback destruction failed: {error}"));
        }
    }
    eprintln!("window-frame-caption-clip-probe note: DIAGNOSTIC ONLY: clips owned Wry before each input; no production refresh lifecycle; this is not full-app or parity acceptance; composited pixels, CSS drag, PTY, restart behavior, and geometry persistence were not verified");
    failures
}
