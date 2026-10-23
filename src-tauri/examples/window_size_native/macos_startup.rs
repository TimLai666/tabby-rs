//! Real AppKit/Wry checks for startup geometry restoration.
use crate::{
    platform::window_state_macos::{nearest_screen, restore_bounds},
    window_geometry::persistence::{LogicalBounds, SavedWindowState},
};
use objc2_app_kit::NSScreen;
use objc2_foundation::MainThreadMarker;
use std::sync::atomic::{AtomicUsize, Ordering};

pub static CHECKS: AtomicUsize = AtomicUsize::new(0);

fn check(failures: &mut Vec<String>, name: impl Into<String>, result: Result<(), String>) {
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = result {
        failures.push(format!("{}: {error}", name.into()));
    }
}

fn zoomed(window: &tauri::WebviewWindow) -> Result<bool, String> {
    crate::macos_saved::on_main(window, |native, _| Ok(native.isZoomed()))
}

fn assert_unchanged(
    active: &tauri::WebviewWindow,
    peer: &tauri::WebviewWindow,
    active_bounds: LogicalBounds,
    peer_bounds: LogicalBounds,
    visible: bool,
    was_zoomed: bool,
) -> Result<(), String> {
    let after = crate::macos_saved::native_bounds(active)?;
    if after != active_bounds {
        return Err(format!(
            "active frame changed: {after:?} vs {active_bounds:?}"
        ));
    }
    let peer_after = crate::macos_saved::native_bounds(peer)?;
    if peer_after != peer_bounds {
        return Err(format!(
            "peer frame changed: {peer_after:?} vs {peer_bounds:?}"
        ));
    }
    let visible_after = active.is_visible().map_err(|error| error.to_string())?;
    if visible_after != visible {
        return Err(format!("visibility changed: {visible_after} vs {visible}"));
    }
    if zoomed(active)? != was_zoomed {
        return Err("startup restore changed the native zoom state".into());
    }
    Ok(())
}

fn screens(window: &tauri::WebviewWindow) -> Result<(f64, f64, f64, f64, f64), String> {
    crate::macos_saved::on_main(window, |_, marker: MainThreadMarker| {
        let list = NSScreen::screens(marker);
        let primary = list
            .firstObject()
            .ok_or("native probe has no primary screen")?
            .frame();
        let mut max_x = f64::NEG_INFINITY;
        for index in 0..list.count() {
            let frame = list.objectAtIndex(index).frame();
            max_x = max_x.max(frame.origin.x + frame.size.width);
        }
        Ok((
            primary.size.width,
            primary.size.height,
            primary.origin.x,
            primary.origin.y,
            max_x,
        ))
    })
}

fn bounds(x: i32, y: i32, width: u32, height: u32) -> LogicalBounds {
    LogicalBounds {
        x,
        y,
        width,
        height,
    }
}

fn restore_original(window: &tauri::WebviewWindow, original: LogicalBounds) -> Result<(), String> {
    restore_bounds(window, original).map_err(|error| error.to_string())?;
    if crate::macos_saved::native_bounds(window)? != original {
        return Err(format!("could not restore owned window frame {original:?}"));
    }
    Ok(())
}

pub fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    let mut failures = Vec::new();
    for (active, peer) in [(first, second), (second, first)] {
        let original = match crate::macos_saved::native_bounds(active) {
            Ok(value) => value,
            Err(error) => {
                failures.push(format!(
                    "{} startup fixture initial frame: {error}",
                    active.label()
                ));
                continue;
            }
        };

        let none_case = (|| {
            let before = crate::macos_saved::native_bounds(active)?;
            let peer_before = crate::macos_saved::native_bounds(peer)?;
            let visible = active.is_visible().map_err(|error| error.to_string())?;
            let was_zoomed = zoomed(active)?;
            let result = crate::platform::window_state_macos::restore_startup(
                active,
                SavedWindowState {
                    bounds: None,
                    maximized: false,
                },
            )
            .map_err(|error| error.to_string())?;
            if result.is_some() {
                return Err(format!("missing saved bounds returned {result:?}"));
            }
            assert_unchanged(active, peer, before, peer_before, visible, was_zoomed)
        })();
        check(
            &mut failures,
            format!("{} startup missing bounds", active.label()),
            none_case,
        );

        let frames_case = (|| {
            let peer_before = crate::macos_saved::native_bounds(peer)?;
            let visible = active.is_visible().map_err(|error| error.to_string())?;
            let was_zoomed = zoomed(active)?;
            for requested in [bounds(80, 120, 700, 450), bounds(-100, 120, 700, 450)] {
                let result = crate::platform::window_state_macos::restore_startup(
                    active,
                    SavedWindowState {
                        bounds: Some(requested),
                        maximized: false,
                    },
                )
                .map_err(|error| error.to_string())?;
                if result != Some(requested) {
                    return Err(format!(
                        "startup bounds differ: got {result:?}, expected {requested:?}"
                    ));
                }
                assert_unchanged(active, peer, requested, peer_before, visible, was_zoomed)?;
            }
            Ok(())
        })();
        check(
            &mut failures,
            format!("{} startup normal and overlapping bounds", active.label()),
            frames_case,
        );

        let relocated_case = (|| {
            let (_, _, _, _, max_x) = screens(active)?;
            let requested_x = (max_x + 2000.0).ceil() as i32;
            let requested_y = 120;
            let peer_before = crate::macos_saved::native_bounds(peer)?;
            let visible = active.is_visible().map_err(|error| error.to_string())?;
            let was_zoomed = zoomed(active)?;
            let selected = nearest_screen(active, requested_x, requested_y)
                .map_err(|error| error.to_string())?;
            let width = if selected.width % 2 == 0 { 700 } else { 701 };
            let height = if selected.height % 2 == 0 { 450 } else { 451 };
            let requested = bounds(requested_x, requested_y, width, height);
            let expected = bounds(
                (selected.width / 2) as i32 - (width / 2) as i32,
                (selected.height / 2) as i32 - (height / 2) as i32,
                width,
                height,
            );
            let result = crate::platform::window_state_macos::restore_startup(
                active,
                SavedWindowState {
                    bounds: Some(requested),
                    maximized: false,
                },
            )
            .map_err(|error| error.to_string())?;
            if result != Some(expected) {
                return Err(format!("off-screen startup result {result:?}, expected {expected:?}; selected screen {selected:?}"));
            }
            if zoomed(active)? {
                return Err("off-screen startup restore maximized the window".into());
            }
            assert_unchanged(active, peer, expected, peer_before, visible, was_zoomed)
        })();
        check(
            &mut failures,
            format!("{} startup off-screen relocation", active.label()),
            relocated_case,
        );

        let half_pixel_case = (|| {
            let (primary_width, primary_height, _, _, max_x) = screens(active)?;
            let requested_x = (max_x + 2000.0).ceil() as i32;
            let requested_y = 120;
            let selected = nearest_screen(active, requested_x, requested_y)
                .map_err(|error| error.to_string())?;
            let width = if selected.width % 2 == 0 { 701 } else { 700 };
            let height = if selected.height % 2 == 0 { 451 } else { 450 };
            let requested = bounds(requested_x, requested_y, width, height);
            let peer_before = crate::macos_saved::native_bounds(peer)?;
            let visible = active.is_visible().map_err(|error| error.to_string())?;
            let was_zoomed = zoomed(active)?;
            let expected = bounds(
                ((primary_width - width as f64) / 2.0).round() as i32,
                ((primary_height - height as f64) / 2.0).round() as i32,
                width,
                height,
            );
            let result = crate::platform::window_state_macos::restore_startup(
                active,
                SavedWindowState {
                    bounds: Some(requested),
                    maximized: false,
                },
            )
            .map_err(|error| error.to_string())?;
            if result != Some(expected) {
                return Err(format!("half-pixel startup result {result:?}, expected primary fallback {expected:?}; selected screen {selected:?}"));
            }
            assert_unchanged(active, peer, expected, peer_before, visible, was_zoomed)
        })();
        check(
            &mut failures,
            format!("{} startup half-pixel primary fallback", active.label()),
            half_pixel_case,
        );

        let maximized_case = (|| {
            let (_, _, _, _, max_x) = screens(active)?;
            let requested = bounds((max_x + 2000.0).ceil() as i32, 120, 700, 450);
            let peer_before = crate::macos_saved::native_bounds(peer)?;
            let visible = active.is_visible().map_err(|error| error.to_string())?;
            let was_zoomed = zoomed(active)?;
            let result = crate::platform::window_state_macos::restore_startup(
                active,
                SavedWindowState {
                    bounds: Some(requested),
                    maximized: true,
                },
            )
            .map_err(|error| error.to_string())?;
            if result != Some(requested) {
                return Err(format!(
                    "maximized startup bounds differ: {result:?} vs {requested:?}"
                ));
            }
            if zoomed(active)? {
                return Err("maximized startup restore applied zoom before presentation".into());
            }
            assert_unchanged(active, peer, requested, peer_before, visible, was_zoomed)
        })();
        check(
            &mut failures,
            format!("{} startup maximized off-screen bounds", active.label()),
            maximized_case,
        );

        check(
            &mut failures,
            format!("{} startup fixture restores original frame", active.label()),
            restore_original(active, original),
        );
    }
    failures
}
