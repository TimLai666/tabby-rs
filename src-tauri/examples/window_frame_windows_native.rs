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
        NotFound(String),
        Io(String),
        Unsupported(String),
    }

    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidArgument(s)
                | Self::NotFound(s)
                | Self::Io(s)
                | Self::Unsupported(s) => f.write_str(s),
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
            "window_frame_windows_native: Windows-only native frame and real caption-button input checks. On Windows it creates two transparent Wry windows and an isolated temporary WebView data directory; it changes frame styles, moves the cursor, clicks only fresh native caption-button rectangles, changes window state, and closes/destroys only those two windows. On other platforms it performs no native checks."
        );
    } else {
        eprintln!(
            "window_frame_windows_native: Windows checks require Windows; no native checks ran"
        );
    }
}

#[cfg(target_os = "windows")]
fn main() {
    use tauri::WebviewUrl;

    if std::env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!(
            "window_frame_windows_native: checks native caption styles, outer bounds, independent-window isolation, real minimize/maximize/restore/close button input, and frame calls after close. Requests two transparent Wry windows with 640x480 inner size at (32,32) and (64,64), plus an isolated temporary WebView data directory; it moves the cursor and clicks only freshly checked caption-button centers on those windows, then closes/destroys only those windows and removes the data directory. It does not verify composited desktop pixels, CSS dragging, PTY behavior, full-app behavior, restart behavior, or geometry persistence."
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
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClientRect, GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW,
        GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsZoomed, SendMessageTimeoutW,
        SetCursorPos, SetWindowPos, WindowFromPoint, GA_ROOT, GUITHREADINFO, GWL_STYLE,
        SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, TITLEBARINFOEX,
        WM_GETTITLEBARINFOEX, WM_NCHITTEST, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
        WS_THICKFRAME,
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

    fn monitor_info(
        handle: windows_sys::Win32::Foundation::HWND,
        label: &str,
    ) -> Result<MONITORINFO, String> {
        let monitor = unsafe { MonitorFromWindow(handle, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            return Err(format!("{label} MonitorFromWindow returned NULL"));
        }
        let mut info = unsafe { std::mem::zeroed::<MONITORINFO>() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return Err(format!("{label} GetMonitorInfoW failed"));
        }
        Ok(info)
    }

    fn capture_owned_keyboard_focus(
        owned: windows_sys::Win32::Foundation::HWND,
        label: &str,
    ) -> Result<windows_sys::Win32::Foundation::HWND, String> {
        let thread_id = unsafe { GetWindowThreadProcessId(owned, std::ptr::null_mut()) };
        if thread_id == 0 {
            return Err(format!("{label} could not read owned HWND thread id"));
        }
        let mut info = unsafe { std::mem::zeroed::<GUITHREADINFO>() };
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if unsafe { GetGUIThreadInfo(thread_id, &mut info) } == 0 {
            return Err(format!(
                "{label} GetGUIThreadInfo failed for owned thread {thread_id}"
            ));
        }
        let focus = info.hwndFocus;
        if focus.is_null() || unsafe { IsWindow(focus) } == 0 {
            return Err(format!(
                "{label} GUI thread returned no live keyboard focus HWND"
            ));
        }
        if focus != owned && unsafe { GetAncestor(focus, GA_ROOT) } != owned {
            return Err(format!(
                "{label} keyboard focus HWND {focus:?} is outside owned HWND tree {owned:?}"
            ));
        }
        Ok(focus)
    }

    fn verify_owned_keyboard_focus(
        owned: windows_sys::Win32::Foundation::HWND,
        saved_focus: windows_sys::Win32::Foundation::HWND,
        label: &str,
    ) -> Result<(), String> {
        if saved_focus.is_null() || unsafe { IsWindow(saved_focus) } == 0 {
            return Err(format!(
                "{label} saved keyboard-focus HWND is no longer live"
            ));
        }
        let current = capture_owned_keyboard_focus(owned, label)?;
        if current != saved_focus {
            return Err(format!(
                "{label} keyboard focus changed: saved={saved_focus:?}, current={current:?}"
            ));
        }
        if unsafe { GetForegroundWindow() } != owned {
            return Err(format!(
                "{label} owned HWND lost foreground after frame change"
            ));
        }
        Ok(())
    }

    fn client_screen_rect(
        window: &tauri::WebviewWindow,
        label: &str,
    ) -> Result<(i32, i32, i32, i32), String> {
        let handle = hwnd(window)?;
        let mut rect = unsafe { std::mem::zeroed::<RECT>() };
        if unsafe { GetClientRect(handle, &mut rect) } == 0 {
            return Err(format!("{label} GetClientRect failed"));
        }
        let mut top_left = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut bottom_right = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        if unsafe { ClientToScreen(handle, &mut top_left) } == 0
            || unsafe { ClientToScreen(handle, &mut bottom_right) } == 0
        {
            return Err(format!("{label} ClientToScreen failed"));
        }
        Ok((top_left.x, top_left.y, bottom_right.x, bottom_right.y))
    }

    fn exercise_fullscreen_round_trip(
        window: &tauri::WebviewWindow,
        frame: &str,
        baseline: &Snapshot,
    ) -> Result<(), String> {
        let handle = hwnd(window)?;
        let operation = (|| {
            window
                .set_fullscreen(true)
                .map_err(|error| format!("fullscreen-on request failed: {error}"))?;
            let deadline = Instant::now() + TIMEOUT;
            let mut last = None;
            loop {
                let state = snapshot(window)?;
                let monitor = monitor_info(handle, "fullscreen monitor")?;
                let client = client_screen_rect(window, "fullscreen client")?;
                let expected = (
                    monitor.rcMonitor.left,
                    monitor.rcMonitor.top,
                    monitor.rcMonitor.right,
                    monitor.rcMonitor.bottom,
                );
                let fullscreen = window
                    .is_fullscreen()
                    .map_err(|error| format!("fullscreen state readback failed: {error}"))?;
                if fullscreen && client == expected {
                    break;
                }
                last = Some((fullscreen, state, client, expected));
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting for monitor-sized caption-free fullscreen; last state={last:?}"
                    ));
                }
                std::thread::sleep(POLL_INTERVAL);
            }

            focus_owned_window(window, handle, "fullscreen WebView recipient")?;
            let deadline = Instant::now() + TIMEOUT;
            loop {
                let (left, top, right, bottom) = client_screen_rect(window, "fullscreen WebView")?;
                if right - left < 100 || bottom - top < 16 {
                    return Err(
                        "fullscreen client has insufficient area for recipient checks".into(),
                    );
                }
                let points = [
                    POINT {
                        x: right - 8,
                        y: top + 8,
                    },
                    POINT {
                        x: right - 50,
                        y: top + 8,
                    },
                    POINT {
                        x: left + (right - left) / 2,
                        y: top + 8,
                    },
                ];
                let blocked = points.iter().find_map(|point| {
                    let target = unsafe { WindowFromPoint(*point) };
                    (target.is_null()
                        || target == handle
                        || unsafe { GetAncestor(target, GA_ROOT) } != handle)
                        .then_some((point.x, point.y, target as usize))
                });
                if blocked.is_none() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "fullscreen WebView retains a clipped or unowned top area: {blocked:?}"
                    ));
                }
                std::thread::sleep(POLL_INTERVAL);
            }

            window
                .set_fullscreen(false)
                .map_err(|error| format!("fullscreen-off request failed: {error}"))?;
            let deadline = Instant::now() + TIMEOUT;
            let mut last_restored = None;
            let restored = loop {
                let fullscreen = window
                    .is_fullscreen()
                    .map_err(|error| format!("fullscreen-off state readback failed: {error}"))?;
                let state = snapshot(window)?;
                if !fullscreen
                    && state.outer == baseline.outer
                    && state.maximized == baseline.maximized
                    && state.minimized == baseline.minimized
                {
                    break state;
                }
                last_restored = Some((fullscreen, state));
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting for fullscreen-off and ordinary outer bounds; last state={last_restored:?}"
                    ));
                }
                std::thread::sleep(POLL_INTERVAL);
            };
            let frame_rule_matches = if baseline.maximized {
                let work_top = monitor_info(handle, "restored maximized monitor")?
                    .rcWork
                    .top;
                if frame == "native" {
                    restored.client_top > work_top
                } else {
                    restored.client_top == work_top
                }
            } else if frame == "native" {
                restored.client_top > restored.outer.1 + 1
            } else {
                restored.client_top <= restored.outer.1 + 1
            };
            if restored.client_top != baseline.client_top || !frame_rule_matches {
                return Err(format!(
                    "{frame} client-top rule did not return after fullscreen: baseline client_top={}, restored outer={:?}, restored client_top={}",
                    baseline.client_top, restored.outer, restored.client_top
                ));
            }
            Ok(())
        })();

        let cleanup = (|| {
            window
                .set_fullscreen(false)
                .map_err(|error| format!("fullscreen-off cleanup request failed: {error}"))?;
            let deadline = Instant::now() + TIMEOUT;
            loop {
                let fullscreen = window.is_fullscreen().map_err(|error| {
                    format!("fullscreen-off cleanup state readback failed: {error}")
                })?;
                if !fullscreen {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("timed out waiting for fullscreen-off cleanup".into());
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Ok(())
        })();

        match (operation, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(operation_error), Ok(())) => Err(operation_error),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(operation_error), Err(cleanup_error)) => Err(format!(
                "{operation_error}; fullscreen-off cleanup also failed: {cleanup_error}"
            )),
        }
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
            eprintln!(
                "window_frame_windows_native caption metadata: label={label:?} index={index} point=({}, {}) rect={:?}",
                point.x, point.y, (rect.left, rect.top, rect.right, rect.bottom)
            );
            let target = unsafe { WindowFromPoint(point) };
            if target != handle {
                return Err(format!(
                    "{label} center ({},{}) resolves to HWND {target:?}, expected owned HWND {handle:?}",
                    point.x, point.y
                ));
            }
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
                "window_frame_windows_native diagnostic: {label} WM_NCHITTEST sent={} reply={}",
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
            if unsafe { WindowFromPoint(point) } != handle {
                return Err(format!(
                    "{label} fresh button center no longer resolves to owned HWND"
                ));
            }
            if unsafe { SetCursorPos(point.x, point.y) } == 0 {
                return Err(format!("{label} SetCursorPos failed"));
            }
            if unsafe { GetForegroundWindow() } != handle {
                return Err(format!(
                    "{label} owned HWND lost foreground immediately before input"
                ));
            }
            if unsafe { WindowFromPoint(point) } != handle {
                return Err(format!(
                    "{label} point target changed immediately before input"
                ));
            }
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
                    |state| !state.minimized && !state.maximized && state.outer == original_outer,
                    "programmatic unminimize fixture reset",
                ) {
                    failures.push(format!("{label} {error}"));
                }
            }
            Err(error) => failures.push(format!("{label} unminimize reset failed: {error}")),
        }

        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Err(error) = wait_for_state(
            window,
            |state| !state.minimized && !state.maximized && state.outer == original_outer,
            "ordinary state before actual maximize",
        ) {
            failures.push(format!(
                "{label} actual maximize precondition failed; maximize and restore skipped: {error}"
            ));
            return;
        }

        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let maximized = match real_caption_click(window, 3, &format!("{label} maximize")) {
            Ok(()) => wait_for_state(
                window,
                |state| state.maximized && !state.minimized,
                "actual maximize",
            ),
            Err(error) => Err(error),
        };
        match maximized {
            Ok(state) => {
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if let Err(error) = exercise_fullscreen_round_trip(window, label, &state) {
                    failures.push(format!(
                        "{label} maximized fullscreen round-trip failed: {error}"
                    ));
                }
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match hwnd(window).and_then(|handle| monitor_info(handle, "maximized monitor")) {
                    Ok(monitor) => {
                        let work_top = monitor.rcWork.top;
                        let frame_matches = if label == "native" {
                            state.client_top > work_top
                        } else {
                            state.client_top == work_top
                        };
                        if !frame_matches {
                            failures.push(format!(
                                "{label} maximized client top {} does not match monitor work-area top {work_top} (native frame must retain caption inset)",
                                state.client_top
                            ));
                        }
                    }
                    Err(error) => failures.push(format!(
                        "{label} maximized monitor work-area readback failed: {error}"
                    )),
                }

                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match real_caption_click(window, 3, &format!("{label} restore")) {
                    Ok(()) => match wait_for_state(
                        window,
                        |state| {
                            !state.minimized && !state.maximized && state.outer == original_outer
                        },
                        "actual restore to original outer bounds",
                    ) {
                        Ok(_) => {}
                        Err(error) => failures.push(error),
                    },
                    Err(error) => failures.push(error),
                }
            }
            Err(error) => {
                failures.push(format!(
                    "{label} actual maximize was not observed; restore action skipped: {error}"
                ));
            }
        }
        // Preserve the actual-input failure, then restore this fixture's own
        // ordinary state so later frame modes have independent preconditions.
        let ordinary = snapshot(window)
            .map(|state| !state.minimized && !state.maximized && state.outer == original_outer)
            .unwrap_or(false);
        if !ordinary {
            let cleanup = window
                .unmaximize()
                .and_then(|()| window.unminimize())
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    wait_for_state(
                        window,
                        |state| {
                            !state.minimized && !state.maximized && state.outer == original_outer
                        },
                        "programmatic cleanup after caption actions",
                    )
                    .map(|_| ())
                });
            if let Err(error) = cleanup {
                failures.push(format!("{label} caption-action cleanup failed: {error}"));
            }
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
            let main_hwnd = match hwnd(first) {
                Ok(handle) => handle,
                Err(error) => {
                    failures.push(format!("{frame} focus setup HWND readback failed: {error}"));
                    std::ptr::null_mut()
                }
            };
            let saved_focus = if main_hwnd.is_null() {
                None
            } else {
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match focus_owned_window(first, main_hwnd, frame)
                    .and_then(|()| capture_owned_keyboard_focus(main_hwnd, frame))
                {
                    Ok(focus) => Some(focus),
                    Err(error) => {
                        failures.push(format!("{frame} pre-frame keyboard focus failed: {error}"));
                        None
                    }
                }
            };
            let result = apply_frame(first, frame);
            check!(
                "set_window_frame succeeds",
                result.is_ok(),
                format!("{frame}: {result:?}")
            );
            if result.is_ok() {
                if let Some(saved_focus) = saved_focus {
                    CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if let Err(error) = verify_owned_keyboard_focus(
                        main_hwnd,
                        saved_focus,
                        &format!("{frame} focus after ordinary frame change"),
                    ) {
                        failures.push(error);
                    }
                }
            }
            if let Err(error) = nudge_without_changing_bounds(first) {
                failures.push(format!("frame settling: {error}"));
            }
            if let Some(state) = capture!("main after frame", first) {
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match client_screen_rect(first, "ordinary WebView content") {
                    Ok((left, top, right, bottom)) if right > left && bottom > top => {
                        let point = POINT {
                            x: left + (right - left) / 2,
                            y: top + (bottom - top) / 2,
                        };
                        let target = unsafe { WindowFromPoint(point) };
                        if target.is_null()
                            || target == main_hwnd
                            || unsafe { GetAncestor(target, GA_ROOT) } != main_hwnd
                        {
                            failures.push(format!("{frame} ordinary content no longer targets the owned WebView: {target:?}"));
                        }
                    }
                    Ok(rect) => {
                        failures.push(format!("{frame} ordinary content has no area: {rect:?}"))
                    }
                    Err(error) => failures.push(error),
                }
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
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if let Err(error) = exercise_fullscreen_round_trip(first, frame, &state) {
                    failures.push(format!("{frame} fullscreen round-trip failed: {error}"));
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
                "window_frame_windows_native diagnostic: secondary close click failed: {close_result:?}; attempting owned-window cleanup"
            );
        }
        let destroyed = if close_observed {
            true
        } else {
            eprintln!(
                "window_frame_windows_native diagnostic: secondary HWND still alive; using programmatic cleanup fallback"
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
    eprintln!("window_frame_windows_native note: actual_pixels, CSS drag, PTY, full-app/restart behavior, and geometry persistence were not verified");
    failures
}
