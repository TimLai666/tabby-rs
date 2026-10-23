//! Real Linux Tauri/Wry content-size regression; not full desktop acceptance.
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
#[path = "../src/platform/window_bounds_linux.rs"]
mod bounds;

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_content_size_linux: Linux Tauri/Wry content-size regression; skipped on this platform");
}

#[cfg(target_os = "linux")]
fn main() {
    use gtk::prelude::*;
    use std::{sync::mpsc, time::Duration};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: cargo run --locked --manifest-path src-tauri/Cargo.toml --example window_content_size_linux [--stock-control]\nChecks real hidden/mapped Linux Tauri/Wry windows, hidden resizing, frame modes, scale conversion, worker reads, and destruction errors. --stock-control reads the original outer-size getter and must fail. This is not full desktop acceptance.\nMapped X11 checks require xwininfo and at least 1920x1080 logical desktop pixels (for example 3840x2160 physical pixels at scale 2).");
        return;
    }
    let control = args == ["--stock-control"];
    assert!(args.is_empty() || control, "unknown arguments; use --help");
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("window_content_size_linux watchdog");
        std::process::exit(2);
    });
    let owned = tempfile::tempdir().expect("isolated fixture directory");
    let data_root = owned.path().to_path_buf();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier =
        format!("io.tabbyrs.tests.content-size.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let outcome = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(2));
    let result = outcome.clone();
    let app = tauri::Builder::default().setup(move |app| {
        let mut windows = Vec::new();
        for (label, decorated, x) in [("native", true, 73.0), ("frameless", false, -80.0)] {
            let window = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::External("about:blank".parse()?))
                .inner_size(1100.0, 720.0).min_inner_size(640.0, 480.0)
                .position(x, 130.0).transparent(true).decorations(decorated)
                .visible(false).incognito(true).data_directory(data_root.join(label)).build()?;
            windows.push(window);
        }
        let read = move |window: &tauri::WebviewWindow| -> Result<tauri::PhysicalSize<u32>, String> {
            if control { window.outer_size().map_err(|e| e.to_string()) }
            else { bounds::window_bounds(window).map(|(_, size)| size).map_err(|e| e.to_string()) }
        };
        let verify = move |phase: &str, window: &tauri::WebviewWindow, width: u32, height: u32| {
            let expected = expected_size(window, width, height);
            let actual = read(window);
            let passed = expected
                .as_ref()
                .is_ok_and(|expected| actual.as_ref().is_ok_and(|size| size == expected));
            println!("content-size {} {phase} {} actual={actual:?} expected={expected:?}", if passed { "PASS" } else { "FAIL" }, window.label());
            passed
        };
        let mut failures = 0;
        for w in &windows { failures += usize::from(!verify("constructed-hidden", w, 1100, 720)); }
        let handle = app.handle().clone();
        let mut stage = 0;
        let (tx, rx) = mpsc::channel();
        let mut workers = 0;
        gtk::glib::timeout_add_local(Duration::from_millis(500), move || {
            match stage {
                0 => { for w in &windows { failures += usize::from(!verify("loaded-hidden", w, 1100, 720)); w.show().unwrap(); } }
                1 => { for w in &windows { failures += usize::from(!verify("mapped", w, 1100, 720)); w.set_size(tauri::LogicalSize::new(900, 650)).unwrap(); } }
                2 => { for w in &windows { failures += usize::from(!verify("resized", w, 900, 650)); w.hide().unwrap(); } }
                3 => { for w in &windows { failures += usize::from(!verify("hidden-after-map", w, 900, 650)); w.set_size(tauri::LogicalSize::new(810, 610)).unwrap(); } }
                4 => { for w in &windows { failures += usize::from(!verify("resized-hidden", w, 810, 610)); w.show().unwrap(); } }
                5 => {
                    for w in &windows {
                        failures += usize::from(!verify("remapped", w, 810, 610));
                        let w = w.clone(); let tx = tx.clone();
                        std::thread::spawn(move || { tx.send((w.label().to_owned(), read(&w))).unwrap(); });
                    }
                }
                6 => {
                    while let Ok((label, result)) = rx.try_recv() {
                        let window = windows.iter().find(|w| w.label() == label).unwrap();
                        let expected = expected_size(window, 810, 610);
                        if !expected
                            .as_ref()
                            .is_ok_and(|expected| result.as_ref().is_ok_and(|size| size == expected))
                        {
                            failures += 1;
                        }
                        println!("content-size worker {label} actual={result:?} expected={expected:?}");
                        workers += 1;
                    }
                    if workers != windows.len() { return gtk::glib::ControlFlow::Continue; }
                    windows[0].destroy().unwrap();
                }
                7 => {
                    let destroyed = read(&windows[0]);
                    println!("content-size destroyed-window actual={destroyed:?}");
                    failures += usize::from(destroyed.is_ok());
                    failures += usize::from(!verify("surviving-window", &windows[1], 810, 610));
                    println!("window_content_size_linux completed: checks={} failures={failures}", 8 * windows.len() + 2);
                    result.store(i32::from(failures != 0), std::sync::atomic::Ordering::SeqCst);
                    windows[1].destroy().unwrap();
                    handle.exit(i32::from(failures != 0));
                    return gtk::glib::ControlFlow::Break;
                }
                _ => unreachable!(),
            }
            stage += 1;
            gtk::glib::ControlFlow::Continue
        });
        Ok(())
    }).build(context).expect("build actual Wry fixture");
    app.run_return(|_, _| {});
    let code = outcome.load(std::sync::atomic::Ordering::SeqCst);
    drop(owned);
    std::process::exit(code);
}

#[cfg(target_os = "linux")]
fn expected_size(
    window: &tauri::WebviewWindow,
    width: u32,
    height: u32,
) -> Result<tauri::PhysicalSize<u32>, String> {
    use gtk::prelude::*;
    use std::process::Command;
    let gtk_window = window.gtk_window().map_err(|error| error.to_string())?;
    let scale = gtk_window.scale_factor().max(1) as i64;
    let requested = tauri::PhysicalSize::new(
        width
            .checked_mul(scale as u32)
            .ok_or_else(|| "requested width overflow".to_owned())?,
        height
            .checked_mul(scale as u32)
            .ok_or_else(|| "requested height overflow".to_owned())?,
    );
    if !gtk_window.is_mapped() {
        return Ok(requested);
    }
    let Some(gdk_window) = gtk_window.window() else {
        return Ok(requested);
    };
    let Ok(x11_window) = gdk_window.downcast::<gdkx11::X11Window>() else {
        return Ok(requested);
    };

    let id = x11_window.xid();
    let output = Command::new("xwininfo")
        .args(["-id", &format!("{id:#x}")])
        .output()
        .map_err(|error| format!("run xwininfo for X11 window {id:#x}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "xwininfo failed for X11 window {id:#x}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let x = parse_xwininfo_signed(&text, "Absolute upper-left X:")? as i64;
    let y = parse_xwininfo_signed(&text, "Absolute upper-left Y:")? as i64;
    let native_width = parse_xwininfo(&text, "Width:")?;
    let native_height = parse_xwininfo(&text, "Height:")?;
    if native_width != requested.width || native_height != requested.height {
        return Err(format!(
            "X11 client size {native_width}x{native_height} does not match requested physical size {}x{}",
            requested.width, requested.height
        ));
    }

    let logical_left = x.div_euclid(scale);
    let logical_top = y.div_euclid(scale);
    let right = x + i64::from(native_width);
    let bottom = y + i64::from(native_height);
    let logical_right = right.div_euclid(scale) + i64::from(right.rem_euclid(scale) != 0);
    let logical_bottom = bottom.div_euclid(scale) + i64::from(bottom.rem_euclid(scale) != 0);
    let physical_width = (logical_right - logical_left)
        .checked_mul(scale)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "expected physical width overflow".to_owned())?;
    let physical_height = (logical_bottom - logical_top)
        .checked_mul(scale)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "expected physical height overflow".to_owned())?;
    Ok(tauri::PhysicalSize::new(physical_width, physical_height))
}

#[cfg(target_os = "linux")]
fn parse_xwininfo(text: &str, key: &str) -> Result<u32, String> {
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|value| value.trim().parse().ok())
        })
        .ok_or_else(|| format!("missing or invalid {key} in xwininfo output: {text}"))
}

#[cfg(target_os = "linux")]
fn parse_xwininfo_signed(text: &str, key: &str) -> Result<i32, String> {
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|value| value.trim().parse().ok())
        })
        .ok_or_else(|| format!("missing or invalid {key} in xwininfo output: {text}"))
}
