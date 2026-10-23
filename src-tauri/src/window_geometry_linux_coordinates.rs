//! Fixed 14e2d60 constructor options in Electron logical client coordinates.
use super::persistence::{LogicalBounds, SavedWindowState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupPosition {
    Explicit { x: i32, y: i32 },
    DefaultCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupBounds {
    pub width: u32,
    pub height: u32,
    pub position: StartupPosition,
}

/// Chromium's first containing monitor, otherwise its first minimum Manhattan
/// distance. Monitor enumeration order is significant, including edge ties.
pub(crate) fn nearest_monitor(monitors: &[LogicalBounds], x: i32, y: i32) -> Option<usize> {
    let x = i64::from(x);
    let y = i64::from(y);

    let mut nearest = None;
    let mut nearest_distance = i64::MAX;

    for (index, monitor) in monitors.iter().enumerate() {
        let left = i64::from(monitor.x);
        let top = i64::from(monitor.y);
        let right = left + i64::from(monitor.width);
        let bottom = top + i64::from(monitor.height);

        if left <= x && x < right && top <= y && y < bottom {
            return Some(index);
        }

        // Chromium's rectangle distance treats the right and bottom edges as
        // the zero-distance boundary, even though containment is half-open.
        let dx = if x < left {
            left - x
        } else if x >= right {
            x - right
        } else {
            0
        };
        let dy = if y < top {
            top - y
        } else if y >= bottom {
            y - bottom
        } else {
            0
        };
        let distance = dx + dy;

        if distance < nearest_distance {
            nearest = Some(index);
            nearest_distance = distance;
        }
    }

    nearest
}

/// Resolve only fixed constructor options. Native default centering, minimums,
/// actual monitor acquisition, presentation, and capture remain host operations.
/// No saved bounds means no override and does not require monitor acquisition.
pub(crate) fn startup_bounds(
    saved: SavedWindowState,
    monitors: &[LogicalBounds],
) -> Result<Option<StartupBounds>, String> {
    let Some(bounds) = saved.bounds else {
        return Ok(None);
    };
    let Some(monitor_index) = nearest_monitor(monitors, bounds.x, bounds.y) else {
        return Err("cannot resolve saved window bounds without a monitor".into());
    };

    let monitor = monitors[monitor_index];
    let bounds_left = i64::from(bounds.x);
    let bounds_top = i64::from(bounds.y);
    let bounds_right = bounds_left + i64::from(bounds.width);
    let bounds_bottom = bounds_top + i64::from(bounds.height);
    let monitor_left = i64::from(monitor.x);
    let monitor_top = i64::from(monitor.y);
    let monitor_right = monitor_left + i64::from(monitor.width);
    let monitor_bottom = monitor_top + i64::from(monitor.height);

    let disjoint = bounds_right < monitor_left
        || bounds_left > monitor_right
        || bounds_bottom < monitor_top
        || bounds_top > monitor_bottom;
    let position = if disjoint && !saved.maximized {
        let width = i64::from(bounds.width);
        let height = i64::from(bounds.height);
        let monitor_width = i64::from(monitor.width);
        let monitor_height = i64::from(monitor.height);

        if (monitor_width - width) % 2 != 0 || (monitor_height - height) % 2 != 0 {
            StartupPosition::DefaultCenter
        } else {
            StartupPosition::Explicit {
                x: (monitor_width / 2 - width / 2) as i32,
                y: (monitor_height / 2 - height / 2) as i32,
            }
        }
    } else {
        StartupPosition::Explicit {
            x: bounds.x,
            y: bounds.y,
        }
    };

    Ok(Some(StartupBounds {
        width: bounds.width,
        height: bounds.height,
        position,
    }))
}

#[cfg(test)]
#[path = "window_geometry_linux_coordinates_tests.rs"]
mod tests;
