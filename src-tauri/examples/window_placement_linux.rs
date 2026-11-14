//! Real Linux Tauri/Wry window placement regression; not full desktop acceptance.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
mod error {
    #[derive(Debug)]
    pub enum AppError {
        InvalidArgument(String),
        InvalidData(String),
        Io(String),
        NotFound(String),
        PermissionDenied(String),
    }

    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidArgument(message)
                | Self::InvalidData(message)
                | Self::Io(message)
                | Self::NotFound(message)
                | Self::PermissionDenied(message) => f.write_str(message),
            }
        }
    }

    impl From<std::io::Error> for AppError {
        fn from(error: std::io::Error) -> Self {
            Self::Io(error.to_string())
        }
    }

    impl From<serde_json::Error> for AppError {
        fn from(error: serde_json::Error) -> Self {
            Self::InvalidData(error.to_string())
        }
    }
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/storage/atomic_file.rs"]
mod atomic_file;

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/storage/window_state.rs"]
mod window_state;

#[cfg(not(target_os = "linux"))]
fn main() {
    println!(
        "window_placement_linux: Linux Tauri/Wry window placement regression; skipped on this platform"
    );
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct Tally {
    checks: usize,
    failures: usize,
}

#[cfg(target_os = "linux")]
impl Tally {
    fn record(&mut self, label: &str, passed: bool, details: String) {
        self.checks += 1;
        if !passed {
            self.failures += 1;
        }
        println!("placement {label} passed={passed} {details}");
    }
}

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::{
        atomic::{AtomicI32, Ordering},
        Arc,
    };

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        eprintln!("window_placement_linux watchdog");
        std::process::exit(2);
    });

    let owned = tempfile::tempdir().expect("isolated fixture directory");
    let data_root = owned.path().to_path_buf();
    let state_path = data_root.join("window.json");

    let mut context = tauri::generate_context!();
    context.config_mut().identifier = format!("io.tabbyrs.tests.placement.{}", std::process::id());
    let main_config = context
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .expect("fixture main window")
        .clone();
    context.config_mut().app.windows.clear();

    let outcome = Arc::new(AtomicI32::new(1));
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            let mut primary = main_config.clone();
            primary.label = "placement-main".into();
            primary.url = tauri::WebviewUrl::External("about:blank".parse()?);
            primary.incognito = true;
            primary.data_directory = Some(data_root.join("primary"));
            let window = tauri::WebviewWindowBuilder::from_config(app, &primary)?
                .visible(false)
                .build()?;
            let tally = run_checks(&window, &state_path);
            let code = i32::from(tally.failures != 0);
            println!(
                "window_placement_linux completed: checks={} failures={}",
                tally.checks, tally.failures
            );
            result.store(code, Ordering::SeqCst);
            app.handle().exit(code);
            Ok(())
        })
        .build(context)
        .expect("build real Tauri/Wry placement fixture");
    app.run_return(|_, _| {});
    let code = outcome.load(Ordering::SeqCst);
    drop(owned);
    std::process::exit(code);
}

#[cfg(target_os = "linux")]
fn run_checks(window: &tauri::WebviewWindow, state_path: &std::path::Path) -> Tally {
    use window_state::{
        load_window_state, restored_placement, save_window_state, MonitorRect, Placement,
        WindowBoundaries, WindowStateFile,
    };

    let mut tally = Tally::default();
    let scale = window.scale_factor().expect("window scale factor");
    let x11 = std::env::var("GDK_BACKEND").is_ok_and(|value| value == "x11");
    let monitors = physical_monitors(window);
    tally.record(
        "monitors-present",
        !monitors.is_empty(),
        format!("count={}", monitors.len()),
    );
    let primary = monitors.first().copied().unwrap_or(MonitorRect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    });

    // Saved size on a hidden window: the saved rectangle already intersects the monitor.
    let saved_bounds = WindowBoundaries {
        x: 10.0,
        y: 20.0,
        width: 640.0,
        height: 480.0,
    };
    let state = WindowStateFile {
        window_boundaries: Some(saved_bounds),
        maximized: false,
    };
    save_window_state(state_path, &state).expect("save saved-size fixture");
    let loaded = load_window_state(state_path).expect("load saved-size fixture");
    let placement = restored_placement(&loaded, &monitors);
    let expected = Some(Placement {
        bounds: Some(saved_bounds),
        maximized: false,
    });
    tally.record(
        "saved-size-placement",
        placement == expected,
        format!("actual={placement:?} expected={expected:?}"),
    );
    if let Some(placement) = placement {
        check_scenario(window, &mut tally, "saved-size", &placement, x11);
    }

    // Off-screen rectangle: centered on the nearest monitor at its saved size.
    let state = WindowStateFile {
        window_boundaries: Some(WindowBoundaries {
            x: 5000.0,
            y: 100.0,
            width: 800.0,
            height: 600.0,
        }),
        maximized: false,
    };
    save_window_state(state_path, &state).expect("save centered fixture");
    let loaded = load_window_state(state_path).expect("load centered fixture");
    let placement = restored_placement(&loaded, &monitors);
    let centered = WindowBoundaries {
        x: primary.x + (primary.width - 800.0) / 2.0,
        y: primary.y + (primary.height - 600.0) / 2.0,
        width: 800.0,
        height: 600.0,
    };
    let expected = Some(Placement {
        bounds: Some(centered),
        maximized: false,
    });
    tally.record(
        "centered-placement",
        placement == expected,
        format!("actual={placement:?} expected={expected:?}"),
    );
    if let Some(placement) = placement {
        check_scenario(window, &mut tally, "centered", &placement, x11);
    }

    // Maximized state: the saved bounds are kept and the maximize request is applied.
    let maximized_bounds = WindowBoundaries {
        x: 100.0,
        y: 100.0,
        width: 700.0,
        height: 500.0,
    };
    let state = WindowStateFile {
        window_boundaries: Some(maximized_bounds),
        maximized: true,
    };
    save_window_state(state_path, &state).expect("save maximized fixture");
    let loaded = load_window_state(state_path).expect("load maximized fixture");
    let placement = restored_placement(&loaded, &monitors);
    let expected = Some(Placement {
        bounds: Some(maximized_bounds),
        maximized: true,
    });
    tally.record(
        "maximized-placement",
        placement == expected,
        format!("actual={placement:?} expected={expected:?}"),
    );
    if let Some(placement) = placement {
        apply(window, &placement).expect("apply maximized placement");
        window.show().expect("show maximized window");
        pump_gtk(std::time::Duration::from_millis(300));
        window.maximize().expect("maximize placement window");
        pump_gtk(std::time::Duration::from_millis(500));
        let maximized = window.is_maximized().expect("maximized state");
        tally.record("maximized-state", maximized, format!("actual={maximized}"));
        window.hide().expect("hide maximized window");
    }

    tally
}

#[cfg(target_os = "linux")]
fn check_scenario(
    window: &tauri::WebviewWindow,
    tally: &mut Tally,
    label: &str,
    placement: &window_state::Placement,
    x11: bool,
) {
    // Linux placement units are physical pixels (see window_persistence.rs).
    let bounds = placement.bounds.expect("placement with bounds");
    apply(window, placement).expect("apply placement");
    let expected_size =
        tauri::PhysicalSize::new(bounds.width.round() as u32, bounds.height.round() as u32);
    // Tao caches hidden sizes until the window is mapped (see AGENTS.md), so
    // the size is asserted after show().
    window.show().expect("show placement window");
    pump_gtk(std::time::Duration::from_millis(300));
    let shown = window.inner_size().expect("shown inner size");
    tally.record(
        &format!("{label}-shown-size"),
        shown == expected_size,
        format!("actual={shown:?} expected={expected_size:?}"),
    );
    if x11 {
        let expected_position =
            tauri::PhysicalPosition::new(bounds.x.round() as i32, bounds.y.round() as i32);
        let position = window.outer_position().expect("outer position");
        tally.record(
            &format!("{label}-position"),
            position == expected_position,
            format!("actual={position:?} expected={expected_position:?}"),
        );
    }
    window.hide().expect("hide placement window");
}

#[cfg(target_os = "linux")]
fn apply(window: &tauri::WebviewWindow, placement: &window_state::Placement) -> tauri::Result<()> {
    let bounds = placement.bounds.expect("placement with bounds");
    window.set_size(tauri::PhysicalSize::new(
        bounds.width.round() as u32,
        bounds.height.round() as u32,
    ))?;
    window.set_position(tauri::PhysicalPosition::new(
        bounds.x.round() as i32,
        bounds.y.round() as i32,
    ))
}

#[cfg(target_os = "linux")]
fn physical_monitors(window: &tauri::WebviewWindow) -> Vec<window_state::MonitorRect> {
    window
        .available_monitors()
        .expect("available monitors")
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            window_state::MonitorRect {
                x: f64::from(position.x),
                y: f64::from(position.y),
                width: f64::from(size.width),
                height: f64::from(size.height),
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn pump_gtk(duration: std::time::Duration) {
    let until = std::time::Instant::now() + duration;
    while std::time::Instant::now() < until {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
