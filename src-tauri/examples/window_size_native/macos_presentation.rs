//! First-load presentation against real Wry windows and AppKit geometry.
//! Only AppState and startup-config I/O come from the example fixture.
use crate::window_geometry::persistence::{LogicalBounds, SavedWindowState};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

pub static CHECKS: AtomicUsize = AtomicUsize::new(0);
const BOUNDS: LogicalBounds = LogicalBounds {
    x: 80,
    y: 120,
    width: 700,
    height: 450,
};

pub fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    let mut failures = Vec::new();
    for window in [first, second] {
        for hidden in [false, true] {
            for maximized in [false, true] {
                let label = format!(
                    "{} hidden={hidden} saved_maximized={maximized}",
                    window.label()
                );
                let peer = if window.label() == first.label() {
                    second
                } else {
                    first
                };
                if let Err(error) = check(window, peer, hidden, maximized) {
                    failures.push(format!("{label}: {error}"));
                }
            }
        }
    }
    failures
}

fn check(
    window: &tauri::WebviewWindow,
    peer: &tauri::WebviewWindow,
    hidden: bool,
    maximized: bool,
) -> Result<(), String> {
    let original_bounds = crate::macos_saved::native_bounds(window)?;
    let peer_bounds = crate::macos_saved::native_bounds(peer)?;
    let original_visible = window.is_visible().map_err(|error| error.to_string())?;
    let original_maximized = window.is_maximized().map_err(|error| error.to_string())?;
    let seed = SavedWindowState {
        bounds: Some(BOUNDS),
        maximized,
    };
    let state = std::sync::Arc::new(crate::macos_capture::CaptureState::new(seed, hidden));

    let result: Result<(), String> = (|| {
        window.hide().map_err(|error| error.to_string())?;
        if window.is_maximized().map_err(|error| error.to_string())? {
            window.unmaximize().map_err(|error| error.to_string())?;
        }
        wait(window)?;

        crate::window_presentation_macos::prepare_window(window, &state)
            .map_err(|error| error.to_string())?;
        tauri::async_runtime::block_on(crate::window_persistence_macos::enable_capture(
            window, &state,
        ))
        .map_err(|error| error.to_string())?;
        tauri::async_runtime::block_on(crate::window_presentation_macos::present_window(
            window, &state,
        ))
        .map_err(|error| error.to_string())?;

        if hidden || !maximized {
            let actual_bounds = crate::macos_saved::native_bounds(window)?;
            verify(
                &format!("{} restored bounds", window.label()),
                actual_bounds == BOUNDS,
                format!("actual={actual_bounds:?}, expected={BOUNDS:?}"),
            )?;
        }
        if hidden {
            verify(
                &format!("{} remains hidden", window.label()),
                !window.is_visible().map_err(|error| error.to_string())?,
                "expected hidden after first-page presentation".into(),
            )?;
            verify(
                &format!("{} retains hidden maximization state", window.label()),
                !window.is_maximized().map_err(|error| error.to_string())?,
                "hidden startup must not maximize".into(),
            )?;
        } else {
            verify(
                &format!("{} is visible", window.label()),
                window.is_visible().map_err(|error| error.to_string())?,
                "expected visible after first-page presentation".into(),
            )?;
            verify(
                &format!("{} saved maximization", window.label()),
                window.is_maximized().map_err(|error| error.to_string())? == maximized,
                format!(
                    "actual={}, expected={maximized}",
                    window.is_maximized().unwrap_or(false)
                ),
            )?;
            if maximized {
                window.unmaximize().map_err(|error| error.to_string())?;
                wait(window)?;
                let restored = crate::macos_saved::native_bounds(window)?;
                verify(
                    &format!("{} bounds after unmaximize", window.label()),
                    restored == BOUNDS,
                    format!("actual={restored:?}, expected={BOUNDS:?}"),
                )?;
            }
        }

        let peer_after = crate::macos_saved::native_bounds(peer)?;
        verify(
            &format!("{} peer bounds", window.label()),
            peer_after == peer_bounds,
            format!("actual={peer_after:?}, expected={peer_bounds:?}"),
        )?;
        Ok(())
    })();

    let cleanup = restore(
        window,
        original_bounds,
        original_visible,
        original_maximized,
    );
    result?;
    cleanup?;
    Ok(())
}

fn wait(window: &tauri::WebviewWindow) -> Result<(), String> {
    tauri::async_runtime::block_on(crate::platform::wait_for_window_operations(window))
        .map_err(|error| error.to_string())
}

fn restore(
    window: &tauri::WebviewWindow,
    bounds: LogicalBounds,
    visible: bool,
    maximized: bool,
) -> Result<(), String> {
    window.hide().map_err(|error| error.to_string())?;
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())?;
        wait(window)?;
    }
    crate::platform::window_state_macos::restore_bounds(window, bounds)
        .map_err(|error| error.to_string())?;
    wait(window)?;
    if maximized {
        window.maximize().map_err(|error| error.to_string())?;
        wait(window)?;
    }
    if visible {
        window.show().map_err(|error| error.to_string())?;
    }
    wait(window)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let actual = crate::macos_saved::native_bounds(window)?;
        if actual == bounds
            && window.is_visible().map_err(|error| error.to_string())? == visible
            && window.is_maximized().map_err(|error| error.to_string())? == maximized
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "{} fixture cleanup did not restore bounds/visibility/maximization",
        window.label()
    ))
}

fn verify(label: &str, passed: bool, detail: String) -> Result<(), String> {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if passed {
        Ok(())
    } else {
        Err(format!("{label}: {detail}"))
    }
}
