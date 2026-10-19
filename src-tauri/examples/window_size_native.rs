//! Real Tauri/Wry outer-size fixture. This is not full desktop parity coverage.
use std::{
    sync::mpsc::sync_channel,
    time::{Duration, Instant},
};

#[path = "../src/window_caption.rs"]
mod caption;

mod desktop {
    pub use crate::caption::WindowCaptionState;
    #[cfg(target_os = "windows")]
    pub use crate::caption::{WindowCaptionButton, WindowCaptionButtonKind};
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
    eprintln!("window_size_native: Windows-only real Tauri/Wry sizing fixture; this is not full desktop parity; skipped on this platform");
}

#[cfg(target_os = "windows")]
fn main() {
    use tauri::WebviewUrl;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Usage: cargo run --locked --manifest-path src-tauri/Cargo.toml --example window_size_native\nRuns a real Windows Tauri/Wry native outer-size fixture for native, thin, and full frames using two WebviewWindows. This is not full desktop parity coverage.");
        return;
    }
    if !args.is_empty() {
        fail(format!(
            "unknown arguments: {} (use --help)",
            args.join(" ")
        ));
    }
    println!("window_size_native: running real Tauri/Wry sizing checks for native/thin/full frames and two windows; not full desktop parity coverage");
    let isolated = tempfile::tempdir()
        .unwrap_or_else(|error| fail(format!("temporary data directory: {error}")));
    let timeout_path = isolated.path().to_path_buf();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("window_size_native watchdog: timed out after 180 seconds");
        let _ = std::fs::remove_dir_all(timeout_path);
        std::process::exit(1);
    });
    let mut context = tauri::generate_context!();
    let Some(base) = context.config().app.windows.first().cloned() else {
        fail("generated config has no primary window".into())
    };
    context.config_mut().identifier =
        format!("io.tabbyrs.tests.window-size.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let (done_tx, done_rx) = sync_channel(1);
    let app_result = tauri::Builder::default()
        .setup(move |app| {
            let blank = WebviewUrl::External("about:blank".parse()?);
            let make = |label: &str, x: i32| {
                let mut config = base.clone();
                config.label = label.into();
                config.url = blank.clone();
                config.title = format!("Window size fixture {label}");
                config.visible = false;
                config.x = Some(x as f64);
                config.y = Some(120.0);
                tauri::WebviewWindowBuilder::from_config(app, &config)?
                    .incognito(true)
                    .data_directory(isolated.path().to_path_buf())
                    .build()
            };
            let first = make("sizeFirst", 80)?;
            let second = make("sizeSecond", 1120)?;
            let handle = app.handle().clone();
            let owned_temp = isolated;
            std::thread::spawn(move || {
                let failures = run_checks(&first, &second);
                for window in [&first, &second] {
                    let _ = window.destroy();
                }
                for failure in &failures {
                    eprintln!("window_size_native failure: {failure}");
                }
                let code = i32::from(!failures.is_empty());
                eprintln!(
                    "window_size_native completed: checks={} failures={}",
                    CHECKS.load(std::sync::atomic::Ordering::Relaxed),
                    failures.len()
                );
                let _ = done_tx.send((code, owned_temp));
                handle.exit(code);
            });
            Ok(())
        })
        .build(context);
    let mut app = app_result.unwrap_or_else(|error| fail(format!("startup: {error}")));
    app.run_return(|_, _| {});
    let (mut exit_code, owned_temp) = match done_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(result) => result,
        Err(error) => fail(format!(
            "worker completion/temporary directory handoff failed: {error}"
        )),
    };
    let temp_path = owned_temp.path().to_path_buf();
    if let Err(error) = remove_temp_with_retry(&temp_path, Duration::from_secs(10)) {
        eprintln!("window_size_native temporary data cleanup failed: {error}");
        exit_code = 1;
    }
    drop(owned_temp);
    std::process::exit(exit_code);
}

#[cfg(target_os = "windows")]
fn fail(message: String) -> ! {
    eprintln!("window_size_native error: {message}");
    std::process::exit(1)
}

#[cfg(target_os = "windows")]
fn remove_temp_with_retry(path: &std::path::Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
                if Instant::now() >= deadline {
                    return Err(format!("{}: {error}", path.display()));
                }
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
}

#[cfg(target_os = "windows")]
static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "windows")]
fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::{
        Foundation::*,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    };
    const QUICK_WAIT: Duration = Duration::from_secs(1);
    const RUN_BUDGET: Duration = Duration::from_secs(120);
    #[derive(Clone, Debug, PartialEq)]
    struct State {
        size: tauri::PhysicalSize<u32>,
        position: tauri::PhysicalPosition<i32>,
        visible: bool,
    }
    fn state(w: &tauri::WebviewWindow) -> Result<State, String> {
        Ok(State {
            size: w.outer_size().map_err(|e| e.to_string())?,
            position: w.outer_position().map_err(|e| e.to_string())?,
            visible: w.is_visible().map_err(|e| e.to_string())?,
        })
    }
    fn outer(w: &tauri::WebviewWindow, size: tauri::PhysicalSize<u32>) -> Result<(), String> {
        platform::set_window_outer_size(w, size).map_err(|e| format!("AppError: {e}"))
    }
    fn wait_size(
        w: &tauri::WebviewWindow,
        expected: tauri::PhysicalSize<u32>,
        label: &str,
        frame: &str,
        timeout: Duration,
    ) -> Result<State, String> {
        let start = Instant::now();
        let mut last = None;
        while start.elapsed() < timeout {
            if let Ok(s) = state(w) {
                last = Some(s.clone());
                if s.size == expected {
                    return Ok(s);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let dpi = w
            .hwnd()
            .ok()
            .map(|h| unsafe { GetDpiForWindow(h.0) })
            .unwrap_or(0);
        Err(format!(
            "frame={frame} label={label} dpi={dpi} actual={:?} expected={expected:?}",
            last.map(|s| s.size)
        ))
    }
    unsafe extern "system" fn enum_child(hwnd: HWND, param: LPARAM) -> BOOL {
        let mut class = [0u16; 128];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let name = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if name == "Chrome_WidgetWin_0" || name == "Chrome_WidgetWin_1" {
            *(param as *mut bool) = true;
            return 0;
        }
        1
    }
    fn webview_ready(w: &tauri::WebviewWindow) -> bool {
        let Ok(h) = w.hwnd() else { return false };
        let mut found = false;
        unsafe {
            EnumChildWindows(h.0, Some(enum_child), &mut found as *mut bool as LPARAM);
        }
        found
    }
    fn check(failures: &mut Vec<String>, label: &str, result: Result<(), String>) {
        CHECKS.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = result {
            failures.push(format!("{label}: {e}"));
        }
    }
    let mut failures = Vec::new();
    for w in [first, second] {
        if let Err(e) = w.show() {
            failures.push(format!("show {}: {e}", w.label()));
        }
    }
    let ready_deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < ready_deadline && !(webview_ready(first) && webview_ready(second)) {
        std::thread::sleep(Duration::from_millis(100));
    }
    for w in [first, second] {
        CHECKS.fetch_add(1, Ordering::Relaxed);
        if !webview_ready(w) {
            failures.push(format!(
                "label={} frame=unknown: real WebView2 child HWND not ready",
                w.label()
            ));
        }
    }

    let frames = ["native", "thin", "full"];
    let targets = [(first, second), (second, first)];
    let run_started = Instant::now();
    'targets: for (active, other) in targets {
        for frame in frames {
            if run_started.elapsed() >= RUN_BUDGET {
                failures
                    .push("run budget exceeded 120 seconds; remaining size cases not run".into());
                break 'targets;
            }
            let fw = active.clone();
            let f = frame.to_owned();
            check(
                &mut failures,
                &format!("set frame {} {frame}", active.label()),
                platform::set_window_frame(&fw, &f).map_err(|e| format!("AppError: {e}")),
            );
            for expected in [
                tauri::PhysicalSize::new(900, 650),
                tauri::PhysicalSize::new(1000, 700),
            ] {
                let before_other = state(other);
                if let Err(error) = &before_other {
                    check(
                        &mut failures,
                        &format!("read other before {} {frame}", other.label()),
                        Err(error.clone()),
                    );
                }
                let before = state(active);
                if let Err(error) = &before {
                    check(
                        &mut failures,
                        &format!("read target before {} {frame}", active.label()),
                        Err(error.clone()),
                    );
                }
                check(
                    &mut failures,
                    &format!("outer size {} {frame} {expected:?}", active.label()),
                    outer(active, expected).and_then(|_| {
                        wait_size(active, expected, active.label(), frame, QUICK_WAIT).map(|_| ())
                    }),
                );
                for _ in 0..3 {
                    check(
                        &mut failures,
                        &format!("stable repeated size {} {frame}", active.label()),
                        outer(active, expected).and_then(|_| {
                            wait_size(active, expected, active.label(), frame, QUICK_WAIT)
                                .map(|_| ())
                        }),
                    );
                }
                match state(active) {
                    Ok(saved) => check(
                        &mut failures,
                        &format!("snapshot outer_size roundtrip {} {frame}", active.label()),
                        outer(active, saved.size).and_then(|_| {
                            wait_size(active, saved.size, active.label(), frame, QUICK_WAIT)
                                .map(|_| ())
                        }),
                    ),
                    Err(error) => check(
                        &mut failures,
                        &format!("snapshot outer_size read {} {frame}", active.label()),
                        Err(error),
                    ),
                }
                let after = state(active);
                if let Err(error) = &after {
                    check(
                        &mut failures,
                        &format!("read target after {} {frame}", active.label()),
                        Err(error.clone()),
                    );
                }
                if let (Ok(before), Ok(after)) = (before, after) {
                    check(
                        &mut failures,
                        &format!(
                            "position and visibility unchanged {} {frame}",
                            active.label()
                        ),
                        if before.position == after.position && before.visible == after.visible {
                            Ok(())
                        } else {
                            Err(format!(
                                "actual={after:?} expected position/visibility from {before:?}"
                            ))
                        },
                    );
                }
                let after_other = state(other);
                if let Err(error) = &after_other {
                    check(
                        &mut failures,
                        &format!("read other after {} {frame}", other.label()),
                        Err(error.clone()),
                    );
                }
                if let (Ok(before), Ok(after)) = (before_other, after_other) {
                    check(
                        &mut failures,
                        &format!("other window bounds unchanged {} {frame}", other.label()),
                        if before == after {
                            Ok(())
                        } else {
                            Err(format!("actual={after:?} expected={before:?}"))
                        },
                    );
                }
            }
            // Tao's set_inner_size path unmaximizes; verify this in every frame/window pair.
            CHECKS.fetch_add(1, Ordering::Relaxed);
            if let Err(error) = active.maximize() {
                failures.push(format!(
                    "label={} frame={frame} maximize command failed: {error}",
                    active.label()
                ));
                continue;
            }
            let maximize_deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < maximize_deadline && !active.is_maximized().unwrap_or(false) {
                std::thread::sleep(Duration::from_millis(50));
            }
            CHECKS.fetch_add(1, Ordering::Relaxed);
            if !active.is_maximized().unwrap_or(false) {
                failures.push(format!(
                    "label={} frame={frame} actual=not-maximized expected=maximized",
                    active.label()
                ));
                continue;
            }
            let expected = tauri::PhysicalSize::new(900, 650);
            check(
                &mut failures,
                &format!(
                    "maximized resize restores exact bounds {} {frame}",
                    active.label()
                ),
                outer(active, expected).and_then(|_| {
                    wait_size(active, expected, active.label(), frame, QUICK_WAIT).map(|_| ())
                }),
            );
            CHECKS.fetch_add(1, Ordering::Relaxed);
            if active.is_maximized().unwrap_or(true) {
                failures.push(format!(
                    "label={} frame={frame} actual=maximized expected=restored",
                    active.label()
                ));
            }
            for invalid_size in [
                tauri::PhysicalSize::new(u32::MAX, 650),
                tauri::PhysicalSize::new(900, u32::MAX),
            ] {
                let before = state(active);
                if let Err(error) = &before {
                    check(
                        &mut failures,
                        &format!(
                            "invalid size pre-state read {} {frame} {invalid_size:?}",
                            active.label()
                        ),
                        Err(error.clone()),
                    );
                }

                let result = platform::set_window_outer_size(active, invalid_size);
                check(
                    &mut failures,
                    &format!(
                        "invalid size rejected {} {frame} {invalid_size:?}",
                        active.label()
                    ),
                    match result {
                        Err(error::AppError::InvalidArgument(_)) => Ok(()),
                        other => Err(format!(
                            "expected AppError::InvalidArgument, actual={other:?}"
                        )),
                    },
                );

                let after = state(active);
                if let Err(error) = &after {
                    check(
                        &mut failures,
                        &format!(
                            "invalid size post-state read {} {frame} {invalid_size:?}",
                            active.label()
                        ),
                        Err(error.clone()),
                    );
                }
                if let (Ok(before), Ok(after)) = (before, after) {
                    check(
                        &mut failures,
                        &format!(
                            "invalid size preserves bounds {} {frame} {invalid_size:?}",
                            active.label()
                        ),
                        if before == after {
                            Ok(())
                        } else {
                            Err(format!("actual={after:?} expected={before:?}"))
                        },
                    );
                }
            }
        }
    }
    failures
}
