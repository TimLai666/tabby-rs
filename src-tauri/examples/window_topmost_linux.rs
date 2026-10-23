//! Native Linux always-on-top acceptance for the production bridge and adapter.
//! The current Wry runtime owns all windows; application command wiring is outside this fixture.
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_topmost_linux: skipped outside Linux");
}
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_bounds_linux.rs"]
mod bounds;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_geometry_linux.rs"]
mod geometry;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_topmost_linux.rs"]
pub(crate) mod native_topmost;
#[cfg(target_os = "linux")]
pub(crate) use native_topmost as window_topmost_linux;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_state_linux.rs"]
pub(crate) mod saved_native;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_topmost.rs"]
mod topmost_bridge;
#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;
#[cfg(target_os = "linux")]
mod platform {
    pub(crate) use crate::bounds::{set_window_position, window_bounds};
    pub(crate) use crate::native_topmost as window_topmost_linux;
    pub(crate) use crate::saved_native as window_state_linux;
}
#[cfg(target_os = "linux")]
mod error {
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum AppError {
        Io(String),
        InvalidArgument(String),
        Unsupported(String),
    }
    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(message) | Self::InvalidArgument(message) | Self::Unsupported(message) => {
                    f.write_str(message)
                }
            }
        }
    }
}
#[cfg(target_os = "linux")]
fn main() {
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::{
            atomic::{AtomicI32, Ordering},
            Arc,
        },
        time::Duration,
    };
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: window_topmost_linux\nChecks the production topmost bridge and Linux adapter on two real hidden Native/frameless Wry windows. Requires X11 and a compositor that supports _NET_WM_STATE_ABOVE. Uses isolated incognito WebViews and exits after staged state checks. Does not establish rendered application behavior, Wayland compositor behavior, packaged-app acceptance, or desktop parity.");
        return;
    }
    if !args.is_empty() {
        eprintln!("unknown argument; use --help");
        std::process::exit(2);
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(15));
        eprintln!("window_topmost_linux: watchdog");
        std::process::exit(2);
    });
    let owned = tempfile::tempdir().unwrap();
    let data_root = owned.path().to_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("io.tabbyrs.tests.topmost.{}", std::process::id());
    let outcome = Arc::new(AtomicI32::new(2));
    let done = outcome.clone();
    let app = tauri::Builder::default()
        .setup(move |app| {
            use gtk::prelude::*;
            use std::cell::Cell;
            use tauri::Manager;
            let create =
                |label: &'static str, frame: bool, builder_hint: bool| -> tauri::WebviewWindow {
                    let mut builder = tauri::WebviewWindowBuilder::new(
                        app,
                        label,
                        tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                    )
                    .inner_size(500., 350.)
                    .decorations(frame)
                    .visible(false)
                    .incognito(true)
                    .data_directory(data_root.join(label));
                    if builder_hint {
                        builder = builder.always_on_top(true);
                    }
                    builder.build().unwrap()
                };
            let native = create("topmost-native", true, false);
            let frameless = create("topmost-frameless", false, false);
            let hinted = create("topmost-builder-hint", false, true);
            let windows = Rc::new(RefCell::new(vec![native, frameless, hinted]));
            let failures = Rc::new(Cell::new(0usize));
            let checks = Rc::new(Cell::new(0usize));
            let check = Rc::new({
                let failures = failures.clone();
                let checks = checks.clone();
                move |name: &str, pass: bool| {
                    checks.set(checks.get() + 1);
                    if !pass {
                        failures.set(failures.get() + 1);
                    }
                    println!("topmost {name} pass={pass}");
                }
            });
            for (index, w) in windows.borrow().iter().enumerate() {
                check(
                    &format!("hidden-initial-{index}"),
                    topmost_bridge::read(w).is_ok_and(|value| value == (index == 2)),
                );
            }
            for index in 0..2 {
                let window = windows.borrow()[index].clone();
                let peer = windows.borrow()[1 - index].clone();
                let set_true = topmost_bridge::set(&window, true);
                let read_true = topmost_bridge::read(&window);
                check(
                    &format!("hidden-set-true-read-{index}"),
                    set_true.is_ok() && read_true == Ok(true),
                );
                check(
                    &format!("hidden-peer-unchanged-{index}"),
                    topmost_bridge::read(&peer) == Ok(false),
                );
                check(
                    &format!("hidden-unrealized-unmapped-{index}"),
                    window.gtk_window().is_ok_and(|native| {
                        native.window().is_none() && !native.is_realized() && !native.is_mapped()
                    }),
                );
                let set_false = topmost_bridge::set(&window, false);
                let read_false = topmost_bridge::read(&window);
                check(
                    &format!("hidden-set-false-read-{index}"),
                    set_false.is_ok() && read_false == Ok(false),
                );
                check(
                    &format!("hidden-remains-unrealized-unmapped-{index}"),
                    window.gtk_window().is_ok_and(|native| {
                        native.window().is_none() && !native.is_realized() && !native.is_mapped()
                    }),
                );
            }
            let handle = app.handle().clone();
            let stage = Rc::new(Cell::new(0u8));
            let windows_for_timer = windows.clone();
            let check_for_timer = check.clone();
            let failures_for_timer = failures.clone();
            let checks_for_timer = checks.clone();
            let data_for_timer = data_root.clone();
            let done_for_timer = done.clone();
            gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
                let windows = windows_for_timer.borrow();
                match stage.get() {
                    0 => {
                        for w in windows.iter() {
                            w.show().unwrap();
                        }
                        windows[0].set_always_on_top(true).unwrap();
                        stage.set(1);
                    }
                    1 | 2 => {
                        let os_state = x11_net_wm_state_above(&windows[0]);
                        println!("topmost shown-x11-net-wm-state-above={os_state:?}");
                        check_for_timer(
                            if stage.get() == 1 {
                                "shown-above"
                            } else {
                                "shown-read-above"
                            },
                            os_state == Ok(true) && topmost_bridge::read(&windows[0]) == Ok(true),
                        );
                        stage.set(stage.get() + 1);
                    }
                    3 => {
                        windows[0].hide().unwrap();
                        topmost_bridge::set(&windows[0], false).unwrap();
                        stage.set(4);
                    }
                    4 => {
                        check_for_timer(
                            "hidden-clear-read",
                            topmost_bridge::read(&windows[0]) == Ok(false),
                        );
                        let stale = windows[1].clone();
                        stale.hide().unwrap();
                        let set_old_hint = topmost_bridge::set(&stale, true);
                        let old_hint = topmost_bridge::read(&stale);
                        check_for_timer(
                            "rebuild-source-has-true-hint",
                            set_old_hint.is_ok() && old_hint == Ok(true),
                        );
                        stale.destroy().unwrap();
                        stage.set(5);
                    }
                    5 => {
                        drop(windows);
                        stage.set(6);
                    }
                    6 => {
                        if handle.get_webview_window("topmost-frameless").is_some() {
                            return gtk::glib::ControlFlow::Continue;
                        }
                        let stale = windows[1].clone();
                        check_for_timer(
                            "destroyed-read-error",
                            topmost_bridge::read(&stale).is_err(),
                        );
                        check_for_timer(
                            "destroyed-set-error",
                            topmost_bridge::set(&stale, true).is_err(),
                        );
                        let rebuilt = tauri::WebviewWindowBuilder::new(
                            &handle,
                            "topmost-frameless",
                            tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                        )
                        .inner_size(500., 350.)
                        .decorations(false)
                        .visible(false)
                        .incognito(true)
                        .data_directory(data_for_timer.join("topmost-frameless-rebuilt"))
                        .build()
                        .unwrap();
                        check_for_timer(
                            "same-label-does-not-inherit",
                            topmost_bridge::read(&rebuilt) == Ok(false),
                        );
                        rebuilt.destroy().unwrap();
                        for label in ["topmost-native", "topmost-builder-hint"] {
                            if let Some(w) = handle.get_webview_window(label) {
                                w.destroy().unwrap();
                            }
                        }
                        let n = checks_for_timer.get();
                        let f = failures_for_timer.get();
                        println!(
                            "window_topmost_linux completed: checks={n} failures={f} display={}",
                            std::env::var("DISPLAY").unwrap_or_default()
                        );
                        done_for_timer.store(i32::from(f != 0), Ordering::SeqCst);
                        handle.exit(i32::from(f != 0));
                        return gtk::glib::ControlFlow::Break;
                    }
                    _ => unreachable!(),
                }
                gtk::glib::ControlFlow::Continue
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    app.run_return(|_, _| {});
    drop(owned);
    std::process::exit(outcome.load(Ordering::SeqCst));
}

#[cfg(target_os = "linux")]
fn x11_net_wm_state_above(window: &tauri::WebviewWindow) -> Result<bool, String> {
    use gdkx11::x11::xlib;
    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use std::{ffi::CString, ptr, slice};

    let native = window.gtk_window().map_err(|error| error.to_string())?;
    let surface = native
        .window()
        .ok_or_else(|| "GTK window has no X11 surface".to_owned())?;
    if surface.is_destroyed() {
        return Err("GTK X11 surface has been destroyed".into());
    }
    let x11_window = surface
        .downcast::<gdkx11::X11Window>()
        .map_err(|_| "GTK surface is not an X11 window".to_owned())?;
    let display = native
        .display()
        .downcast::<gdkx11::X11Display>()
        .map_err(|_| "GTK display is not X11".to_owned())?;
    let xdisplay = unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(display.to_glib_none().0) };
    if xdisplay.is_null() {
        return Err("GDK returned a null X11 display".into());
    }

    let state_name = CString::new("_NET_WM_STATE").unwrap();
    let above_name = CString::new("_NET_WM_STATE_ABOVE").unwrap();
    let (mut actual_type, mut actual_format) = (0 as xlib::Atom, 0 as i32);
    let (mut item_count, mut bytes_after) =
        (0 as std::os::raw::c_ulong, 0 as std::os::raw::c_ulong);
    let mut property: *mut std::os::raw::c_uchar = ptr::null_mut();
    display.error_trap_push();
    let (state_atom, above_atom, status) = unsafe {
        let state_atom = xlib::XInternAtom(xdisplay, state_name.as_ptr(), xlib::True);
        let above_atom = xlib::XInternAtom(xdisplay, above_name.as_ptr(), xlib::True);
        let status = if state_atom == 0 || above_atom == 0 {
            -1
        } else {
            xlib::XGetWindowProperty(
                xdisplay,
                x11_window.xid() as xlib::Window,
                state_atom,
                0,
                64,
                xlib::False,
                xlib::XA_ATOM,
                &mut actual_type,
                &mut actual_format,
                &mut item_count,
                &mut bytes_after,
                &mut property,
            )
        };
        (state_atom, above_atom, status)
    };
    let x_error = display.error_trap_pop();

    let result = if x_error != 0 {
        Err(format!(
            "X11 _NET_WM_STATE read failed with error {x_error}"
        ))
    } else if state_atom == 0 || above_atom == 0 {
        Err("X11 window-manager state atoms are unavailable".into())
    } else if status != xlib::Success as i32 {
        Err(format!("XGetWindowProperty failed with status {status}"))
    } else if actual_type == 0 && actual_format == 0 && item_count == 0 && bytes_after == 0 {
        Ok(false)
    } else if actual_type != xlib::XA_ATOM || actual_format != 32 {
        Err(format!(
            "unexpected _NET_WM_STATE property type/format: {actual_type}/{actual_format}"
        ))
    } else if item_count > 64 || bytes_after != 0 {
        Err(format!(
            "_NET_WM_STATE property exceeds the bounded read: items={item_count}, bytes_after={bytes_after}"
        ))
    } else if item_count == 0 {
        Ok(false)
    } else if property.is_null() {
        Err("XGetWindowProperty returned a null atom list".into())
    } else {
        let atoms =
            unsafe { slice::from_raw_parts(property.cast::<xlib::Atom>(), item_count as usize) };
        Ok(atoms.contains(&above_atom))
    };
    if !property.is_null() {
        unsafe { xlib::XFree(property.cast()) };
    }
    result
}
