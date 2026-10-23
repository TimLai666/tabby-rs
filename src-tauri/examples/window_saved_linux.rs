//! Native primitive acceptance only; AppState/startup/close wiring is separate.
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("window_saved_linux: skipped outside Linux");
}
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_bounds_linux.rs"]
mod bounds;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_geometry_linux.rs"]
mod geometry;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_maximized.rs"]
mod maximized_bridge;
#[cfg(target_os = "linux")]
#[path = "../src/platform/window_state_linux.rs"]
pub(crate) mod saved_native;
#[cfg(target_os = "linux")]
#[path = "../src/window_geometry.rs"]
mod window_geometry;
#[cfg(target_os = "linux")]
mod platform {
    pub(crate) use crate::bounds::{set_window_position, window_bounds};
    pub(crate) use crate::saved_native as window_state_linux;
}
#[cfg(target_os = "linux")]
mod error {
    #[derive(Debug)]
    pub enum AppError {
        Io(String),
        InvalidArgument(String),
    }
    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{self:?}")
        }
    }
}
#[cfg(target_os = "linux")]
fn main() {
    use gtk::prelude::*;
    use std::{
        sync::{
            atomic::{AtomicI32, Ordering},
            Arc,
        },
        time::Duration,
    };
    use window_geometry::persistence::{LogicalBounds, SavedWindowState};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: window_saved_linux\nChecks production Linux saved-window restore/read primitives on two owned blank Native/frameless Wry windows.\nRequires one 1920x1080-or-larger logical monitor and GTK scale one or two.\nChecks constructor metadata, actual hidden GTK requests, native client reads, real maximize/fullscreen transitions, X11 minimization, destroyed handles, and independent windows. Wayland minimization has no compositor acknowledgement and remains separate acceptance.\nThis does not accept AppState, startup/close integration, native default-monitor selection on multiple displays, rendered application behavior, or desktop parity.\nNo arguments runs checks; unknown arguments exit 2.");
        return;
    }
    if !args.is_empty() {
        eprintln!("unknown argument; use --help");
        std::process::exit(2);
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("window_saved_linux: watchdog");
        std::process::exit(2);
    });
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().to_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("io.tabbyrs.tests.saved.{}", std::process::id());
    let outcome = Arc::new(AtomicI32::new(2));
    let result = outcome.clone();
    let app=tauri::Builder::default().setup(move |app| {
        let display=gtk::gdk::Display::default().unwrap();
        assert_eq!(display.n_monitors(),1,"fixture requires exactly one monitor");
        let monitor=display.monitor(0).unwrap().geometry();
        assert!(monitor.width()>=1920 && monitor.height()>=1080,"logical desktop too small");
        let x11=display.downcast_ref::<gdkx11::X11Display>().is_some();
        let centered=|width:u32,height:u32| LogicalBounds {
            x:monitor.x()+((monitor.width() as i64-width as i64).max(0)/2) as i32,
            y:monitor.y()+((monitor.height() as i64-height as i64).max(0)/2) as i32,
            width,height,
        };
        let constructor=centered(800,600);
        let ordinary=LogicalBounds{x:100,y:120,width:700,height:450};
        let offscreen=LogicalBounds{x:5000,y:4000,width:700,height:450};
        let even=LogicalBounds{x:(monitor.width()-700)/2,y:(monitor.height()-450)/2,width:700,height:450};
        let odd=centered(701,451);
        let cases=[
            ("unsaved",SavedWindowState::default(),constructor),
            ("ordinary",SavedWindowState{bounds:Some(ordinary),maximized:false},ordinary),
            ("offscreen",SavedWindowState{bounds:Some(offscreen),maximized:false},even),
            ("odd-center",SavedWindowState{bounds:Some(LogicalBounds{width:701,height:451,..offscreen}),maximized:false},odd),
            ("oversize-default",SavedWindowState{bounds:Some(LogicalBounds{width:2501,height:1801,..offscreen}),maximized:false},centered(2501,1801)),
            ("below-minimum",SavedWindowState{bounds:Some(LogicalBounds{width:200,height:100,..ordinary}),maximized:false},LogicalBounds{width:200,height:100,..ordinary}),
            ("zero",SavedWindowState{bounds:Some(LogicalBounds{width:0,height:0,..ordinary}),maximized:false},LogicalBounds{width:1,height:1,..ordinary}),
            ("saved-maximized",SavedWindowState{bounds:Some(offscreen),maximized:true},offscreen),
        ];
        let mut checks=0usize;let mut failures=0usize;
        let mut check=move |name:&str,pass:bool| {checks+=1;failures+=usize::from(!pass);println!("saved {name} pass={pass}");(checks,failures)};
        let mut windows=Vec::new();
        for (label,frame) in [("saved-native",true),("saved-frameless",false)] {
            let w=tauri::WebviewWindowBuilder::new(app,label,tauri::WebviewUrl::External("about:blank".parse()?))
                .inner_size(800.,600.).min_inner_size(400.,300.).center().decorations(frame).visible(false)
                .incognito(true).data_directory(root.join(label)).build()?;
            geometry::initialize(&w).unwrap();windows.push(w);
        }
        for (index,w) in windows.iter().enumerate() {
            for (name,seed,expected) in cases {
                let restored=saved_native::restore_startup(w,seed,None);
                println!("{name} {index} restored={restored:?} expected={expected:?}");
                check(&format!("{name}-{index}-constructor"),restored.as_ref().is_ok_and(|s|*s==expected));
                let native=w.gtk_window().unwrap();
                let (width,height)=native.size();
                let below_minimum=expected.width<400 || expected.height<300;
                // Before mapping, an invisible WebView does not contribute its
                // minimum to GTK's preferred size. Read the raw hidden size;
                // verify the content request here and its allocation after map.
                check(&format!("{name}-{index}-native-request"),if below_minimum {
                    w.default_vbox().unwrap().size_request()==(400,300)
                } else {width==expected.width as i32 && height==expected.height as i32});
                let live=saved_native::read(w,expected);
                let expected_live=if below_minimum {LogicalBounds{width:width as u32,height:height as u32,..expected}} else {expected};
                println!("{name} {index} live={live:?} expected={expected_live:?}");
                check(&format!("{name}-{index}-read"),live.as_ref().is_ok_and(|s|s.bounds==Some(expected_live) && !s.maximized));
            }
            if index==0 {
                check("untouched-peer",saved_native::read(&windows[1],constructor).is_ok_and(|s|s.bounds==Some(constructor)&&!s.maximized));
            }
        }
        for w in &windows {
            let _=saved_native::restore_startup(w,SavedWindowState{bounds:Some(LogicalBounds{width:0,height:0,..ordinary}),maximized:false},None);
            w.show().unwrap();
        }
        let handle=app.handle().clone();let mut stage=0;
        gtk::glib::timeout_add_local(Duration::from_millis(600),move || {
            match stage {
                0|1=> {
                    for w in &windows {
                        let native=w.gtk_window().unwrap();
                        let body=w.default_vbox().unwrap();
                        check(if stage==0 {"mapped-zero-minimum"} else {"mapped-small-minimum"},native.size()==(400,300));
                        check("mapped-content-minimum",body.allocated_width()>=400 && body.allocated_height()>=300);
                        let next=if stage==0 {LogicalBounds{width:200,height:100,..ordinary}} else {ordinary};
                        saved_native::restore_startup(w,SavedWindowState{bounds:Some(next),maximized:false},None).unwrap();
                    }
                }
                2=> {
                    for w in &windows {check("mapped-normal",saved_native::read(w,ordinary).is_ok_and(|s|s.bounds==Some(ordinary)&&!s.maximized));}
                    for w in &windows {check("bridge-normal",maximized_bridge::read(w).is_ok_and(|v|!v));}
                    windows[0].maximize().unwrap();
                }
                3=> {
                    check("bridge-maximized",maximized_bridge::read(&windows[0]).is_ok_and(|v|v));
                    check("live-maximized",saved_native::read(&windows[0],ordinary).is_ok_and(|s|s.maximized));
                    windows[0].unmaximize().unwrap();
                }
                4=> {
                    check("restored-normal",saved_native::read(&windows[0],ordinary).is_ok_and(|s|s.bounds==Some(ordinary)&&!s.maximized));
                    windows[0].maximize().unwrap();
                }
                5=> {
                    check("maximized-before-minimize",windows[0].is_maximized().unwrap());
                    if x11 {
                        windows[0].minimize().unwrap();
                    } else {
                        // Wayland does not acknowledge iconification. Do not infer
                        // minimized state from focus loss or accept that flow here.
                        stage=7;
                        windows[0].set_fullscreen(true).unwrap();
                    }
                }
                6=> {
                    check("bridge-minimized",maximized_bridge::read(&windows[0]).is_ok_and(|v|!v));
                    check("native-minimized",windows[0].is_minimized().unwrap());
                    check("minimized-not-maximized",saved_native::read(&windows[0],ordinary).is_ok_and(|s|!s.maximized));
                    windows[0].unminimize().unwrap();
                }
                7=> {
                    check("unminimized-maximized",saved_native::read(&windows[0],ordinary).is_ok_and(|s|s.maximized));
                    windows[0].set_fullscreen(true).unwrap();
                }
                8=> {
                    check("bridge-fullscreen",maximized_bridge::read(&windows[0]).is_ok_and(|v|!v));
                    check("native-fullscreen",windows[0].is_fullscreen().unwrap());
                    check("fullscreen-not-maximized",saved_native::read(&windows[0],ordinary).is_ok_and(|s|!s.maximized));
                    windows[0].set_fullscreen(false).unwrap();
                }
                9=> {
                    check("bridge-exit-fullscreen",maximized_bridge::read(&windows[0]).is_ok_and(|v|v));
                    check("exit-fullscreen-maximized",saved_native::read(&windows[0],ordinary).is_ok_and(|s|s.maximized));
                    windows[0].unmaximize().unwrap();
                }
                10=> {
                    check("final-normal",saved_native::read(&windows[0],ordinary).is_ok_and(|s|s.bounds==Some(ordinary)&&!s.maximized));
                    windows[0].destroy().unwrap();
                }
                11=> {
                    check("bridge-destroyed",maximized_bridge::read(&windows[0]).is_err());
                    check("destroyed-handle",saved_native::read(&windows[0],ordinary).is_err());
                    let (checks,failures)=check("surviving-peer",saved_native::read(&windows[1],ordinary).is_ok_and(|s|s.bounds==Some(ordinary)&&!s.maximized));
                    println!("window_saved_linux completed: checks={checks} failures={failures} x11={x11}");
                    result.store(i32::from(failures!=0),Ordering::SeqCst);windows[1].destroy().unwrap();handle.exit(i32::from(failures!=0));
                    return gtk::glib::ControlFlow::Break;
                }
                _=>unreachable!(),
            }
            stage+=1;gtk::glib::ControlFlow::Continue
        });
        Ok(())
    }).build(context).unwrap();
    app.run_return(|_, _| {});
    drop(owned);
    std::process::exit(outcome.load(Ordering::SeqCst));
}
