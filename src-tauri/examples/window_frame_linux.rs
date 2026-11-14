//! Real Linux Tauri/Wry window frame regression; not full desktop acceptance.
#[cfg(target_os = "linux")]
mod desktop {
    #[allow(dead_code)]
    #[derive(Clone)]
    pub struct VibrancyOptions {
        pub enabled: bool,
        pub effect: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(Debug)]
    pub struct WindowCaptionState;
}

#[cfg(target_os = "linux")]
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
                Self::InvalidArgument(message) | Self::Io(message) | Self::Unsupported(message) => {
                    f.write_str(message)
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/mod.rs"]
mod platform;

#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;

#[cfg(not(target_os = "linux"))]
fn main() {
    println!(
        "window_frame_linux: Linux Tauri/Wry window frame regression; skipped on this platform"
    );
}

#[cfg(target_os = "linux")]
static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "linux")]
fn record(failures: &mut Vec<String>, label: &str, passed: bool, details: String) {
    CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if !passed {
        failures.push(format!("{label}: {details}"));
    }
}

#[cfg(target_os = "linux")]
fn decorated(window: &tauri::WebviewWindow) -> Result<bool, String> {
    use gtk::prelude::*;

    window
        .gtk_window()
        .map(|native| native.is_decorated())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn check_decorated(
    failures: &mut Vec<String>,
    label: &str,
    window: &tauri::WebviewWindow,
    expected: bool,
) {
    let actual = decorated(window);
    record(
        failures,
        &format!("{label} decoration"),
        matches!(&actual, Ok(value) if *value == expected),
        format!("expected decorated={expected}, got {actual:?}"),
    );
}

#[cfg(target_os = "linux")]
fn hidden_inner_size(window: &tauri::WebviewWindow) -> Result<tauri::PhysicalSize<u32>, String> {
    platform::window_bounds(window)
        .map(|(_, size)| size)
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    let owned = tempfile::tempdir().expect("isolated fixture directory");
    let data_root = owned.path().to_path_buf();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("window_frame_linux watchdog");
        std::process::exit(2);
    });

    let mut context = tauri::generate_context!();
    let Some(main_config) = context.config().app.windows.first().cloned() else {
        eprintln!("window_frame_linux: generated config has no primary window");
        std::process::exit(1);
    };
    context.config_mut().identifier =
        format!("io.tabbyrs.tests.window-frame-linux.{}", std::process::id());
    context.config_mut().app.windows.clear();

    let outcome = Arc::new(std::sync::atomic::AtomicI32::new(2));
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            let blank = tauri::WebviewUrl::External("about:blank".parse()?);
            let mut windows = Vec::new();
            for (label, title) in [
                ("frame-main", "Linux frame test"),
                ("frame-secondary", "Linux frame test secondary"),
            ] {
                let mut window_config = main_config.clone();
                window_config.label = label.into();
                window_config.url = blank.clone();
                window_config.title = title.into();
                window_config.visible = false;
                let window = tauri::WebviewWindowBuilder::from_config(app, &window_config)?
                    .incognito(true)
                    .data_directory(data_root.join(label))
                    .build()?;
                windows.push(window);
            }
            let first = windows[0].clone();
            let second = windows[1].clone();

            let handle = app.handle().clone();
            let mut failures: Vec<String> = Vec::new();
            let mut phase = 0u32;
            let mut ticks = 0u32;
            let mut initial_size: Option<tauri::PhysicalSize<u32>> = None;
            let mut invalid_result: Option<Result<(), error::AppError>> = None;
            gtk::glib::timeout_add_local(Duration::from_millis(300), move || {
                ticks += 1;
                if ticks > 100 {
                    record(
                        &mut failures,
                        "completion",
                        false,
                        format!("timed out in phase {phase}"),
                    );
                    eprintln!(
                        "window_frame_linux completed: checks={} failures={}",
                        CHECKS.load(Ordering::Relaxed),
                        failures.len()
                    );
                    for failure in &failures {
                        eprintln!("window_frame_linux failure: {failure}");
                    }
                    result.store(1, Ordering::SeqCst);
                    handle.exit(1);
                    return gtk::glib::ControlFlow::Break;
                }
                match phase {
                    0 => {
                        let fresh = decorated(&first);
                        record(
                            &mut failures,
                            "fresh window decoration",
                            matches!(&fresh, Ok(true)),
                            format!("expected decorated=true, got {fresh:?}"),
                        );
                        check_decorated(&mut failures, "fresh secondary", &second, true);
                        match hidden_inner_size(&first) {
                            Ok(size) => {
                                initial_size = Some(size);
                                record(
                                    &mut failures,
                                    "hidden inner size",
                                    size.width > 0 && size.height > 0,
                                    format!("{size:?}"),
                                );
                            }
                            Err(error) => {
                                record(&mut failures, "hidden inner size", false, error);
                            }
                        }
                        let applied = platform::set_window_frame(&first, "thin");
                        record(
                            &mut failures,
                            "apply thin",
                            applied.is_ok(),
                            format!("{applied:?}"),
                        );
                    }
                    1 => {
                        check_decorated(&mut failures, "thin", &first, false);
                        check_decorated(&mut failures, "thin secondary", &second, true);
                        let applied = platform::set_window_frame(&first, "native");
                        record(
                            &mut failures,
                            "apply native",
                            applied.is_ok(),
                            format!("{applied:?}"),
                        );
                    }
                    2 => {
                        check_decorated(&mut failures, "native", &first, true);
                        check_decorated(&mut failures, "native secondary", &second, true);
                        let applied = platform::set_window_frame(&first, "full");
                        record(
                            &mut failures,
                            "apply full",
                            applied.is_ok(),
                            format!("{applied:?}"),
                        );
                    }
                    3 => {
                        check_decorated(&mut failures, "full", &first, false);
                        check_decorated(&mut failures, "full secondary", &second, true);
                        let applied = platform::set_window_frame(&first, "native");
                        record(
                            &mut failures,
                            "apply native again",
                            applied.is_ok(),
                            format!("{applied:?}"),
                        );
                    }
                    4 => {
                        check_decorated(&mut failures, "native again", &first, true);
                        check_decorated(&mut failures, "native again secondary", &second, true);
                        let after = hidden_inner_size(&first);
                        let same_size = match (initial_size, &after) {
                            (Some(before), Ok(actual)) => before == *actual,
                            _ => false,
                        };
                        record(
                            &mut failures,
                            "frame change preserves hidden inner size",
                            same_size,
                            format!("before={initial_size:?}, after={after:?}"),
                        );
                        invalid_result = Some(platform::set_window_frame(&first, "invalid"));
                    }
                    5 => {
                        record(
                            &mut failures,
                            "invalid frame AppError",
                            matches!(
                                &invalid_result,
                                Some(Err(error::AppError::InvalidArgument(_)))
                            ),
                            format!("expected inner InvalidArgument, got {invalid_result:?}"),
                        );
                        check_decorated(&mut failures, "invalid frame", &first, true);
                        check_decorated(&mut failures, "invalid frame secondary", &second, true);
                        for window in &windows {
                            let _ = window.destroy();
                        }
                        let exit_code = i32::from(!failures.is_empty());
                        eprintln!(
                            "window_frame_linux completed: checks={} failures={}",
                            CHECKS.load(Ordering::Relaxed),
                            failures.len()
                        );
                        for failure in &failures {
                            eprintln!("window_frame_linux failure: {failure}");
                        }
                        result.store(exit_code, Ordering::SeqCst);
                        handle.exit(exit_code);
                        return gtk::glib::ControlFlow::Break;
                    }
                    _ => {}
                }
                phase += 1;
                gtk::glib::ControlFlow::Continue
            });
            Ok(())
        })
        .build(context)
        .expect("build real Tauri/Wry frame fixture");
    app.run_return(|_, _| {});
    let code = outcome.load(Ordering::SeqCst);
    drop(owned);
    std::process::exit(code);
}
