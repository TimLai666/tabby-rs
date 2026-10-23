use super::{nearest_monitor, startup_bounds, StartupBounds, StartupPosition};
use crate::window_geometry::persistence::{LogicalBounds, SavedWindowState};

fn bounds(x: i32, y: i32, width: u32, height: u32) -> LogicalBounds {
    LogicalBounds {
        x,
        y,
        width,
        height,
    }
}

fn saved(bounds: Option<LogicalBounds>, maximized: bool) -> SavedWindowState {
    SavedWindowState { bounds, maximized }
}

#[test]
fn nearest_monitor_returns_none_for_empty_monitor_list() {
    assert_eq!(nearest_monitor(&[], 0, 0), None);
}

#[test]
fn nearest_monitor_uses_right_edge_distance_without_plus_one() {
    let monitors = [bounds(11, 0, 10, 10), bounds(0, 0, 10, 10)];
    assert_eq!(nearest_monitor(&monitors, 10, 5), Some(1));
}

#[test]
fn nearest_monitor_uses_bottom_edge_distance_without_plus_one() {
    let monitors = [bounds(0, 11, 10, 10), bounds(0, 0, 10, 10)];
    assert_eq!(nearest_monitor(&monitors, 5, 10), Some(1));
}

#[test]
fn nearest_monitor_prefers_containment_to_another_monitors_edge_distance_zero() {
    let monitors = [bounds(0, 0, 10, 10), bounds(8, 0, 5, 10)];
    assert_eq!(nearest_monitor(&monitors, 10, 5), Some(1));
}

#[test]
fn nearest_monitor_uses_manhattan_rectangle_distance() {
    let monitors = [bounds(6, 6, 1, 1), bounds(10, -1, 1, 2)];
    assert_eq!(nearest_monitor(&monitors, 0, 0), Some(1));
}

#[test]
fn nearest_monitor_uses_half_open_bounds_and_first_match_for_overlaps() {
    let monitors = [bounds(0, 0, 10, 10), bounds(5, 5, 10, 10)];
    assert_eq!(nearest_monitor(&monitors, 5, 5), Some(0));
    assert_eq!(nearest_monitor(&monitors, 10, 5), Some(1));
    assert_eq!(nearest_monitor(&monitors, 5, 10), Some(1));
}

#[test]
fn nearest_monitor_keeps_enumeration_order_for_equal_distances() {
    let monitors = [bounds(-10, 0, 5, 5), bounds(5, 0, 5, 5)];
    assert_eq!(nearest_monitor(&monitors, 0, 2), Some(0));
}

#[test]
fn startup_bounds_without_saved_bounds_does_not_require_monitors() {
    for maximized in [false, true] {
        assert_eq!(startup_bounds(saved(None, maximized), &[]), Ok(None));
    }
}

#[test]
fn startup_bounds_with_saved_bounds_requires_a_monitor() {
    assert!(startup_bounds(saved(Some(bounds(0, 0, 800, 600)), false), &[]).is_err());
}

#[test]
fn startup_bounds_preserves_size_and_centers_even_sized_offscreen_window() {
    let result = startup_bounds(
        saved(Some(bounds(3000, 2000, 700, 450)), false),
        &[bounds(0, 0, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 700,
            height: 450,
            position: StartupPosition::Explicit { x: 610, y: 315 },
        }))
    );
}

#[test]
fn startup_bounds_uses_default_center_when_center_coordinates_are_half_pixels() {
    let result = startup_bounds(
        saved(Some(bounds(3000, 2000, 701, 451)), false),
        &[bounds(0, 0, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 701,
            height: 451,
            position: StartupPosition::DefaultCenter,
        }))
    );
}

#[test]
fn startup_bounds_uses_default_center_when_only_one_coordinate_is_half_pixel() {
    let result = startup_bounds(
        saved(Some(bounds(3000, 2000, 701, 450)), false),
        &[bounds(0, 0, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 701,
            height: 450,
            position: StartupPosition::DefaultCenter,
        }))
    );
}

#[test]
fn startup_bounds_centers_oversized_window_with_negative_even_coordinates() {
    let result = startup_bounds(
        saved(Some(bounds(3000, 2000, 2500, 1800)), false),
        &[bounds(0, 0, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 2500,
            height: 1800,
            position: StartupPosition::Explicit { x: -290, y: -360 },
        }))
    );
}

#[test]
fn startup_bounds_uses_default_center_for_half_pixel_oversized_window() {
    let result = startup_bounds(
        saved(Some(bounds(3000, 2000, 2501, 1801)), false),
        &[bounds(0, 0, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 2501,
            height: 1801,
            position: StartupPosition::DefaultCenter,
        }))
    );
}

#[test]
fn startup_bounds_keeps_touching_edges_and_explicit_zero_or_negative_positions() {
    let monitor = [bounds(0, 0, 1920, 1080)];
    for point in [(1920, 0), (0, 1080), (0, 0), (-1, -1)] {
        let result = startup_bounds(
            saved(Some(bounds(point.0, point.1, 800, 600)), false),
            &monitor,
        );
        assert_eq!(
            result,
            Ok(Some(StartupBounds {
                width: 800,
                height: 600,
                position: StartupPosition::Explicit {
                    x: point.0,
                    y: point.1,
                },
            })),
            "saved position ({}, {}) should be retained when touching or intersecting",
            point.0,
            point.1
        );
    }
}

#[test]
fn startup_bounds_uses_nearest_monitor_to_saved_top_left_before_resetting() {
    let monitors = [bounds(0, 0, 800, 600), bounds(2000, 1000, 1920, 1080)];
    let result = startup_bounds(saved(Some(bounds(2500, 500, 700, 400)), false), &monitors);
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 700,
            height: 400,
            position: StartupPosition::Explicit { x: 610, y: 340 },
        }))
    );
}

#[test]
fn startup_bounds_centering_omits_nonzero_monitor_origin() {
    let result = startup_bounds(
        saved(Some(bounds(9000, 9000, 700, 450)), false),
        &[bounds(-1920, 300, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 700,
            height: 450,
            position: StartupPosition::Explicit { x: 610, y: 315 },
        }))
    );
}

#[test]
fn startup_bounds_preserves_offscreen_position_when_maximized() {
    let result = startup_bounds(
        saved(Some(bounds(-4000, 2500, 1200, 900)), true),
        &[bounds(0, 0, 1920, 1080)],
    );
    assert_eq!(
        result,
        Ok(Some(StartupBounds {
            width: 1200,
            height: 900,
            position: StartupPosition::Explicit { x: -4000, y: 2500 },
        }))
    );
}

#[test]
fn startup_options_keep_zero_and_below_minimum_dimensions_for_native_construction() {
    for (width, height) in [(0, 0), (0, 100), (200, 0), (200, 100)] {
        assert_eq!(
            startup_bounds(
                saved(Some(bounds(100, 120, width, height)), false),
                &[bounds(0, 0, 1920, 1080)]
            ),
            Ok(Some(StartupBounds {
                width,
                height,
                position: StartupPosition::Explicit { x: 100, y: 120 },
            }))
        );
    }
}
