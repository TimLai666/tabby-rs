//! Native Linux position regression using the real Tauri/Wry window and GTK/X11.
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_position_linux: skipped outside Linux");
}

#[cfg(target_os = "linux")]
#[path = "../src/platform/window_bounds_linux.rs"]
mod bounds;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_geometry_linux.rs"]
mod geometry;
#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;

#[cfg(target_os = "linux")]
mod error {
    #[derive(Debug, PartialEq, Eq)]
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
fn main() {
    use gtk::prelude::*;
    use std::{
        process::Command,
        sync::{
            atomic::{AtomicI32, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };

    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!(
            "Usage: window_position_linux [--stock-control]
\
Checks isolated Native, frameless, and custom Linux Wry client bounds.
\
With no arguments, runs the corrected adapter (exit 0 only when all checks pass).
\
  --stock-control  Run the old getter/setter and startup behavior; X11 must fail.
\
  -h, --help       Show this help. Unknown arguments exit 2.
\
X11 requires xwininfo, an EWMH window manager, and GTK scale 1 or 2.
\
Wayland runs compatibility checks only; it does not accept global positions.
\
Example: GDK_SCALE=2 cargo run --locked --example window_position_linux"
        );
        return;
    }
    let stock = args == ["--stock-control"];
    if !args.is_empty() && !stock {
        eprintln!("window_position_linux: unknown arguments: {args:?}");
        std::process::exit(2);
    }
    gtk::init().expect("initialize GTK display for backend detection");
    let x11_backend = gtk::gdk::Display::default()
        .is_some_and(|display| display.downcast::<gdkx11::X11Display>().is_ok());
    if !x11_backend {
        run_wayland_compatibility_probe();
        return;
    }
    let outcome = Arc::new(AtomicI32::new(2));
    let watchdog_outcome = outcome.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(45));
        eprintln!("window_position_linux completed: checks=0 failures=1 watchdog=true");
        watchdog_outcome.store(1, Ordering::SeqCst);
        std::process::exit(1);
    });

    let screen = Command::new("xwininfo")
        .arg("-root")
        .output()
        .expect("xwininfo -root");
    assert!(screen.status.success(), "xwininfo -root failed");
    let screen_text = String::from_utf8_lossy(&screen.stdout);
    let screen_width = parse_xwininfo(&screen_text, "Width:");
    let screen_height = parse_xwininfo(&screen_text, "Height:");
    let temporary = tempfile::tempdir().expect("private isolated webview data");
    let data_root = temporary.path().to_path_buf();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = format!("io.tabbyrs.tests.position.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let mut config: tauri::utils::config::Config =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("fixture config");
    window_geometry::prepare_startup(&mut config);
    let main_config = config
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .unwrap()
        .clone();
    let mut main_config = main_config;
    if stock {
        main_config.center = false;
    }
    let (checks, failures) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            let mut primary = main_config;
            primary.label = "position-primary".into();
            primary.url = tauri::WebviewUrl::External("about:blank".parse()?);
            primary.visible = false;
            primary.incognito = true;
            primary.data_directory = Some(data_root.join("primary"));
            let primary = tauri::WebviewWindowBuilder::from_config(app, &primary)?.build()?;
            let secondary = tauri::WebviewWindowBuilder::new(
                app,
                "position-frameless",
                tauri::WebviewUrl::External("about:blank".parse()?),
            )
            .inner_size(800.0, 600.0)
            .position(350.0, 240.0)
            .decorations(false)
            .visible(false)
            .incognito(true)
            .data_directory(data_root.join("secondary"))
            .build()?;
            // Custom windows do not run the stock geometry initializer.
            let custom = tauri::WebviewWindowBuilder::new(
                app,
                "position-custom",
                tauri::WebviewUrl::External("about:blank".parse()?),
            )
            .inner_size(900.0, 650.0)
            .visible(false)
            .incognito(true)
            .data_directory(data_root.join("custom"))
            .build()?;
            let windows = vec![primary, secondary, custom];
            let checks = checks.clone();
            let failures = failures.clone();
            let app_handle = app.handle().clone();
            let result = result.clone();
            let (tx, rx) = mpsc::channel();
            let mut phase = 0;
            let mut ticks = 0;
            let mut frameless_xid = None;
            let mut native_xid = None;
            gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
                ticks += 1;
                if ticks > 120 {
                    failures.fetch_add(1, Ordering::SeqCst);
                    finish(&checks, &failures, &result, &app_handle);
                    return gtk::glib::ControlFlow::Break;
                }
                let (native, frameless, custom) = (&windows[0], &windows[1], &windows[2]);
                match phase {
                    0 => {
                        if !stock {
                            for window in &windows[..2] {
                                checks.fetch_add(1, Ordering::SeqCst);
                                let initialized = geometry::initialize(window).map_err(|error| error.to_string());
                                if initialized.is_err() {
                                    failures.fetch_add(1, Ordering::SeqCst);
                                }
                                println!("position phase=static-initializer window={} actual={initialized:?} expected=Ok(())", window.label());
                            }
                        }
                        let scale = native.gtk_window().unwrap().scale_factor().max(1);
                        let centered = (
                            ((screen_width / scale as u32) as i32 - 800) / 2 * scale,
                            ((screen_height / scale as u32) as i32 - 600) / 2 * scale,
                        );
                        verify_pre_realization(
                            "first-hidden-native",
                            native,
                            centered,
                            (800 * scale as u32, 600 * scale as u32),
                            stock,
                            &checks,
                            &failures,
                        );
                        verify_pre_realization(
                            "first-hidden-secondary",
                            frameless,
                            (350 * scale, 240 * scale),
                            (800 * scale as u32, 600 * scale as u32),
                            stock,
                            &checks,
                            &failures,
                        );
                        set_position(custom, (460, 260), stock);
                        custom.show().unwrap();
                        native.show().unwrap();
                        set_position(frameless, (350, 240), stock);
                        frameless.show().unwrap();
                    }
                    1 => {
                        set_position(custom, (560, 300), stock);
                        verify_position("stock-centered", native, (screen_width, screen_height), stock, &checks, &failures);
                        native_xid = Some(x11_id(native));
                        let xid = x11_id(frameless);
                        frameless_xid = Some(xid);
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let mapped_position = (350 * scale, 240 * scale);
                        let mapped_size = (
                            800 * scale as u32,
                            600 * scale as u32,
                        );
                        verify_bounds("mapped-explicit", frameless, (mapped_position.0, mapped_position.1), mapped_size, xid, (mapped_position.0, mapped_position.1), mapped_size, stock, &checks, &failures);
                        set_position(frameless, (350, 240), stock);
                        let moved = frameless.clone();
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let result = if stock { stock_bounds(&moved) } else { bounds::window_bounds(&moved).map_err(|e| e.to_string()) };
                            tx.send(result.map_err(|e| e.to_string())).unwrap()
                        });
                    }
                    2 => {
                        let worker_expected = {
                            let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                            ((350 * scale, 240 * scale), (800 * scale as u32, 600 * scale as u32))
                        };
                        match rx.try_recv() {
                            Ok(worker) => verify_worker(Ok(worker), worker_expected, &checks, &failures),
                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                return gtk::glib::ControlFlow::Continue;
                            }
                            Err(error) => verify_worker(Err(error), worker_expected, &checks, &failures),
                        }
                        let custom_scale = custom.gtk_window().unwrap().scale_factor().max(1);
                        let custom_position = (560 * custom_scale, 300 * custom_scale);
                        let custom_size = (900 * custom_scale as u32, 650 * custom_scale as u32);
                        verify_bounds("custom-without-stock-initializer", custom, custom_position, custom_size, x11_id(custom), custom_position, custom_size, stock, &checks, &failures);
                        frameless.hide().unwrap();
                        set_position(frameless, (220, 180), stock);
                        native.set_size(tauri::LogicalSize::new(810.0, 610.0)).unwrap();
                        frameless.set_size(tauri::LogicalSize::new(810.0, 610.0)).unwrap();
                    }
                    3 => {
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let expected = ((220 * scale, 180 * scale), (810 * scale as u32, 610 * scale as u32));
                        let raw_size = (800 * scale as u32, 600 * scale as u32);
                        verify_bounds("hidden-position-resize", frameless, expected.0, expected.1, frameless_xid.unwrap(), expected.0, raw_size, stock, &checks, &failures);
                        frameless.show().unwrap();
                    }
                    4 => {
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let expected = ((220 * scale, 180 * scale), (810 * scale as u32, 610 * scale as u32));
                        verify_bounds("reshown-position-resize", frameless, expected.0, expected.1, frameless_xid.unwrap(), expected.0, expected.1, stock, &checks, &failures);
                        set_position(frameless, (350, 240), stock);
                        frameless.set_size(tauri::LogicalSize::new(810.0, 610.0)).unwrap();
                    }
                    5 => {
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let expected = ((350 * scale, 240 * scale), (810 * scale as u32, 610 * scale as u32));
                        verify_bounds("mapped-position-resize-settled", frameless, expected.0, expected.1, frameless_xid.unwrap(), expected.0, expected.1, stock, &checks, &failures);
                        send_ewmh_move(native, -1, 181);
                        send_ewmh_move(frameless, -1, 181);
                    }
                    6 => {
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let expected = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-2, 180), (1622, 1222)),
                            other => panic!("unverified GTK scale factor {other}"),
                        };
                        let native_expected = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-1, 181), (1620, 1220)),
                            _ => unreachable!(),
                        };
                        verify_bounds("mapped-ewmh", frameless, expected.0, expected.1, frameless_xid.unwrap(), native_expected.0, native_expected.1, stock, &checks, &failures);
                        let native_expected = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-2, 180), (1622, 1222)),
                            _ => unreachable!(),
                        };
                        let native_client = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-1, 181), (1620, 1220)),
                            _ => unreachable!(),
                        };
                        verify_bounds("native-mapped-ewmh", native, native_expected.0, native_expected.1, native_xid.unwrap(), native_client.0, native_client.1, stock, &checks, &failures);
                        native.hide().unwrap();
                        frameless.hide().unwrap();
                    }
                    7 => {
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let expected = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-2, 180), (1622, 1222)),
                            other => panic!("unverified GTK scale factor {other}"),
                        };
                        let native_expected = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-1, 181), (1620, 1220)),
                            _ => unreachable!(),
                        };
                        verify_bounds("hidden-retains-ewmh", frameless, expected.0, expected.1, frameless_xid.unwrap(), native_expected.0, native_expected.1, stock, &checks, &failures);
                        let native_expected = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-2, 180), (1622, 1222)),
                            _ => unreachable!(),
                        };
                        let native_client = match scale {
                            1 => ((-1, 181), (810, 610)),
                            2 => ((-1, 181), (1620, 1220)),
                            _ => unreachable!(),
                        };
                        verify_bounds("native-hidden-retains-ewmh", native, native_expected.0, native_expected.1, native_xid.unwrap(), native_client.0, native_client.1, stock, &checks, &failures);
                        native.show().unwrap();
                        frameless.show().unwrap();
                    }
                    8 => {
                        let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                        let expected = match scale {
                            1 => ((0, 181), (810, 610)),
                            2 => ((0, 180), (1620, 1222)),
                            other => panic!("unverified GTK scale factor {other}"),
                        };
                        let native_expected = match scale {
                            1 => ((0, 181), (810, 610)),
                            2 => ((0, 181), (1620, 1220)),
                            _ => unreachable!(),
                        };
                        verify_bounds("reshown-openbox-clamp", frameless, expected.0, expected.1, frameless_xid.unwrap(), native_expected.0, native_expected.1, stock, &checks, &failures);
                        verify_bounds("repeated-read", frameless, expected.0, expected.1, frameless_xid.unwrap(), native_expected.0, native_expected.1, stock, &checks, &failures);
                        let native_getter = match scale {
                            1 => ((1, 181), (810, 610)),
                            2 => ((0, 180), (1622, 1222)),
                            _ => unreachable!(),
                        };
                        let native_xwin = match scale {
                            1 => ((1, 181), (810, 610)),
                            2 => ((1, 181), (1620, 1220)),
                            _ => unreachable!(),
                        };
                        verify_bounds("native-reshown-openbox-clamp", native, native_getter.0, native_getter.1, native_xid.unwrap(), native_xwin.0, native_xwin.1, stock, &checks, &failures);
                        let support = if stock {
                            stock_supports_absolute_positioning()
                        } else {
                            bounds::supports_absolute_positioning(frameless)
                                .map_err(|error| error.to_string())
                        };
                        checks.fetch_add(1, Ordering::SeqCst);
                        let expected = gtk::gdk::Display::default()
                            .is_some_and(|display| display.downcast::<gdkx11::X11Display>().is_ok());
                        if support != Ok(expected) {
                            failures.fetch_add(1, Ordering::SeqCst);
                        }
                        println!("position phase=backend-capability actual={support:?} expected=Ok({expected})");
                        native.destroy().unwrap();
                        let dead = native.clone();
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let result = if stock { stock_bounds(&dead) } else { bounds::window_bounds(&dead).map_err(|e| e.to_string()) };
                            tx.send(result.map_err(|e| e.to_string())).unwrap()
                        });
                    }
                    9 => {
                        if let Ok(actual) = rx.try_recv() {
                            checks.fetch_add(1, Ordering::SeqCst);
                            if actual.is_ok() {
                                failures.fetch_add(1, Ordering::SeqCst);
                            }
                            println!("position phase=destroyed-handle actual={actual:?} expected=Err");
                            let scale = frameless.gtk_window().unwrap().scale_factor().max(1);
                            let surviving_getter = if scale == 1 {
                                ((0, 181), (810, 610))
                            } else {
                                ((0, 180), (1620, 1222))
                            };
                            let surviving_native = ((0, 181), (810 * scale as u32, 610 * scale as u32));
                            verify_bounds("surviving-window-isolation", frameless, surviving_getter.0, surviving_getter.1, frameless_xid.unwrap(), surviving_native.0, surviving_native.1, stock, &checks, &failures);
                            custom.destroy().unwrap();
                            frameless.destroy().unwrap();
                            finish(&checks, &failures, &result, &app_handle);
                            return gtk::glib::ControlFlow::Break;
                        }
                        phase -= 1;
                    }
                    _ => unreachable!(),
                }
                phase += 1;
                gtk::glib::ControlFlow::Continue
            });
            Ok(())
        })
        .build(context)
        .expect("real Tauri/Wry fixture");
    app.run_return(|_, _| {});
    std::process::exit(outcome.load(Ordering::SeqCst));
}

#[cfg(target_os = "linux")]
fn run_wayland_compatibility_probe() {
    use std::{
        sync::{
            atomic::{AtomicI32, AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    let temporary = tempfile::tempdir().expect("isolated WebView data");
    let data_root = temporary.path().to_path_buf();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier =
        format!("io.tabbyrs.tests.position.wayland.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let outcome = Arc::new(AtomicI32::new(2));
    let watchdog_outcome = outcome.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(45));
        eprintln!(
            "window_position_linux completed: checks=0 failures=1 watchdog=true backend=Wayland"
        );
        watchdog_outcome.store(1, Ordering::SeqCst);
        std::process::exit(1);
    });
    let result = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "wayland-compatibility",
                tauri::WebviewUrl::External("about:blank".parse()?),
            )
            .inner_size(800.0, 600.0)
            .visible(false)
            .incognito(true)
            .data_directory(data_root.join("wayland"))
            .build()?;
            window.show()?;
            let handle = app.handle().clone();
            let result = result.clone();
            let checks = Arc::new(AtomicUsize::new(0));
            let failures = Arc::new(AtomicUsize::new(0));
            let local_checks = checks.clone();
            let local_failures = failures.clone();
            gtk::glib::timeout_add_local(Duration::from_millis(500), move || {
                let capability = bounds::supports_absolute_positioning(&window);
                local_checks.fetch_add(1, Ordering::SeqCst);
                if capability != Ok(false) { local_failures.fetch_add(1, Ordering::SeqCst); }
                let old_read = stock_bounds(&window);
                let read = bounds::window_bounds(&window).map_err(|error| error.to_string());
                local_checks.fetch_add(1, Ordering::SeqCst);
                if old_read.is_ok() != read.is_ok() { local_failures.fetch_add(1, Ordering::SeqCst); }
                let requested = tauri::PhysicalPosition::new(30, 40);
                let old_move = window.set_position(requested);
                let move_result = bounds::set_window_position(&window, requested).map_err(|error| error.to_string());
                local_checks.fetch_add(1, Ordering::SeqCst);
                if old_move.is_ok() != move_result.is_ok() { local_failures.fetch_add(1, Ordering::SeqCst); }
                println!("position backend=Wayland absolute={capability:?} getter_old={old_read:?} getter_current={read:?} setter_old={old_move:?} setter_current={move_result:?}; compatibility only, absolute coordinates not accepted");
                window.destroy().unwrap();
                let count = local_checks.load(Ordering::SeqCst);
                let failed = local_failures.load(Ordering::SeqCst);
                println!("window_position_linux completed: checks={count} failures={failed} backend=Wayland");
                result.store(i32::from(failed != 0), Ordering::SeqCst);
                handle.exit(i32::from(failed != 0));
                gtk::glib::ControlFlow::Break
            });
            Ok(())
        })
        .build(context)
        .expect("Wayland compatibility fixture");
    app.run_return(|_, _| {});
    std::process::exit(outcome.load(Ordering::SeqCst));
}

#[cfg(target_os = "linux")]
fn parse_xwininfo(text: &str, key: &str) -> u32 {
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or_else(|| panic!("missing {key} in xwininfo output: {text}"))
}

#[cfg(target_os = "linux")]
fn parse_xwininfo_signed(text: &str, key: &str) -> i32 {
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or_else(|| panic!("missing {key} in xwininfo output: {text}"))
}

#[cfg(target_os = "linux")]
fn set_position(window: &tauri::WebviewWindow, logical: (i32, i32), stock: bool) {
    use gtk::prelude::*;
    let scale = window
        .gtk_window()
        .map(|w| w.scale_factor().max(1))
        .unwrap_or(1);
    let position = if stock {
        tauri::PhysicalPosition::new(logical.0 * scale, logical.1 * scale)
    } else {
        tauri::LogicalPosition::new(logical.0 as f64, logical.1 as f64)
            .to_physical::<i32>(scale as f64)
    };
    if stock {
        window.set_position(position).unwrap();
    } else {
        bounds::set_window_position(window, position).unwrap();
    }
}

#[cfg(target_os = "linux")]
fn stock_bounds(
    window: &tauri::WebviewWindow,
) -> Result<(tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>), String> {
    use gtk::prelude::*;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let native = window.gtk_window().map_err(|error| error.to_string())?;
    let (width, height) = native.size();
    Ok((
        position,
        tauri::LogicalSize::new(width, height).to_physical(native.scale_factor() as f64),
    ))
}

#[cfg(target_os = "linux")]
fn x11_id(window: &tauri::WebviewWindow) -> u64 {
    use gtk::prelude::*;
    window
        .gtk_window()
        .expect("GTK window")
        .window()
        .expect("realized GDK window")
        .downcast::<gdkx11::X11Window>()
        .expect("X11 GDK window")
        .xid() as u64
}

#[cfg(target_os = "linux")]
fn current_x11_id(window: &tauri::WebviewWindow) -> Option<u64> {
    use gtk::prelude::*;
    window
        .gtk_window()
        .ok()?
        .window()?
        .downcast::<gdkx11::X11Window>()
        .ok()
        .map(|window| window.xid() as u64)
}

#[cfg(target_os = "linux")]
fn send_ewmh_move(window: &tauri::WebviewWindow, x: i32, y: i32) {
    use std::{
        ffi::{c_long, CString},
        ptr,
    };
    use x11::xlib;

    let xid = x11_id(window) as xlib::Window;
    unsafe {
        let display = xlib::XOpenDisplay(ptr::null());
        assert!(!display.is_null(), "open fixture X display");
        let atom_name = CString::new("_NET_MOVERESIZE_WINDOW").unwrap();
        let atom = xlib::XInternAtom(display, atom_name.as_ptr(), xlib::False);
        let mut event: xlib::XEvent = std::mem::zeroed();
        event.client_message = xlib::XClientMessageEvent {
            type_: xlib::ClientMessage,
            serial: 0,
            send_event: xlib::True,
            display,
            window: xid,
            message_type: atom,
            format: 32,
            data: [
                10 | (1 << 8) | (1 << 9) | (1 << 12),
                x as c_long,
                y as c_long,
                0,
                0,
            ]
            .into(),
        };
        let root = xlib::XDefaultRootWindow(display);
        let sent = xlib::XSendEvent(
            display,
            root,
            xlib::False,
            xlib::SubstructureRedirectMask | xlib::SubstructureNotifyMask,
            &mut event,
        );
        xlib::XFlush(display);
        xlib::XCloseDisplay(display);
        assert_ne!(sent, 0, "send EWMH move request");
    }
}

#[cfg(target_os = "linux")]
fn verify_bounds(
    phase: &str,
    window: &tauri::WebviewWindow,
    position: (i32, i32),
    size: (u32, u32),
    xid: u64,
    native_position: (i32, i32),
    native_size: (u32, u32),
    stock: bool,
    checks: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    failures: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::process::Command;
    let actual = if stock {
        stock_bounds(window)
    } else {
        bounds::window_bounds(window).map_err(|e| e.to_string())
    }
    .map(|(p, s)| ((p.x, p.y), (s.width, s.height)))
    .map_err(|e| e.to_string());
    let current_xid = current_x11_id(window);
    let query_xid = current_xid.unwrap_or(xid);
    let native_output = Command::new("xwininfo")
        .args(["-id", &format!("0x{query_xid:x}")])
        .output();
    checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let expected = Ok((position, size));
    let native_observation = native_output.as_ref().ok().and_then(|output| {
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Some((
            output.status.success(),
            parse_xwininfo_signed(&text, "Absolute upper-left X:"),
            parse_xwininfo_signed(&text, "Absolute upper-left Y:"),
            parse_xwininfo(&text, "Width:"),
            parse_xwininfo(&text, "Height:"),
        ))
    });
    let native_matches = native_observation.is_some_and(|(success, x, y, width, height)| {
        success
            && (x, y, width, height)
                == (
                    native_position.0,
                    native_position.1,
                    native_size.0,
                    native_size.1,
                )
    });
    if actual != expected
        || current_xid.is_none()
        || native_output
            .as_ref()
            .map_or(true, |output| !output.status.success())
        || !native_matches
    {
        failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    let native_state = match native_output.as_ref() {
        Ok(output) if output.status.success() => "xid-queryable",
        _ => {
            let tree = Command::new("xwininfo").args(["-root", "-tree"]).output();
            if tree.as_ref().is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout).contains(&format!("0x{query_xid:x}"))
            }) {
                "listed-in-root-tree"
            } else {
                "owned-xid-not-listed-or-withdrawn"
            }
        }
    };
    println!(
        "position phase={phase} actual={actual:?} expected={expected:?} xwininfo={native_observation:?} native_expected={native_position:?}/{native_size:?} xid_before={xid:#x} xid_current={current_xid:?} xwininfo_state={native_state} stderr={:?}",
        native_output.as_ref().map(|o| String::from_utf8_lossy(&o.stderr).to_string()).unwrap_or_else(|e| e.to_string())
    );
}

#[cfg(target_os = "linux")]
fn verify_position(
    phase: &str,
    window: &tauri::WebviewWindow,
    screen: (u32, u32),
    stock: bool,
    checks: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    failures: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use gtk::prelude::*;
    let actual = (if stock {
        stock_bounds(window)
    } else {
        bounds::window_bounds(window).map_err(|e| e.to_string())
    })
    .map(|(p, _)| (p.x, p.y))
    .map_err(|e| e.to_string());
    let gtk = window.gtk_window().unwrap();
    let scale = gtk.scale_factor().max(1) as u32;
    let expected = (
        ((screen.0 / scale) as i32 - 800) / 2,
        ((screen.1 / scale) as i32 - 600) / 2,
    );
    let expected = (expected.0 * scale as i32, expected.1 * scale as i32);
    checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if actual != Ok(expected) {
        failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    println!("position phase={phase} actual={actual:?} expected={expected:?} screen={screen:?} scale={scale}");
}

#[cfg(target_os = "linux")]
fn verify_pre_realization(
    phase: &str,
    window: &tauri::WebviewWindow,
    position: (i32, i32),
    size: (u32, u32),
    stock: bool,
    checks: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    failures: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let actual = if stock {
        stock_bounds(window)
    } else {
        bounds::window_bounds(window).map_err(|error| error.to_string())
    }
    .map(|(position, size)| ((position.x, position.y), (size.width, size.height)))
    .map_err(|error| error.to_string());
    checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let expected = Ok((position, size));
    if actual != expected {
        failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    println!("position phase={phase} actual={actual:?} expected={expected:?} xwininfo=not-applicable pre-realization");
}

#[cfg(target_os = "linux")]
fn stock_supports_absolute_positioning() -> Result<bool, String> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|value| value.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
    Ok(!wayland)
}

#[cfg(target_os = "linux")]
fn verify_worker(
    actual: Result<
        Result<(tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>), String>,
        std::sync::mpsc::TryRecvError,
    >,
    expected: ((i32, i32), (u32, u32)),
    checks: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    failures: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let matches = actual.as_ref().is_ok_and(|bounds| {
        bounds.as_ref().is_ok_and(|(position, size)| {
            ((position.x, position.y), (size.width, size.height)) == expected
        })
    });
    if !matches {
        failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    println!("position phase=worker-getter actual={actual:?} expected={expected:?}");
}

#[cfg(target_os = "linux")]
fn finish(
    checks: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    failures: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    result: &std::sync::Arc<std::sync::atomic::AtomicI32>,
    app: &tauri::AppHandle,
) {
    let checks = checks.load(std::sync::atomic::Ordering::SeqCst);
    let failures = failures.load(std::sync::atomic::Ordering::SeqCst);
    println!("window_position_linux completed: checks={checks} failures={failures}");
    result.store(
        i32::from(failures != 0),
        std::sync::atomic::Ordering::SeqCst,
    );
    app.exit(i32::from(failures != 0));
}
