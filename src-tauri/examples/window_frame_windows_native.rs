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
            "window_frame_windows_native: Windows-only native frame checks. On Windows this creates two transparent Wry windows and an isolated temporary WebView data directory; it changes frame styles, window state, and destroys only those two windows. On other platforms it performs no native checks."
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
            "window_frame_windows_native: checks native caption styles, outer bounds, independent-window isolation, minimize/maximize/restore, and frame calls after destroy. Creates two transparent Wry windows and a temporary isolated WebView data directory, then destroys only those windows and removes that directory. It does not verify actual_pixels, CSS dragging, PTY behavior, full-app behavior, restart behavior, or geometry persistence."
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
    let watchdog_data = isolated_data.path().to_path_buf();
    let timeout_data = watchdog_data.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("window_frame_windows_native watchdog: timed out after 180 seconds");
        let _ = std::fs::remove_dir_all(timeout_data);
        std::process::exit(1);
    });

    let mut context = tauri::generate_context!();
    let Some(main_config) = context.config().app.windows.first().cloned() else {
        let _ = isolated_data.close();
        eprintln!("window_frame_windows_native: generated config has no primary window");
        std::process::exit(1);
    };
    context.config_mut().identifier = format!(
        "io.tabbyrs.tests.window-frame-windows.{}",
        std::process::id()
    );
    context.config_mut().app.windows.clear();
    let (completed, completion) = sync_channel(1);
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
                .incognito(true)
                .data_directory(isolated_data.path().to_path_buf())
                .build()?;

            let mut second_config = first_config.clone();
            second_config.label = "frameWindowsSecondary".into();
            second_config.url = blank;
            second_config.title = "Native frame Windows secondary".into();
            let second = tauri::WebviewWindowBuilder::from_config(app, &second_config)?
                .incognito(true)
                .data_directory(isolated_data.path().to_path_buf())
                .build()?;

            let app_handle = app.handle().clone();
            let owned_data = isolated_data;
            std::thread::spawn(move || {
                let mut failures = run_checks(&first, &second);
                let _ = first.destroy();
                let _ = second.destroy();
                if let Err(error) = owned_data.close() {
                    failures.push(format!("temporary data cleanup failed: {error}"));
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
            Ok(())
        })
        .build(context);
    let app = match app_result {
        Ok(app) => app,
        Err(error) => {
            eprintln!("window_frame_windows_native startup failure: {error}");
            let _ = std::fs::remove_dir_all(watchdog_data);
            std::process::exit(1);
        }
    };
    app.run_return(|_, _| {});
    let exit_code = completion
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or(1);
    std::process::exit(exit_code);
}

#[cfg(target_os = "windows")]
static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "windows")]
fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindow, IsZoomed,
        SendMessageTimeoutW, SetWindowPos, GWL_STYLE, HTCLOSE, HTMAXBUTTON, HTMINBUTTON,
        SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, TITLEBARINFOEX,
        WM_GETTITLEBARINFOEX, WM_NCHITTEST, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
        WS_THICKFRAME,
    };

    const TIMEOUT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const STATE_SYSTEM_INVISIBLE: u32 = 0x0000_8000;
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

    fn hit_test_caption_button(
        handle: windows_sys::Win32::Foundation::HWND,
        index: usize,
        expected_hit: u32,
        label: &str,
    ) -> Result<(), String> {
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
        if state & STATE_SYSTEM_INVISIBLE != 0 {
            return Err(format!("{label} is marked invisible: state={state:#x}"));
        }
        let x = rect.left + (rect.right - rect.left) / 2;
        let y = rect.top + (rect.bottom - rect.top) / 2;
        let packed_point = (((y as u16 as u32) << 16) | x as u16 as u32) as isize;
        message_result = 0;
        let sent = unsafe {
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
        if sent == 0 {
            return Err(format!("WM_NCHITTEST timed out or failed for {label}"));
        }
        if message_result as u32 != expected_hit {
            return Err(format!(
                "{label} center ({x},{y}) returned hit code {}, expected {expected_hit}",
                message_result as u32
            ));
        }
        Ok(())
    }

    fn check_caption_buttons(
        window: &tauri::WebviewWindow,
        label: &str,
        failures: &mut Vec<String>,
    ) {
        let handle = match hwnd(window) {
            Ok(handle) => handle,
            Err(error) => {
                failures.push(format!("{label} caption HWND: {error}"));
                return;
            }
        };
        for (index, expected, name) in [
            (2, HTMINBUTTON, "minimize"),
            (3, HTMAXBUTTON, "maximize"),
            (5, HTCLOSE, "close"),
        ] {
            CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Err(error) = hit_test_caption_button(handle, index, expected, name) {
                failures.push(format!("{label} {name} caption hit-test: {error}"));
            }
        }
    }

    fn exercise_window_states(
        window: &tauri::WebviewWindow,
        label: &str,
        failures: &mut Vec<String>,
    ) {
        let baseline = match snapshot(window) {
            Ok(state) => state,
            Err(error) => {
                failures.push(format!("{label} state baseline: {error}"));
                return;
            }
        };
        macro_rules! state_step {
            ($name:literal, $call:expr, $predicate:expr) => {{
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match $call {
                    Ok(()) => match wait_for_state(window, $predicate, $name) {
                        Ok(_) => {}
                        Err(error) => failures.push(format!("{label} {error}")),
                    },
                    Err(error) => failures.push(format!("{label} {} operation: {error}", $name)),
                }
            }};
        }
        state_step!("minimize", window.minimize(), |state: &Snapshot| state
            .minimized);
        state_step!("unminimize", window.unminimize(), |state: &Snapshot| !state
            .minimized
            && !state.maximized);
        state_step!("maximize", window.maximize(), |state: &Snapshot| state
            .maximized);
        state_step!("unmaximize", window.unmaximize(), |state: &Snapshot| !state
            .minimized
            && !state.maximized);
        match wait_for_state(
            window,
            |state| !state.minimized && !state.maximized && state.outer == baseline.outer,
            "restored outer bounds",
        ) {
            Ok(_) => {
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(error) => failures.push(format!("{label} restore bounds: {error}")),
        };
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
            check_caption_buttons(first, frame, &mut failures);
            exercise_window_states(first, frame, &mut failures);
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
            check_caption_buttons(second, &format!("secondary {frame}"), &mut failures);
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
        let restore_secondary = apply_frame(second, "native");
        check!(
            "restore secondary native frame",
            restore_secondary.is_ok(),
            format!("{restore_secondary:?}")
        );
    }

    let destroyed_hwnd = hwnd(second).ok();
    let destroyed = {
        let window = second.clone();
        let (tx, rx) = sync_channel(1);
        let scheduled = second.run_on_main_thread(move || {
            let _ = tx.send(window.destroy().map_err(|error| error.to_string()));
        });
        scheduled.map_err(|error| error.to_string()).and_then(|_| {
            rx.recv_timeout(TIMEOUT)
                .map_err(|error| format!("destroy wait error: {error}"))?
        })
    };
    check!(
        "destroy second test window",
        destroyed.is_ok(),
        format!("{destroyed:?}")
    );
    if let Some(handle) = destroyed_hwnd {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if unsafe { IsWindow(handle) } == 0 {
                CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                break;
            }
            if Instant::now() >= deadline {
                failures.push("timed out waiting for the owned HWND to be destroyed".into());
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    } else {
        failures.push("could not save secondary HWND before destroy".into());
    }
    let after_destroy = {
        let window = second.clone();
        issue(move || platform::set_window_frame(&window, "thin"))
    };
    check!(
        "frame call after destroy returns AppError",
        matches!(after_destroy, Ok(Err(_))),
        format!("{after_destroy:?}")
    );
    eprintln!("window_frame_windows_native note: actual_pixels, CSS drag, PTY, full-app/restart behavior, and geometry persistence were not verified");
    failures
}
