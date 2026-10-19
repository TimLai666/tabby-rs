use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::sync_channel,
    },
    time::{Duration, Instant},
};

#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: std::ffi::c_void;
    fn dispatch_async_f(
        queue: *const std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: unsafe extern "C" fn(*mut std::ffi::c_void),
    );
}

pub static CHECKS: AtomicUsize = AtomicUsize::new(0);
const CALL_TIMEOUT: Duration = Duration::from_secs(8);
const RUN_BUDGET: Duration = Duration::from_secs(120);
const ORDERING_SIZE: tauri::PhysicalSize<u32> = tauri::PhysicalSize::new(1800, 1300);

#[derive(Clone, Debug, PartialEq)]
struct TauriState {
    size: tauri::PhysicalSize<u32>,
    position: tauri::PhysicalPosition<i32>,
    visible: bool,
}

#[derive(Clone, Debug)]
struct NativeState {
    frame: (f64, f64, f64, f64),
    scale: f64,
    responder: usize,
    alpha: f64,
    opaque: bool,
}

#[derive(Clone, Debug)]
struct Snapshot {
    tauri: TauriState,
    native: NativeState,
}

struct NativeControlWindow(objc2::rc::Retained<objc2_app_kit::NSWindow>);

impl Drop for NativeControlWindow {
    fn drop(&mut self) {
        self.0.close();
    }
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
    rx.recv_timeout(CALL_TIMEOUT)
        .map_err(|error| format!("AppKit main-thread read timed out: {error}"))
}

fn snapshot(window: &tauri::WebviewWindow) -> Result<Snapshot, String> {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSWindow;

    let window_for_native = window.clone();
    let native = on_main(window, move || {
        let raw = window_for_native
            .ns_window()
            .map_err(|error| error.to_string())?;
        let native: &NSWindow = unsafe { &*raw.cast() };
        let frame = native.frame();
        let responder: *mut AnyObject = unsafe { objc2::msg_send![native, firstResponder] };
        Ok::<_, String>(NativeState {
            frame: (
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ),
            scale: native.backingScaleFactor(),
            responder: responder as usize,
            alpha: native.alphaValue(),
            opaque: native.isOpaque(),
        })
    })??;
    Ok(Snapshot {
        tauri: TauriState {
            size: window.outer_size().map_err(|error| error.to_string())?,
            position: window.outer_position().map_err(|error| error.to_string())?,
            visible: window.is_visible().map_err(|error| error.to_string())?,
        },
        native,
    })
}

fn native_control_expected(
    owner: &tauri::WebviewWindow,
    requested: tauri::PhysicalSize<u32>,
) -> Result<tauri::PhysicalSize<u32>, String> {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSBackingStoreType, NSWindow};
    use objc2_foundation::{MainThreadMarker, NSRect, NSSize};

    let owner_for_native = owner.clone();
    on_main(owner, move || {
        let raw = owner_for_native
            .ns_window()
            .map_err(|error| error.to_string())?;
        let owner_native: &NSWindow = unsafe { &*raw.cast() };
        let owner_scale = owner_native.backingScaleFactor();
        if !owner_scale.is_finite() || owner_scale <= 0.0 {
            return Err("owner has an invalid backing scale factor".into());
        }

        let main_thread = MainThreadMarker::new()
            .ok_or_else(|| "AppKit control creation requires the main thread".to_owned())?;
        let owner_frame = owner_native.frame();
        let control = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(main_thread),
                NSRect::new(owner_frame.origin, NSSize::new(100.0, 100.0)),
                owner_native.styleMask(),
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { control.setReleasedWhenClosed(false) };
        let control = NativeControlWindow(control);
        control.0.setFrame_display(
            NSRect::new(
                owner_frame.origin,
                NSSize::new(
                    f64::from(requested.width) / owner_scale,
                    f64::from(requested.height) / owner_scale,
                ),
            ),
            false,
        );

        let control_scale = control.0.backingScaleFactor();
        if !control_scale.is_finite() || control_scale <= 0.0 {
            return Err("AppKit control has an invalid backing scale factor".into());
        }
        if (control_scale - owner_scale).abs() > 0.001 {
            return Err(format!(
                "AppKit control backing scale differs from owner: owner={owner_scale}, control={control_scale}"
            ));
        }

        let control_frame = control.0.frame();
        let width = control_frame.size.width * control_scale;
        let height = control_frame.size.height * control_scale;
        if !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || width > f64::from(u32::MAX)
            || height > f64::from(u32::MAX)
        {
            return Err(format!(
                "AppKit control returned an unrepresentable physical frame: {width}x{height}"
            ));
        }
        let expected = tauri::PhysicalSize::new(width.round() as u32, height.round() as u32);
        eprintln!(
            "window_size_native: label={} requestedPhysical={requested:?} nativeControlExpectedPhysical={expected:?}",
            owner_for_native.label()
        );
        Ok(expected)
    })?
}

fn issue(
    task: impl FnOnce() -> Result<(), crate::error::AppError> + Send + 'static,
) -> Result<Result<(), crate::error::AppError>, String> {
    let (tx, rx) = sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(task());
    });
    rx.recv_timeout(CALL_TIMEOUT)
        .map_err(|error| format!("outer-size operation timed out: {error}"))
}

fn outer(window: &tauri::WebviewWindow, size: tauri::PhysicalSize<u32>) -> Result<(), String> {
    let window = window.clone();
    issue(move || crate::platform::set_window_outer_size(&window, size))?
        .map_err(|error| format!("AppError: {error:?}"))
}

fn frame(window: &tauri::WebviewWindow, name: &str) -> Result<(), String> {
    let window = window.clone();
    let name = name.to_owned();
    issue(move || crate::platform::set_window_frame(&window, &name))?
        .map_err(|error| format!("frame AppError: {error:?}"))
}

fn check(failures: &mut Vec<String>, label: impl Into<String>, result: Result<(), String>) {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = result {
        failures.push(format!("{}: {error}", label.into()));
    }
}

fn same_native_state(a: &NativeState, b: &NativeState) -> bool {
    [a.frame.0, a.frame.1, a.frame.2, a.frame.3]
        .into_iter()
        .zip([b.frame.0, b.frame.1, b.frame.2, b.frame.3])
        .all(|(left, right)| (left - right).abs() <= 0.01)
        && (a.scale - b.scale).abs() <= 0.001
        && a.responder == b.responder
        && (a.alpha - b.alpha).abs() <= 0.001
        && a.opaque == b.opaque
}

fn wait_exact(
    window: &tauri::WebviewWindow,
    expected: tauri::PhysicalSize<u32>,
    top_left: (f64, f64),
    timeout: Duration,
) -> Result<Snapshot, String> {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < timeout {
        if let Ok(current) = snapshot(window) {
            let frame = current.native.frame;
            let physical = (
                frame.2 * current.native.scale,
                frame.3 * current.native.scale,
            );
            let requested = (expected.width as f64, expected.height as f64);
            let current_top_left = (frame.0, frame.1 + frame.3);
            let exact_native =
                (physical.0 - requested.0).abs() <= 1.0 && (physical.1 - requested.1).abs() <= 1.0;
            let stable_top_left = (current_top_left.0 - top_left.0).abs() <= 0.5
                && (current_top_left.1 - top_left.1).abs() <= 0.5;
            if current.tauri.size == expected && exact_native && stable_top_left {
                return Ok(current);
            }
            last = Some(current);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "expected Tauri outer_size={expected:?}, AppKit physical frame and top-left to match; last={last:?}"
    ))
}

fn verify_size(
    window: &tauri::WebviewWindow,
    requested: tauri::PhysicalSize<u32>,
    expected: tauri::PhysicalSize<u32>,
    before: &Snapshot,
    label: &str,
) -> Result<Snapshot, String> {
    outer(window, requested)?;
    let after = wait_exact(
        window,
        expected,
        (
            before.native.frame.0,
            before.native.frame.1 + before.native.frame.3,
        ),
        Duration::from_millis(700),
    )?;
    if after.tauri.position != before.tauri.position {
        return Err(format!(
            "Tauri outer_position changed: before={:?} after={:?}",
            before.tauri.position, after.tauri.position
        ));
    }
    if after.tauri.visible != before.tauri.visible {
        return Err(format!(
            "visibility changed: before={} after={}",
            before.tauri.visible, after.tauri.visible
        ));
    }
    if after.native.responder != before.native.responder
        || (after.native.alpha - before.native.alpha).abs() > 0.001
        || after.native.opaque != before.native.opaque
        || (after.native.scale - before.native.scale).abs() > 0.001
    {
        return Err(format!(
            "native responder/alpha/opaque/scale changed during {label}: before={:?} after={:?}",
            before.native, after.native
        ));
    }
    Ok(after)
}

unsafe extern "C" fn signal_gcd_barrier(context: *mut std::ffi::c_void) {
    let sender = Box::from_raw(context.cast::<std::sync::mpsc::SyncSender<()>>());
    let _ = sender.send(());
}

fn wait_for_main_gcd_queue() -> Result<(), String> {
    let (sender, receiver) = sync_channel(1);
    let context = Box::into_raw(Box::new(sender)).cast();
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q),
            context,
            signal_gcd_barrier,
        );
    }
    receiver
        .recv_timeout(CALL_TIMEOUT)
        .map_err(|error| format!("main GCD FIFO barrier timed out: {error}"))
}

fn wait_maximized(window: &tauri::WebviewWindow, expected: bool) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < CALL_TIMEOUT {
        if window.is_maximized().map_err(|error| error.to_string())? == expected {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "Tauri maximized state did not settle to {expected}"
    ))
}

fn set_maximized(window: &tauri::WebviewWindow, maximized: bool) -> Result<(), String> {
    let dispatch_window = window.clone();
    let operation_window = window.clone();
    on_main(&dispatch_window, move || {
        if maximized {
            operation_window.maximize()
        } else {
            operation_window.unmaximize()
        }
        .map_err(|error| error.to_string())
    })?
}

fn combined_maximized_size(window: &tauri::WebviewWindow, maximized: bool) -> Result<(), String> {
    let dispatch_window = window.clone();
    let operation_window = window.clone();
    let operation = on_main(&dispatch_window, move || {
        if maximized {
            operation_window
                .maximize()
                .map_err(|error| error.to_string())?;
        } else {
            operation_window
                .unmaximize()
                .map_err(|error| error.to_string())?;
        }
        crate::platform::set_window_outer_size(&operation_window, ORDERING_SIZE)
            .map_err(|error| error.to_string())
    })
    .and_then(|result| result);
    operation.and(wait_for_main_gcd_queue())
}

fn wait_final_size(window: &tauri::WebviewWindow) -> Result<(), String> {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < CALL_TIMEOUT {
        if let Ok(current) = snapshot(window) {
            let frame = current.native.frame;
            let physical = (
                frame.2 * current.native.scale,
                frame.3 * current.native.scale,
            );
            if current.tauri.size == ORDERING_SIZE
                && (physical.0 - f64::from(ORDERING_SIZE.width)).abs() <= 1.0
                && (physical.1 - f64::from(ORDERING_SIZE.height)).abs() <= 1.0
            {
                return Ok(());
            }
            last = Some(current);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "after GCD barrier expected Tauri and AppKit physical outer size {ORDERING_SIZE:?}, last={last:?}"
    ))
}

pub fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    use objc2_app_kit::NSWindow;

    let mut failures = Vec::new();
    let started = Instant::now();
    for (active, other) in [(first, second), (second, first)] {
        for frame_name in ["native", "thin", "full"] {
            if started.elapsed() >= RUN_BUDGET {
                failures
                    .push("120-second run budget exceeded; remaining cases were not run".into());
                return failures;
            }
            check(
                &mut failures,
                format!("set {} frame {frame_name}", active.label()),
                frame(active, frame_name),
            );
            let baseline = match snapshot(active) {
                Ok(value) => value,
                Err(error) => {
                    check(
                        &mut failures,
                        format!("read {} in {frame_name}", active.label()),
                        Err(error),
                    );
                    continue;
                }
            };
            if frame_name == "native" {
                let window = active.clone();
                let delta = on_main(active, move || {
                    let raw = window.ns_window().map_err(|error| error.to_string())?;
                    let native: &NSWindow = unsafe { &*raw.cast() };
                    let content = native.contentView().ok_or("NSWindow has no contentView")?;
                    let content_bounds: objc2_foundation::NSRect =
                        unsafe { objc2::msg_send![&*content, bounds] };
                    let content_height = content_bounds.size.height;
                    Ok::<_, String>(native.frame().size.height - content_height)
                });
                let native_titlebar_verified = match delta {
                    Ok(Ok(value)) if value > 0.5 => true,
                    Ok(Ok(value)) => {
                        check(
                            &mut failures,
                            format!("native titlebar delta {}", active.label()),
                            Err(format!("expected nonzero titlebar delta, actual={value}")),
                        );
                        false
                    }
                    Ok(Err(error)) | Err(error) => {
                        check(
                            &mut failures,
                            format!("native titlebar delta {}", active.label()),
                            Err(error),
                        );
                        false
                    }
                };
                if !native_titlebar_verified {
                    break;
                }
                let active_before = snapshot(active);
                let peer_before = snapshot(other);
                let invalid = tauri::PhysicalSize::new(1000, 1);
                let invalid_window = active.clone();
                let result =
                    issue(move || crate::platform::set_window_outer_size(&invalid_window, invalid));
                check(
                    &mut failures,
                    format!(
                        "native {} negative content height returns InvalidArgument",
                        active.label()
                    ),
                    match result {
                        Ok(Err(crate::error::AppError::InvalidArgument(_))) => Ok(()),
                        other => Err(format!(
                            "expected AppError::InvalidArgument, actual={other:?}"
                        )),
                    },
                );
                match (active_before, snapshot(active)) {
                    (Ok(before), Ok(after)) => check(
                        &mut failures,
                        format!(
                            "native {} negative content height preserves window state",
                            active.label()
                        ),
                        if before.tauri == after.tauri
                            && same_native_state(&before.native, &after.native)
                        {
                            Ok(())
                        } else {
                            Err(format!("before={before:?} after={after:?}"))
                        },
                    ),
                    (Err(error), _) => check(
                        &mut failures,
                        "native invalid-height pre-state read",
                        Err(error),
                    ),
                    (_, Err(error)) => check(
                        &mut failures,
                        "native invalid-height post-state read",
                        Err(error),
                    ),
                }
                match (peer_before, snapshot(other)) {
                    (Ok(before), Ok(after)) => check(
                        &mut failures,
                        format!(
                            "native {} negative content height preserves peer state",
                            active.label()
                        ),
                        if before.tauri == after.tauri
                            && same_native_state(&before.native, &after.native)
                        {
                            Ok(())
                        } else {
                            Err(format!("before={before:?} after={after:?}"))
                        },
                    ),
                    (Err(error), _) => check(
                        &mut failures,
                        "native invalid-height peer pre-state read",
                        Err(error),
                    ),
                    (_, Err(error)) => check(
                        &mut failures,
                        "native invalid-height peer post-state read",
                        Err(error),
                    ),
                }
            }
            eprintln!(
                "window_size_native: label={} frame={} backingScaleFactor={}",
                active.label(),
                frame_name,
                baseline.native.scale
            );
            for (logical_width, logical_height) in [(900.0, 650.0), (1000.0, 700.0), (900.5, 650.5)]
            {
                let scale = baseline.native.scale;
                let target = tauri::PhysicalSize::new(
                    (logical_width * scale).round() as u32,
                    (logical_height * scale).round() as u32,
                );
                let other_before = snapshot(other);
                let before = snapshot(active);
                let label = format!(
                    "{} {frame_name} logical={logical_width}x{logical_height} scale={scale}",
                    active.label()
                );
                let fractional_points = [
                    f64::from(target.width) / scale,
                    f64::from(target.height) / scale,
                ]
                .into_iter()
                .any(|points| (points - points.round()).abs() > 0.000_001);
                let (expected, oracle_error) = if fractional_points {
                    match native_control_expected(active, target) {
                        Ok(expected) => (expected, None),
                        Err(error) => (target, Some(error)),
                    }
                } else {
                    (target, None)
                };
                let before = match before {
                    Ok(value) => value,
                    Err(error) => {
                        check(
                            &mut failures,
                            format!("{label} initial snapshot"),
                            Err(error),
                        );
                        continue;
                    }
                };
                let first_readback = verify_size(active, target, expected, &before, &label);
                check(
                    &mut failures,
                    format!("{label} first outer-size write and independent readback"),
                    match oracle_error {
                        Some(error) => Err(format!(
                            "AppKit native control oracle failed: {error}; continued with requested size, readback={first_readback:?}"
                        )),
                        None => first_readback.map(|_| ()),
                    },
                );
                for repeat in 1..=3 {
                    let current = snapshot(active);
                    check(
                        &mut failures,
                        format!("{label} repeat {repeat} pre-snapshot"),
                        current.as_ref().map(|_| ()).map_err(Clone::clone),
                    );
                    if let Ok(current) = current {
                        check(
                            &mut failures,
                            format!("{label} repeated identical bounds {repeat}"),
                            verify_size(active, target, expected, &current, &label).map(|_| ()),
                        );
                    }
                }
                let roundtrip = snapshot(active);
                check(
                    &mut failures,
                    format!("{label} Tauri getter roundtrip read"),
                    roundtrip.as_ref().map(|_| ()).map_err(Clone::clone),
                );
                if let Ok(roundtrip) = roundtrip {
                    check(
                        &mut failures,
                        format!("{label} getter roundtrip retains exact native bounds"),
                        verify_size(
                            active,
                            roundtrip.tauri.size,
                            roundtrip.tauri.size,
                            &roundtrip,
                            &label,
                        )
                        .map(|_| ()),
                    );
                }
                if let (Ok(other_before), Ok(other_after)) = (other_before, snapshot(other)) {
                    check(
                        &mut failures,
                        format!("{label} other window Tauri and AppKit state unchanged"),
                        if other_before.tauri == other_after.tauri
                            && same_native_state(&other_before.native, &other_after.native)
                        {
                            Ok(())
                        } else {
                            Err(format!("before={other_before:?} after={other_after:?}"))
                        },
                    );
                } else {
                    check(
                        &mut failures,
                        format!("{label} other window snapshots"),
                        Err("could not read both peer snapshots".into()),
                    );
                }
            }
        }
    }

    // Reproduce startup ordering: a bounds request can be queued before the
    // ready event installs the native frame, so its content conversion must
    // remain correct after that style change runs.
    for (active, other) in [(first, second), (second, first)] {
        if started.elapsed() >= RUN_BUDGET {
            failures.push("120-second run budget exceeded before Ready-frame handoff cases".into());
            return failures;
        }
        let label = format!("{} Thin-to-Native Ready-frame handoff", active.label());
        check(
            &mut failures,
            format!("{label} reset frame to Thin"),
            frame(active, "thin"),
        );
        check(
            &mut failures,
            format!("{label} reset GCD barrier"),
            wait_for_main_gcd_queue(),
        );
        let peer_before = snapshot(other);
        let dispatch_window = active.clone();
        let operation_window = active.clone();
        let handoff = on_main(&dispatch_window, move || {
            crate::platform::set_window_outer_size(&operation_window, ORDERING_SIZE)
                .map_err(|error| error.to_string())?;
            crate::platform::set_window_frame(&operation_window, "native")
                .map_err(|error| error.to_string())
        })
        .and_then(|result| result);
        check(
            &mut failures,
            format!("{label} production outer-size then native-frame sequence"),
            handoff,
        );
        check(
            &mut failures,
            format!("{label} final main GCD barrier"),
            wait_for_main_gcd_queue(),
        );
        check(
            &mut failures,
            format!("{label} final AppKit frame and Tauri getter"),
            wait_final_size(active),
        );
        match (peer_before, snapshot(other)) {
            (Ok(before), Ok(after)) => check(
                &mut failures,
                format!("{label} leaves peer unchanged"),
                if before.tauri == after.tauri && same_native_state(&before.native, &after.native) {
                    Ok(())
                } else {
                    Err(format!("before={before:?} after={after:?}"))
                },
            ),
            (Err(error), _) => check(
                &mut failures,
                format!("{label} peer pre-state read"),
                Err(error),
            ),
            (_, Err(error)) => check(
                &mut failures,
                format!("{label} peer post-state read"),
                Err(error),
            ),
        }
    }

    for (active, other) in [(first, second), (second, first)] {
        for frame_name in ["native", "thin", "full"] {
            if started.elapsed() >= RUN_BUDGET {
                failures.push(
                    "120-second run budget exceeded before maximize/size ordering cases".into(),
                );
                return failures;
            }
            check(
                &mut failures,
                format!("set ordering-test {} frame {frame_name}", active.label()),
                frame(active, frame_name),
            );
            for maximize_after_size in [true, false] {
                if started.elapsed() >= RUN_BUDGET {
                    failures.push(
                        "120-second run budget exceeded; remaining ordering cases not run".into(),
                    );
                    return failures;
                }
                let label = format!(
                    "{} {frame_name} maximize={maximize_after_size} ordered outer size",
                    active.label()
                );
                let reset_to = !maximize_after_size;
                check(
                    &mut failures,
                    format!("{label} opposite state command"),
                    set_maximized(active, reset_to),
                );
                check(
                    &mut failures,
                    format!("{label} opposite state GCD barrier"),
                    wait_for_main_gcd_queue(),
                );
                check(
                    &mut failures,
                    format!("{label} opposite state settled"),
                    wait_maximized(active, reset_to),
                );

                let peer_before = snapshot(other);
                let combined = combined_maximized_size(active, maximize_after_size);
                check(
                    &mut failures,
                    format!("{label} main-thread state then size and FIFO barrier"),
                    combined,
                );
                check(
                    &mut failures,
                    format!("{label} final AppKit frame and Tauri getter"),
                    wait_final_size(active),
                );
                match (peer_before, snapshot(other)) {
                    (Ok(before), Ok(after)) => check(
                        &mut failures,
                        format!("{label} leaves peer unchanged"),
                        if before.tauri == after.tauri
                            && same_native_state(&before.native, &after.native)
                        {
                            Ok(())
                        } else {
                            Err(format!("before={before:?} after={after:?}"))
                        },
                    ),
                    (Err(error), _) => check(
                        &mut failures,
                        format!("{label} peer pre-state read"),
                        Err(error),
                    ),
                    (_, Err(error)) => check(
                        &mut failures,
                        format!("{label} peer post-state read"),
                        Err(error),
                    ),
                }
            }
        }
    }

    for window in [first, second] {
        let before = snapshot(window);
        for size in [
            tauri::PhysicalSize::new(u32::MAX, 650),
            tauri::PhysicalSize::new(900, u32::MAX),
        ] {
            let label = format!("invalid overflow {} {size:?}", window.label());
            let operation_window = window.clone();
            let result =
                issue(move || crate::platform::set_window_outer_size(&operation_window, size));
            check(
                &mut failures,
                format!("{label} returns InvalidArgument"),
                match result {
                    Ok(Err(crate::error::AppError::InvalidArgument(_))) => Ok(()),
                    other => Err(format!(
                        "expected AppError::InvalidArgument, actual={other:?}"
                    )),
                },
            );
            match (&before, snapshot(window)) {
                (Ok(before), Ok(after)) => check(
                    &mut failures,
                    format!("{label} leaves all state unchanged"),
                    if before.tauri == after.tauri
                        && same_native_state(&before.native, &after.native)
                    {
                        Ok(())
                    } else {
                        Err(format!("before={before:?} after={after:?}"))
                    },
                ),
                (Err(error), _) => check(
                    &mut failures,
                    format!("{label} initial state readable"),
                    Err(error.clone()),
                ),
                (_, Err(error)) => check(
                    &mut failures,
                    format!("{label} post-state readable"),
                    Err(error),
                ),
            }
        }
    }

    let peer_before = snapshot(first);
    let destroyed = second.clone();
    check(
        &mut failures,
        "destroy owned window before dead-handle probe",
        second.destroy().map_err(|error| error.to_string()),
    );
    let dead = issue(move || {
        crate::platform::set_window_outer_size(&destroyed, tauri::PhysicalSize::new(900, 650))
    });
    check(
        &mut failures,
        "destroyed window outer-size call returns an error within watchdog",
        match dead {
            Ok(Err(_)) => Ok(()),
            other => Err(format!("expected an adapter error, actual={other:?}")),
        },
    );
    match (peer_before, snapshot(first)) {
        (Ok(before), Ok(after)) => check(
            &mut failures,
            "destroyed-window probe leaves live peer unchanged",
            if before.tauri == after.tauri && same_native_state(&before.native, &after.native) {
                Ok(())
            } else {
                Err(format!("before={before:?} after={after:?}"))
            },
        ),
        (Err(error), _) => check(&mut failures, "live peer pre-state readable", Err(error)),
        (_, Err(error)) => check(&mut failures, "live peer post-state readable", Err(error)),
    }
    failures
}
