use std::sync::mpsc::sync_channel;
use std::time::Duration;

// These are deliberately minimal boundary shapes for the real platform module.
// They are not application implementations and contain no tested behavior.
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
    eprintln!("window_effects_native: AppKit checks require macOS; no native checks ran");
}

#[cfg(target_os = "macos")]
fn main() {
    use tauri::WebviewUrl;

    let pid = std::process::id();
    let isolated_data = match tempfile::tempdir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("window_effects_native: cannot create isolated data directory: {error}");
            std::process::exit(1);
        }
    };
    let watchdog_data = isolated_data.path().to_path_buf();
    let timeout_data = watchdog_data.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("window_effects_native watchdog: timed out after 180 seconds");
        let _ = std::fs::remove_dir_all(timeout_data);
        std::process::exit(1);
    });

    let mut context = tauri::generate_context!();
    let Some(main_config) = context.config().app.windows.first().cloned() else {
        let _ = isolated_data.close();
        eprintln!("window_effects_native: generated config has no primary window");
        std::process::exit(1);
    };
    context.config_mut().identifier = format!("io.tabbyrs.tests.window-effects.{pid}");
    context.config_mut().app.windows.clear();
    let (completed, completion) = sync_channel(1);

    let app_result = tauri::Builder::default()
        .setup(move |app| {
            let blank = WebviewUrl::External("about:blank".parse()?);
            // Both native windows inherit the actual configured main-window options. This
            // example does not call the production create_window function for the secondary;
            // its shared transparency setup is covered by the repository's Node regression.
            let mut main_window_config = main_config.clone();
            main_window_config.label = "effectsMain".into();
            main_window_config.url = blank.clone();
            main_window_config.title = "Native effects test".into();
            main_window_config.visible = false;
            let main = tauri::WebviewWindowBuilder::from_config(app, &main_window_config)?
                .incognito(true)
                .data_directory(isolated_data.path().to_path_buf())
                .build()?;
            let mut secondary_config = main_config.clone();
            secondary_config.label = "effectsSecondary".into();
            secondary_config.url = blank;
            secondary_config.title = "Native effects test secondary".into();
            secondary_config.visible = false;
            let secondary = tauri::WebviewWindowBuilder::from_config(app, &secondary_config)?
                .incognito(true)
                .data_directory(isolated_data.path().to_path_buf())
                .build()?;

            let app_handle = app.handle().clone();
            let owned_data = isolated_data;
            std::thread::spawn(move || {
                let mut failures = run_checks(&main, &secondary);
                let _ = main.destroy();
                let _ = secondary.destroy();
                if let Err(error) = owned_data.close() {
                    failures.push(format!("temporary data cleanup failed: {error}"));
                }
                for failure in &failures {
                    eprintln!("window_effects_native failure: {failure}");
                }
                let exit_code = if failures.is_empty() { 0 } else { 1 };
                let _ = completed.send(exit_code);
                app_handle.exit(exit_code);
            });
            Ok(())
        })
        .build(context);
    let app = match app_result {
        Ok(app) => app,
        Err(error) => {
            eprintln!("window_effects_native startup failure: {error}");
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
fn run_checks(main: &tauri::WebviewWindow, secondary: &tauri::WebviewWindow) -> Vec<String> {
    use objc2::runtime::{AnyClass, NSObjectProtocol};
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSView, NSWindow};
    use objc2_foundation::NSRect;

    const TIMEOUT: Duration = Duration::from_secs(10);

    #[derive(Clone, Debug)]
    struct EffectInfo {
        material: isize,
        state: isize,
        blending: isize,
        autoresizing: usize,
        frame: NSRect,
        parent_bounds: NSRect,
    }
    #[derive(Debug)]
    struct Snapshot {
        effects: Vec<EffectInfo>,
        alpha: f64,
        opaque: bool,
    }

    fn on_main<T: Send + 'static>(
        window: &tauri::WebviewWindow,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = sync_channel(1);
        window
            .run_on_main_thread(move || {
                let _ = tx.send(f());
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(TIMEOUT)
            .map_err(|_| "main-thread readback timed out".to_string())
    }

    fn snapshot(window: &tauri::WebviewWindow) -> Result<Snapshot, String> {
        on_main(window, {
            let window = window.clone();
            move || {
                let native = window.ns_window().map_err(|e| e.to_string())?;
                let native: &NSWindow = unsafe { &*native.cast() };
                let content = native.contentView().ok_or("NSWindow has no contentView")?;
                let bounds: NSRect = unsafe { objc2::msg_send![&*content, bounds] };
                let alpha: f64 = unsafe { objc2::msg_send![native, alphaValue] };
                let opaque: bool = unsafe { objc2::msg_send![native, isOpaque] };
                let mut effects = Vec::new();
                fn walk(
                    view: &NSView,
                    class: Option<&AnyClass>,
                    parent_bounds: NSRect,
                    out: &mut Vec<EffectInfo>,
                ) {
                    if let Some(class) = class {
                        if view.isKindOfClass(class) {
                            out.push(EffectInfo {
                                material: unsafe { objc2::msg_send![&*view, material] },
                                state: unsafe { objc2::msg_send![&*view, state] },
                                blending: unsafe { objc2::msg_send![&*view, blendingMode] },
                                autoresizing: unsafe { objc2::msg_send![&*view, autoresizingMask] },
                                frame: unsafe { objc2::msg_send![&*view, frame] },
                                parent_bounds,
                            });
                        }
                    }
                    let view_bounds: NSRect = unsafe { objc2::msg_send![&*view, bounds] };
                    let children = view.subviews();
                    for index in 0..children.count() {
                        let child = children.objectAtIndex(index);
                        let child: &NSView = &child;
                        walk(child, class, view_bounds, out);
                    }
                }
                let class = AnyClass::get(c"NSVisualEffectViewTagged");
                walk(&content, class, bounds, &mut effects);
                Ok::<_, String>(Snapshot {
                    effects,
                    alpha,
                    opaque,
                })
            }
        })?
    }

    fn issue(
        f: impl FnOnce() -> Result<(), error::AppError> + Send + 'static,
    ) -> Result<Result<(), error::AppError>, String> {
        let (tx, rx) = sync_channel(1);
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(TIMEOUT)
            .map_err(|error| format!("platform reply unavailable: {error}"))
    }

    fn effect_with(
        window: &tauri::WebviewWindow,
        enabled: bool,
        requested_effect: Option<&str>,
    ) -> Result<(), String> {
        let window = window.clone();
        let options = desktop::VibrancyOptions {
            enabled,
            effect: if enabled {
                requested_effect.map(str::to_owned)
            } else {
                None
            },
        };
        issue(move || platform::set_window_vibrancy(&window, &options))
            .and_then(|result| result.map_err(|error| format!("{error:?}")))
    }
    fn effect(window: &tauri::WebviewWindow, enabled: bool) -> Result<(), String> {
        effect_with(window, enabled, Some("blur"))
    }
    fn opacity(window: &tauri::WebviewWindow, value: f64) -> Result<(), String> {
        let window = window.clone();
        issue(move || platform::set_window_opacity(&window, value))
            .and_then(|result| result.map_err(|error| format!("{error:?}")))
    }

    let mut failures = Vec::new();
    // The fixed original macos-release table recognizes Darwin 18 through 25
    // as macOS 10.14+; an unknown future release falls back to Dark.
    let expected_material = match sysinfo::System::kernel_version()
        .and_then(|version| version.split('.').next()?.parse::<u32>().ok())
    {
        Some(18 | 19 | 20 | 21 | 22 | 23 | 24 | 25) => 15,
        _ => 2,
    };
    let mut checks = 0usize;
    macro_rules! check {
        ($name:expr, $condition:expr, $details:expr) => {{
            checks += 1;
            if !$condition {
                let details: String = $details;
                failures.push(format!("{}: {}", $name, details));
            }
        }};
    }
    macro_rules! apply {
        ($name:expr, $call:expr) => {
            checks += 1;
            if let Err(error) = $call {
                failures.push(format!("{} operation: {}", $name, error));
            }
        };
    }

    for (name, window) in [("main", main), ("secondary", secondary)] {
        match snapshot(window) {
            Ok(state) => check!(
                format!("{name} initial transparency"),
                !state.opaque,
                "expected initial NSWindow isOpaque=false".into()
            ),
            Err(error) => {
                checks += 1;
                failures.push(format!("{name} initial snapshot: {error}"));
            }
        }
    }

    apply!("enable main", effect(main, true));
    let first = snapshot(main);
    check!(
        "enable main",
        first.as_ref().is_ok_and(|s| s.effects.len() == 1),
        format!("expected one effect, got {first:?}")
    );
    if let Ok(state) = &first {
        check!(
            "native effect properties",
            state
                .effects
                .first()
                .is_some_and(|e| e.material == expected_material
                    && e.state == 1
                    && e.blending == 0),
            format!(
                "expected material={expected_material} state=1 blending=0, got {:?}",
                state.effects
            )
        );
        check!(
            "native autoresizing",
            state
                .effects
                .first()
                .is_some_and(|e| e.autoresizing & 18 == 18),
            format!(
                "expected width+height autoresizing bits, got {:?}",
                state.effects
            )
        );
    }
    apply!("repeat main", effect(main, true));
    let repeated = snapshot(main);
    check!(
        "repeat main",
        repeated.as_ref().is_ok_and(|s| s.effects.len() == 1),
        format!("expected one effect after repeat, got {repeated:?}")
    );
    apply!("sidebar main", effect_with(main, true, Some("sidebar")));
    let sidebar_once = snapshot(main);
    check!(
        "sidebar material",
        sidebar_once.as_ref().is_ok_and(|s| s.effects.len() == 1
            && s.effects[0].material == expected_material
            && s.effects[0].state == 1),
        format!(
            "expected one effect with original material {expected_material}, got {sidebar_once:?}"
        )
    );
    apply!(
        "repeat sidebar main",
        effect_with(main, true, Some("sidebar"))
    );
    let sidebar_twice = snapshot(main);
    check!("repeat sidebar", sidebar_twice.as_ref().is_ok_and(|s| s.effects.len() == 1 && s.effects[0].material == expected_material && s.effects[0].state == 1), format!("expected one effect with original material {expected_material} after repeat, got {sidebar_twice:?}"));
    apply!(
        "enabled with no requested effect",
        effect_with(main, true, None)
    );
    let no_requested_effect = snapshot(main);
    check!("no requested effect material", no_requested_effect.as_ref().is_ok_and(|s| s.effects.len() == 1 && s.effects[0].material == expected_material && s.effects[0].state == 1), format!("expected one effect with original material {expected_material} for None, got {no_requested_effect:?}"));
    apply!("disable main", effect(main, false));
    let disabled = snapshot(main);
    check!(
        "disable main",
        disabled.as_ref().is_ok_and(|s| s.effects.is_empty()),
        format!("expected zero effects, got {disabled:?}")
    );
    apply!("disable main twice", effect(main, false));
    check!(
        "disable main twice",
        snapshot(main).is_ok_and(|s| s.effects.is_empty()),
        "expected zero effects".into()
    );
    apply!("reenable main", effect(main, true));
    check!(
        "reenable main",
        snapshot(main).is_ok_and(|s| s.effects.len() == 1),
        "expected one effect".into()
    );

    // Add an unrelated NSView, then prove the production clear operation preserves it.
    let plain: Result<usize, String> = on_main(main, {
        let window = main.clone();
        move || {
            let native = window.ns_window().map_err(|e| e.to_string())?;
            let native: &NSWindow = unsafe { &*native.cast() };
            let content = native.contentView().ok_or("missing contentView")?;
            let marker =
                objc2::MainThreadMarker::new().ok_or("missing AppKit main-thread marker")?;
            let frame: NSRect = unsafe { objc2::msg_send![&*content, bounds] };
            let view = NSView::initWithFrame(NSView::alloc(marker), frame);
            let address = (&*view as *const NSView) as usize;
            content.addSubview(&view);
            Ok::<_, String>(address)
        }
    })
    .and_then(|v| v);
    apply!("clear with ordinary NSView", effect(main, false));
    if let Ok(address) = plain {
        let attached = on_main(main, {
            let window = main.clone();
            move || {
                let native = window.ns_window().ok()?;
                let native: &NSWindow = unsafe { &*native.cast() };
                let content = native.contentView()?;
                fn contains(view: &NSView, address: usize) -> bool {
                    if view as *const NSView as usize == address {
                        return true;
                    }
                    let children = view.subviews();
                    (0..children.count()).any(|index| {
                        let child = children.objectAtIndex(index);
                        contains(&child, address)
                    })
                }
                Some(contains(&content, address))
            }
        });
        check!(
            "ordinary NSView preservation",
            attached == Ok(Some(true)),
            format!("ordinary view remains in native hierarchy: {attached:?}")
        );
    } else if let Err(error) = plain {
        failures.push(format!("create ordinary NSView: {error}"));
    }

    apply!("enable main independently", effect(main, true));
    apply!("enable secondary", effect(secondary, true));
    let m = snapshot(main);
    let s = snapshot(secondary);
    check!(
        "independent effects before clear",
        m.as_ref().is_ok_and(|x| x.effects.len() == 1)
            && s.as_ref().is_ok_and(|x| x.effects.len() == 1),
        format!("expected 1,1 got {m:?} and {s:?}")
    );
    apply!("clear main only", effect(main, false));
    let m = snapshot(main);
    let s = snapshot(secondary);
    check!(
        "independent effects after clear",
        m.as_ref().is_ok_and(|x| x.effects.is_empty())
            && s.as_ref().is_ok_and(|x| x.effects.len() == 1),
        format!("expected 0,1 got {m:?} and {s:?}")
    );

    for value in [0.4, 1.0] {
        for (name, window) in [("main", main), ("secondary", secondary)] {
            apply!(format!("{name} opacity {value}"), opacity(window, value));
            match snapshot(window) {
                Ok(state) => {
                    check!(
                        format!("{name} opacity {value} alpha"),
                        (state.alpha - value).abs() <= 0.01,
                        format!("expected {value}, got {}", state.alpha)
                    );
                    check!(
                        format!("{name} opacity {value} opaque"),
                        !state.opaque,
                        "expected NSWindow isOpaque=false".into()
                    );
                }
                Err(error) => failures.push(format!("{name} opacity {value} readback: {error}")),
            }
        }
    }

    apply!("enable for resize", effect(main, true));
    apply!("resize main", {
        let (tx, rx) = sync_channel(1);
        let window = main.clone();
        let r = main.run_on_main_thread(move || {
            let _ = tx.send(
                window
                    .set_size(tauri::LogicalSize::new(1100.0, 760.0))
                    .map_err(|e| e.to_string()),
            );
        });
        r.map_err(|e| e.to_string()).and_then(|_| {
            rx.recv_timeout(TIMEOUT)
                .map_err(|_| "resize timed out".to_string())?
        })
    });
    let resized = snapshot(main);
    if let Ok(state) = &resized {
        if let Some(view) = state.effects.first() {
            check!(
                "resize effect frame",
                (view.frame.origin.x - view.parent_bounds.origin.x).abs() <= 1.0
                    && (view.frame.origin.y - view.parent_bounds.origin.y).abs() <= 1.0
                    && (view.frame.size.width - view.parent_bounds.size.width).abs() <= 1.0
                    && (view.frame.size.height - view.parent_bounds.size.height).abs() <= 1.0,
                format!(
                    "effect frame {:?} does not cover its parent bounds {:?}",
                    view.frame, view.parent_bounds
                )
            );
        } else {
            checks += 1;
            failures.push("resize effect frame: no native effect view".into());
        }
    } else if let Err(error) = resized {
        checks += 1;
        failures.push(format!("resize readback: {error}"));
    }

    let close = {
        let (tx, rx) = sync_channel(1);
        let window = secondary.clone();
        let result = secondary.run_on_main_thread(move || {
            let _ = tx.send(window.destroy().map_err(|e| e.to_string()));
        });
        result.map_err(|e| e.to_string()).and_then(|_| {
            rx.recv_timeout(TIMEOUT)
                .map_err(|_| "destroy timed out".to_string())?
        })
    };
    if let Err(error) = close {
        failures.push(format!("destroy secondary: {error}"));
    }
    let closed_window = secondary.clone();
    let closed_operation = issue(move || {
        platform::set_window_vibrancy(
            &closed_window,
            &desktop::VibrancyOptions {
                enabled: true,
                effect: Some("blur".into()),
            },
        )
    });
    eprintln!("window_effects_native closed-window outcome: {closed_operation:?}");
    check!(
        "closed-window operation",
        matches!(&closed_operation, Ok(Err(_))),
        format!("expected a returned native error, got {closed_operation:?}")
    );
    if failures.is_empty() {
        eprintln!("window_effects_native completed: checks={checks}");
    } else {
        eprintln!(
            "window_effects_native completed with failures: checks={checks} failures={}",
            failures.len()
        );
    }
    failures
}
