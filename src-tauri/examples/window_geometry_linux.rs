//! Real Linux Tauri/Wry startup geometry regression; not full desktop acceptance.
#[cfg(target_os = "linux")]
mod error {
    #[derive(Debug)]
    pub enum AppError {
        Io(String),
    }
    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(message) => f.write_str(message),
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_content_size_linux.rs"]
mod content_size;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_geometry_linux.rs"]
mod geometry;
#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;

#[cfg(not(target_os = "linux"))]
fn main() {
    println!(
        "window_geometry_linux: Linux Tauri/Wry geometry regression; skipped on this platform"
    );
}

#[cfg(target_os = "linux")]
fn main() {
    use gtk::prelude::*;
    use std::{
        sync::{
            atomic::{AtomicI32, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };

    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: cargo run --locked --example window_geometry_linux [--stock-control]\nChecks Linux Tauri/Wry prepared startup geometry, content minima, frame modes, and initialization behavior.");
        return;
    }
    let stock_control = args == ["--stock-control"];
    assert!(
        args.is_empty() || stock_control,
        "unknown arguments; use --help"
    );
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("window_geometry_linux watchdog");
        std::process::exit(2);
    });

    let owned = tempfile::tempdir().expect("isolated fixture directory");
    let data_root = owned.path().to_path_buf();
    let mut config: tauri::utils::config::Config =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("fixture config");
    let before = serde_json::to_value(&config).expect("serialize original fixture config");
    if !stock_control {
        window_geometry::prepare_startup(&mut config);
    }
    let main_config = config
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .expect("fixture main window")
        .clone();
    let mut expected_config =
        serde_json::from_value::<tauri::utils::config::Config>(before.clone())
            .expect("clone fixture config");
    if !stock_control {
        if let Some(window) = expected_config
            .app
            .windows
            .iter_mut()
            .find(|w| w.label == "main")
        {
            window.width = 800.0;
            window.height = 600.0;
            window.min_width = Some(400.0);
            window.min_height = Some(300.0);
        }
    }
    let expected_config_value =
        serde_json::to_value(expected_config).expect("serialize expected config");
    let actual_config_value = serde_json::to_value(&config).expect("serialize prepared config");
    let mut checks = 1usize;
    let mut initial_failures = usize::from(actual_config_value != expected_config_value);
    let expected_geometry = (800.0, 600.0, 400.0, 300.0);
    let actual_geometry = (
        main_config.width,
        main_config.height,
        main_config.min_width.unwrap_or_default(),
        main_config.min_height.unwrap_or_default(),
    );
    checks += 1;
    initial_failures += usize::from(actual_geometry != expected_geometry);
    println!("geometry phase=prepared-config window=main actual={actual_geometry:?} expected={expected_geometry:?}; config_equal={}", actual_config_value == expected_config_value);

    let mut context = tauri::generate_context!();
    context.config_mut().identifier = format!("io.tabbyrs.tests.geometry.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let outcome = Arc::new(AtomicI32::new(2));
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            let mut primary = main_config.clone();
            primary.label = "geometry-main".into();
            primary.url = tauri::WebviewUrl::External("about:blank".parse()?);
            primary.incognito = true;
            primary.data_directory = Some(data_root.join("primary"));
            let main = tauri::WebviewWindowBuilder::from_config(app, &primary)?
                .position(73.0, 130.0)
                .build()?;
            let (width, height, min_width, min_height) = if stock_control {
                // Match both original Linux constructors, not only the primary config.
                (1100.0, 720.0, 640.0, 480.0)
            } else {
                let (width, height) = window_geometry::DEFAULT_WINDOW_SIZE;
                let (min_width, min_height) = window_geometry::MIN_WINDOW_SIZE;
                (width, height, min_width, min_height)
            };
            let secondary = tauri::WebviewWindowBuilder::new(
                app,
                "geometry-secondary",
                tauri::WebviewUrl::External("about:blank".parse()?),
            )
            .inner_size(width, height)
            .min_inner_size(min_width, min_height)
            .position(-80.0, -90.0)
            .transparent(true)
            .decorations(false)
            .visible(false)
            .incognito(true)
            .data_directory(data_root.join("secondary"))
            .build()?;
            let windows = vec![main, secondary];
            let checks = Arc::new(AtomicUsize::new(checks));
            let failures = Arc::new(AtomicUsize::new(initial_failures));
            let read_size = move |window: &tauri::WebviewWindow| -> Result<tauri::PhysicalSize<u32>, String> {
                content_size::window_content_size(window).map_err(|e| e.to_string())
            };
            let verify_size = {
                let checks = checks.clone();
                let failures = failures.clone();
                move |phase: &str, window: &tauri::WebviewWindow, logical: (u32, u32)| {
                    checks.fetch_add(1, Ordering::SeqCst);
                    let scale = window.gtk_window().map(|g| g.scale_factor() as u32).unwrap_or(1);
                    let expected = tauri::PhysicalSize::new(logical.0 * scale, logical.1 * scale);
                    let actual = read_size(window);
                    let passed = actual.as_ref().is_ok_and(|size| *size == expected);
                    if !passed { failures.fetch_add(1, Ordering::SeqCst); }
                    println!("geometry phase={phase} window={} actual={actual:?} expected={expected:?}", window.label());
                }
            };
            let verify_state = {
                let checks = checks.clone();
                let failures = failures.clone();
                move |phase: &str, window: &tauri::WebviewWindow, visible: bool, decorated: bool| {
                    checks.fetch_add(1, Ordering::SeqCst);
                    let actual = window
                        .gtk_window()
                        .map(|gtk| (gtk.is_visible(), gtk.is_decorated()))
                        .map_err(|e| e.to_string());
                    let expected = Ok((visible, decorated));
                    let passed = actual == expected;
                    if !passed { failures.fetch_add(1, Ordering::SeqCst); }
                    println!("geometry phase={phase} window={} actual={actual:?} expected={expected:?}", window.label());
                }
            };
            let handle = app.handle().clone();
            let mut phase = 0;
            let mut ticks = 0;
            let mut repeated_snapshot = None;
            let (tx, rx) = mpsc::channel();
            gtk::glib::timeout_add_local(Duration::from_millis(350), move || {
                ticks += 1;
                if ticks > 70 {
                    checks.fetch_add(1, Ordering::SeqCst);
                    let n = checks.load(Ordering::SeqCst);
                    let f = failures.load(Ordering::SeqCst) + 1;
                    eprintln!("window_geometry_linux completed: checks={n} failures={f} phase={phase} timeout=true");
                    result.store(1, Ordering::SeqCst);
                    handle.exit(1);
                    return gtk::glib::ControlFlow::Break;
                }
                let native = &windows[0];
                let frameless = &windows[1];
                match phase {
                    0 => {
                        for (i, window) in windows.iter().enumerate() {
                            verify_state("constructed-hidden", window, false, i == 0);
                            verify_size("constructed-hidden", window, (800, 600));
                            let position = window
                                .outer_position()
                                .map(|p| (p.x, p.y))
                                .map_err(|e| e.to_string());
                            if !stock_control {
                                checks.fetch_add(1, Ordering::SeqCst);
                                if let Err(e) = geometry::initialize(window) {
                                    eprintln!("geometry phase=initialize window={} actual=Err({e}) expected=Ok", window.label());
                                    failures.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                            let after = window
                                .outer_position()
                                .map(|p| (p.x, p.y))
                                .map_err(|e| e.to_string());
                            checks.fetch_add(1, Ordering::SeqCst);
                            if position != after { failures.fetch_add(1, Ordering::SeqCst); }
                            println!("geometry phase=initialize-position window={} actual={after:?} expected={position:?}", window.label());
                            verify_size("after-initialize-hidden", window, (800, 600));
                            verify_state("after-initialize-hidden", window, false, i == 0);
                        }
                    }
                    1 => { for window in &windows { window.show().unwrap(); } }
                    2 => {
                        for (i, window) in windows.iter().enumerate() {
                            verify_state("mapped-native-frameless", window, true, i == 0);
                            verify_size("mapped", window, (800, 600));
                            window.set_size(tauri::LogicalSize::new(1.0, 1.0)).unwrap();
                        }
                    }
                    3 => {
                        for window in &windows {
                            verify_size("minimum-content", window, (400, 300));
                        }
                        native.set_size(tauri::LogicalSize::new(900.0, 650.0)).unwrap();
                    }
                    4 => {
                        verify_size("isolated-resize-primary", native, (900, 650));
                        verify_size("isolated-resize-secondary", frameless, (400, 300));
                        let pos = native
                            .outer_position()
                            .map(|p| (p.x, p.y))
                            .map_err(|e| e.to_string());
                        let vis = native.gtk_window().map(|g| g.is_visible()).map_err(|e| e.to_string());
                        let deco = native.gtk_window().map(|g| g.is_decorated()).map_err(|e| e.to_string());
                        if !stock_control {
                            checks.fetch_add(1, Ordering::SeqCst);
                            if let Err(e) = geometry::initialize(native) {
                                eprintln!("geometry phase=repeated-init actual=Err({e}) expected=Ok");
                                failures.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        repeated_snapshot = Some((pos, vis, deco));
                    }
                    5 => {
                        let actual = (
                            native
                                .outer_position()
                                .map(|p| (p.x, p.y))
                                .map_err(|e| e.to_string()),
                            native.gtk_window().map(|g| g.is_visible()).map_err(|e| e.to_string()),
                            native.gtk_window().map(|g| g.is_decorated()).map_err(|e| e.to_string()),
                        );
                        checks.fetch_add(1, Ordering::SeqCst);
                        if repeated_snapshot.as_ref() != Some(&actual) { failures.fetch_add(1, Ordering::SeqCst); }
                        println!("geometry phase=repeated-init-settled window={} actual={actual:?} expected={repeated_snapshot:?}", native.label());
                        verify_size("repeated-init-size", native, (900, 650));
                        verify_size("repeated-init-isolation", frameless, (400, 300));
                        native.destroy().unwrap();
                        if !stock_control {
                            let dead = native.clone();
                            let tx = tx.clone();
                            std::thread::spawn(move || tx.send(geometry::initialize(&dead).map_err(|e| e.to_string())).unwrap());
                        }
                    }
                    6 => {
                        if stock_control {
                            verify_size("surviving-window", frameless, (400, 300));
                            frameless.destroy().unwrap();
                            let n = checks.load(Ordering::SeqCst);
                            let f = failures.load(Ordering::SeqCst);
                            println!("window_geometry_linux completed: checks={n} failures={f}");
                            result.store(i32::from(f != 0), Ordering::SeqCst);
                            handle.exit(i32::from(f != 0));
                            return gtk::glib::ControlFlow::Break;
                        }
                        if let Ok(actual) = rx.try_recv() {
                            checks.fetch_add(1, Ordering::SeqCst);
                            if actual.is_ok() { failures.fetch_add(1, Ordering::SeqCst); }
                            println!("geometry phase=destroyed-init window=geometry-main actual={actual:?} expected=Err");
                            verify_size("surviving-window", frameless, (400, 300));
                            frameless.destroy().unwrap();
                            let n = checks.load(Ordering::SeqCst);
                            let f = failures.load(Ordering::SeqCst);
                            println!("window_geometry_linux completed: checks={n} failures={f}");
                            result.store(i32::from(f != 0), Ordering::SeqCst);
                            handle.exit(i32::from(f != 0));
                            return gtk::glib::ControlFlow::Break;
                        }
                        phase -= 1;
                    }
                    _ => {}
                }
                phase += 1;
                gtk::glib::ControlFlow::Continue
            });
            Ok(())
        })
        .build(context)
        .expect("build real Tauri/Wry geometry fixture");
    app.run_return(|_, _| {});
    let code = outcome.load(Ordering::SeqCst);
    drop(owned);
    std::process::exit(code);
}
