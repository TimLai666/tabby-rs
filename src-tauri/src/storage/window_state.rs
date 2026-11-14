//! Persisted window placement, matching upstream Tabby's `window.json`
//! (`electron-config` store named `window` with `windowBoundaries` and `maximized`).

use std::path::Path;

use crate::error::AppError;

use super::atomic_file::{atomic_write, read_optional_regular_file};

/// Window placement in the platform's native placement space: physical
/// pixels on Windows and Linux, points on macOS. Size semantics follow each
/// platform's bounds API: outer frame on Windows and macOS, GTK content on Linux.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowBoundaries {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowStateFile {
    pub window_boundaries: Option<WindowBoundaries>,
    pub maximized: bool,
}

/// A monitor rectangle in the same placement units as [`WindowBoundaries`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a restored window should be placed before it is shown. `bounds` is
/// `None` when only a maximize request survives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub bounds: Option<WindowBoundaries>,
    pub maximized: bool,
}

/// Reads `window.json`. A missing file yields the default (no saved placement).
/// Unreadable or invalid content also yields the default so startup never fails
/// because of this file; the problem is reported through the returned error
/// only when the file exists and cannot be parsed.
pub fn load_window_state(path: &Path) -> Result<WindowStateFile, AppError> {
    let Some(bytes) = read_optional_regular_file(path)? else {
        return Ok(WindowStateFile::default());
    };
    if bytes.len() > 64 * 1024 {
        return Err(AppError::InvalidData("window.json is too large".into()));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn save_window_state(path: &Path, state: &WindowStateFile) -> Result<(), AppError> {
    let mut bytes = serde_json::to_vec_pretty(state)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

/// Decides the startup placement from a saved state and the current monitors,
/// following upstream `app/lib/window.ts` with two recorded differences:
/// off-screen windows are centered on the nearest monitor's own rectangle
/// (upstream centers on the primary origin), and sizes are clamped to that
/// monitor so a corrupt file cannot request an enormous window.
///
/// Expected results:
/// - no `window_boundaries` and not maximized → `None` (stock geometry stays).
/// - no `window_boundaries` but maximized → `Some(Placement { bounds: None,
///   maximized: true })` (upstream maximizes regardless of saved bounds).
/// - non-finite or non-positive width/height → treated like missing bounds.
/// - monitors is empty → saved bounds unchanged.
/// - the saved rectangle intersects the monitor nearest to its top-left corner
///   (distance from the point to the monitor rectangle, ties keep the first
///   monitor) → saved bounds unchanged.
/// - it does not intersect that monitor and `maximized` is false → same size,
///   centered on that monitor (`x = monitor.x + (monitor.width - width) / 2`,
///   same for `y`).
/// - it does not intersect and `maximized` is true → saved bounds unchanged
///   (the maximize request decides the visible placement).
/// - width or height larger than the nearest monitor → reduced to that
///   monitor's size before the intersection and centering rules apply.
pub fn restored_placement(saved: &WindowStateFile, monitors: &[MonitorRect]) -> Option<Placement> {
    let maximized = saved.maximized;
    let usable = saved.window_boundaries.filter(|bounds| {
        bounds.width.is_finite()
            && bounds.height.is_finite()
            && bounds.x.is_finite()
            && bounds.y.is_finite()
            && bounds.width > 0.0
            && bounds.height > 0.0
    });
    let Some(mut bounds) = usable else {
        return maximized.then_some(Placement {
            bounds: None,
            maximized,
        });
    };
    let Some(nearest) = nearest_monitor(bounds.x, bounds.y, monitors) else {
        return Some(Placement {
            bounds: Some(bounds),
            maximized,
        });
    };
    bounds.width = bounds.width.min(nearest.width);
    bounds.height = bounds.height.min(nearest.height);
    if maximized || intersects(&bounds, nearest) {
        return Some(Placement {
            bounds: Some(bounds),
            maximized,
        });
    }
    Some(Placement {
        bounds: Some(WindowBoundaries {
            x: nearest.x + (nearest.width - bounds.width) / 2.0,
            y: nearest.y + (nearest.height - bounds.height) / 2.0,
            width: bounds.width,
            height: bounds.height,
        }),
        maximized,
    })
}

fn nearest_monitor(x: f64, y: f64, monitors: &[MonitorRect]) -> Option<&MonitorRect> {
    let mut best: Option<(&MonitorRect, f64)> = None;
    for monitor in monitors {
        let dx = (monitor.x - x)
            .max(0.0)
            .max(x - (monitor.x + monitor.width));
        let dy = (monitor.y - y)
            .max(0.0)
            .max(y - (monitor.y + monitor.height));
        let distance = dx * dx + dy * dy;
        if best.is_none_or(|(_, current)| distance < current) {
            best = Some((monitor, distance));
        }
    }
    best.map(|(monitor, _)| monitor)
}

fn intersects(bounds: &WindowBoundaries, monitor: &MonitorRect) -> bool {
    let (left1, top1, right1, bottom1) = (
        bounds.x,
        bounds.y,
        bounds.x + bounds.width,
        bounds.y + bounds.height,
    );
    let (left2, top2, right2, bottom2) = (
        monitor.x,
        monitor.y,
        monitor.x + monitor.width,
        monitor.y + monitor.height,
    );
    !(left2 > right1 || right2 < left1 || top2 > bottom1 || bottom2 < top1)
}

#[cfg(test)]
#[path = "window_state_tests.rs"]
mod tests;
