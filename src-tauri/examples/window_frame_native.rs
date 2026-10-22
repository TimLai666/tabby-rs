use std::sync::mpsc::sync_channel;
use std::time::Duration;

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

#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("window_frame_native: AppKit checks require macOS; no native checks ran");
}

#[cfg(target_os = "macos")]
fn main() {
    use tauri::WebviewUrl;

    let isolated_data = match tempfile::tempdir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("window_frame_native: cannot create isolated data directory: {error}");
            std::process::exit(1);
        }
    };
    let watchdog_data = isolated_data.path().to_path_buf();
    let timeout_data = watchdog_data.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("window_frame_native watchdog: timed out after 180 seconds");
        let _ = std::fs::remove_dir_all(timeout_data);
        std::process::exit(1);
    });

    let mut context = tauri::generate_context!();
    let Some(main_config) = context.config().app.windows.first().cloned() else {
        let _ = isolated_data.close();
        eprintln!("window_frame_native: generated config has no primary window");
        std::process::exit(1);
    };
    context.config_mut().identifier =
        format!("io.tabbyrs.tests.window-frame.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let (completed, completion) = sync_channel(1);
    let app_result = tauri::Builder::default()
        .setup(move |app| {
            let blank = WebviewUrl::External("about:blank".parse()?);
            let mut first_config = main_config.clone();
            first_config.label = "frameMain".into();
            first_config.url = blank.clone();
            first_config.title = "Native frame test".into();
            first_config.visible = false;
            let first = tauri::WebviewWindowBuilder::from_config(app, &first_config)?
                .incognito(true)
                .data_directory(isolated_data.path().to_path_buf())
                .build()?;

            let mut second_config = main_config.clone();
            second_config.label = "frameSecondary".into();
            second_config.url = blank;
            second_config.title = "Native frame test secondary".into();
            second_config.visible = false;
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
                    eprintln!("window_frame_native failure: {failure}");
                }
                let exit_code = if failures.is_empty() { 0 } else { 1 };
                eprintln!(
                    "window_frame_native completed: checks={} failures={}",
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
            eprintln!("window_frame_native startup failure: {error}");
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

#[cfg(target_os = "macos")]
static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "macos")]
fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSWindow;

    const TIMEOUT: Duration = Duration::from_secs(10);
    const TITLED: u64 = 1 << 0;
    const CLOSABLE: u64 = 1 << 1;
    const MINIATURIZABLE: u64 = 1 << 2;
    const ZOOMABLE: u64 = 1 << 3;
    const FULL_SIZE_CONTENT: u64 = 1 << 15;
    const HIDDEN_TITLE: isize = 1;

    #[derive(Clone, Debug, PartialEq)]
    struct Snapshot {
        title_visibility: isize,
        style_mask: u64,
        frame_rect: (f64, f64, f64, f64),
        first_responder: usize,
        titlebar_transparent: bool,
        alpha: f64,
        opaque: bool,
        close_button: bool,
        minimize_button: bool,
        zoom_button: bool,
    }

    fn on_main<T: Send + 'static>(
        window: &tauri::WebviewWindow,
        task: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = sync_channel(1);
        window
            .run_on_main_thread(move || {
                let _ = tx.send(task());
            })
            .map_err(|error| error.to_string())?;
        rx.recv_timeout(TIMEOUT)
            .map_err(|error| format!("outer main-thread wait error: {error}"))
    }

    fn snapshot(window: &tauri::WebviewWindow) -> Result<Snapshot, String> {
        on_main(window, {
            let window = window.clone();
            move || {
                let raw = window.ns_window().map_err(|error| error.to_string())?;
                let native: &NSWindow = unsafe { &*raw.cast() };
                let title_visibility: isize = unsafe { objc2::msg_send![native, titleVisibility] };
                let style_mask: u64 = unsafe { objc2::msg_send![native, styleMask] };
                let rect: objc2_foundation::NSRect = unsafe { objc2::msg_send![native, frame] };
                let responder: *mut AnyObject = unsafe { objc2::msg_send![native, firstResponder] };
                let titlebar_transparent: bool =
                    unsafe { objc2::msg_send![native, titlebarAppearsTransparent] };
                let alpha: f64 = unsafe { objc2::msg_send![native, alphaValue] };
                let opaque: bool = unsafe { objc2::msg_send![native, isOpaque] };
                let has_button = |kind: isize| {
                    let button: *mut AnyObject =
                        unsafe { objc2::msg_send![native, standardWindowButton: kind] };
                    !button.is_null()
                };
                Ok::<_, String>(Snapshot {
                    title_visibility,
                    style_mask,
                    frame_rect: (
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    ),
                    first_responder: responder as usize,
                    titlebar_transparent,
                    alpha,
                    opaque,
                    close_button: has_button(0),
                    minimize_button: has_button(1),
                    zoom_button: has_button(2),
                })
            }
        })?
    }

    fn issue(
        task: impl FnOnce() -> Result<(), error::AppError> + Send + 'static,
    ) -> Result<Result<(), error::AppError>, String> {
        let (tx, rx) = sync_channel(1);
        std::thread::spawn(move || {
            let _ = tx.send(task());
        });
        rx.recv_timeout(TIMEOUT)
            .map_err(|error| format!("outer operation wait error: {error}"))
    }

    fn apply_frame(window: &tauri::WebviewWindow, frame: &str) -> Result<(), String> {
        let window = window.clone();
        let frame = frame.to_owned();
        issue(move || platform::set_window_frame(&window, &frame))
            .map_err(|wait_error| wait_error)?
            .map_err(|app_error| format!("inner AppError: {app_error:?}"))
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

    let opacity_window = first.clone();
    let opacity = issue(move || platform::set_window_opacity(&opacity_window, 0.63));
    check!(
        "set baseline opacity",
        matches!(opacity, Ok(Ok(()))),
        format!("{opacity:?}")
    );
    let initial_first = capture!("initial main", first);
    let initial_second = capture!("initial secondary", second);
    check!(
        "initial main readback",
        initial_first.is_some(),
        "native snapshot must succeed"
    );
    check!(
        "initial secondary readback",
        initial_second.is_some(),
        "native snapshot must succeed"
    );

    if let (Some(baseline), Some(untouched)) = (initial_first.clone(), initial_second.clone()) {
        for frame in ["native", "thin", "full", "native", "full", "thin"] {
            CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let result = apply_frame(first, frame);
            check!(
                "apply frame",
                result.is_ok(),
                format!("{frame}: {result:?}")
            );
            if let Some(state) = capture!("main after frame", first) {
                let native = frame == "native";
                check!(
                    "frame title visibility",
                    state.title_visibility == if native { 0 } else { HIDDEN_TITLE },
                    format!(
                        "{frame} expected titleVisibility {}, got {}",
                        if native { 0 } else { HIDDEN_TITLE },
                        state.title_visibility
                    )
                );
                check!(
                    "frame content style",
                    (state.style_mask & FULL_SIZE_CONTENT != 0) == !native,
                    format!("{frame} styleMask={:#x}", state.style_mask)
                );
                check!(
                    "frame titlebar transparency",
                    state.titlebar_transparent == !native,
                    format!(
                        "{frame} titlebarAppearsTransparent={}",
                        state.titlebar_transparent
                    )
                );
                check!(
                    "titlebar and controls retained",
                    state.style_mask & (TITLED | CLOSABLE | MINIATURIZABLE | ZOOMABLE)
                        == (TITLED | CLOSABLE | MINIATURIZABLE | ZOOMABLE)
                        && state.close_button
                        && state.minimize_button
                        && state.zoom_button,
                    format!(
                        "{frame} styleMask={:#x}, buttons close={} minimize={} zoom={}",
                        state.style_mask,
                        state.close_button,
                        state.minimize_button,
                        state.zoom_button
                    )
                );
                check!(
                    "frame preserves opacity",
                    (state.alpha - baseline.alpha).abs() <= 0.01 && state.opaque == baseline.opaque,
                    format!(
                        "baseline alpha/opaque={}/{}, after={}/{}",
                        baseline.alpha, baseline.opaque, state.alpha, state.opaque
                    )
                );
                check!(
                    "frame preserves outer bounds",
                    [
                        state.frame_rect.0,
                        state.frame_rect.1,
                        state.frame_rect.2,
                        state.frame_rect.3
                    ]
                    .into_iter()
                    .zip([
                        baseline.frame_rect.0,
                        baseline.frame_rect.1,
                        baseline.frame_rect.2,
                        baseline.frame_rect.3
                    ])
                    .all(|(actual, expected)| (actual - expected).abs() <= 1.0),
                    format!(
                        "baseline rect={:?}, after={:?}",
                        baseline.frame_rect, state.frame_rect
                    )
                );
                check!(
                    "frame preserves existing first responder",
                    state.first_responder != 0 && state.first_responder == baseline.first_responder,
                    format!(
                        "baseline responder={:#x}, after={:#x}",
                        baseline.first_responder, state.first_responder
                    )
                );
            }
            if let Some(state) = capture!("secondary isolation", second) {
                check!(
                    "other window unchanged",
                    state == untouched,
                    format!("expected {untouched:?}, got {state:?} after main frame {frame}")
                );
            }
        }

        let before_invalid = capture!("before invalid frame", first);
        CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let invalid = {
            let window = first.clone();
            issue(move || platform::set_window_frame(&window, "invalid"))
        };
        check!(
            "invalid frame AppError",
            matches!(&invalid, Ok(Err(error::AppError::InvalidArgument(_)))),
            format!("expected inner InvalidArgument, got {invalid:?}")
        );
        if let (Some(before), Some(after)) =
            (before_invalid, capture!("after invalid frame", first))
        {
            check!(
                "invalid frame leaves state unchanged",
                before == after,
                format!("before={before:?}, after={after:?}")
            );
        }
    }

    let close = {
        let window = second.clone();
        let (tx, rx) = sync_channel(1);
        let scheduled = second.run_on_main_thread(move || {
            let _ = tx.send(window.destroy().map_err(|e| e.to_string()));
        });
        scheduled.map_err(|error| error.to_string()).and_then(|_| {
            rx.recv_timeout(TIMEOUT)
                .map_err(|error| format!("outer destroy wait error: {error}"))?
        })
    };
    check!("destroy secondary", close.is_ok(), format!("{close:?}"));
    let closed = {
        let window = second.clone();
        issue(move || platform::set_window_frame(&window, "thin"))
    };
    eprintln!("window_frame_native closed-window outcome: {closed:?}");
    check!(
        "closed-window reports inner AppError",
        matches!(&closed, Ok(Err(_))),
        format!("outer wait must succeed and native call must return AppError; got {closed:?}")
    );

    failures
}
