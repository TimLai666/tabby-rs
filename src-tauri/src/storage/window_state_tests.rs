use super::*;
use tempfile::tempdir;

fn bounds(x: f64, y: f64, width: f64, height: f64) -> WindowBoundaries {
    WindowBoundaries {
        x,
        y,
        width,
        height,
    }
}

fn monitor(x: f64, y: f64, width: f64, height: f64) -> MonitorRect {
    MonitorRect {
        x,
        y,
        width,
        height,
    }
}

fn saved(window_boundaries: WindowBoundaries, maximized: bool) -> WindowStateFile {
    WindowStateFile {
        window_boundaries: Some(window_boundaries),
        maximized,
    }
}

fn two_monitors() -> [MonitorRect; 2] {
    [
        monitor(0.0, 0.0, 1920.0, 1080.0),
        monitor(1920.0, 0.0, 1280.0, 1024.0),
    ]
}

#[test]
fn missing_file_loads_default() {
    let temp = tempdir().unwrap();
    let state = load_window_state(&temp.path().join("window.json")).unwrap();
    assert_eq!(state, WindowStateFile::default());
    assert_eq!(state.window_boundaries, None);
    assert!(!state.maximized);
}

#[test]
fn loads_bounds_and_maximized_from_camel_case_file() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    std::fs::write(
        &path,
        r#"{"windowBoundaries":{"x":10.5,"y":20,"width":800,"height":600},"maximized":true}"#,
    )
    .unwrap();
    let state = load_window_state(&path).unwrap();
    assert_eq!(
        state.window_boundaries,
        Some(bounds(10.5, 20.0, 800.0, 600.0))
    );
    assert!(state.maximized);
}

#[test]
fn file_with_only_maximized_keeps_bounds_absent() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    std::fs::write(&path, r#"{"maximized":true}"#).unwrap();
    let state = load_window_state(&path).unwrap();
    assert_eq!(state.window_boundaries, None);
    assert!(state.maximized);
}

#[test]
fn empty_object_loads_default() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    std::fs::write(&path, "{}").unwrap();
    assert_eq!(
        load_window_state(&path).unwrap(),
        WindowStateFile::default()
    );
}

#[test]
fn invalid_json_is_an_error() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(load_window_state(&path).is_err());
}

#[test]
fn file_size_limit_is_64_kib_inclusive() {
    let limit = 64 * 1024;
    // `{"pad":"` and `"}` together take 10 bytes; the padding fills the rest.
    let at_limit = format!(r#"{{"pad":"{}"}}"#, "a".repeat(limit - 10));
    assert_eq!(at_limit.len(), limit);
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    std::fs::write(&path, &at_limit).unwrap();
    assert_eq!(
        load_window_state(&path).unwrap(),
        WindowStateFile::default()
    );

    let over_limit = format!(r#"{{"pad":"{}"}}"#, "a".repeat(limit - 9));
    assert_eq!(over_limit.len(), limit + 1);
    std::fs::write(&path, &over_limit).unwrap();
    assert!(load_window_state(&path).is_err());
}

#[test]
fn save_then_load_round_trips_bounds_and_maximized() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    let state = saved(bounds(-40.0, 12.5, 1024.0, 768.0), true);
    save_window_state(&path, &state).unwrap();
    assert_eq!(load_window_state(&path).unwrap(), state);
}

#[test]
fn save_then_load_round_trips_default_state() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    save_window_state(&path, &WindowStateFile::default()).unwrap();
    assert_eq!(
        load_window_state(&path).unwrap(),
        WindowStateFile::default()
    );
}

#[test]
fn save_writes_camel_case_keys_and_trailing_newline() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("window.json");
    save_window_state(&path, &saved(bounds(1.0, 2.0, 3.0, 4.0), false)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(text.contains("\"windowBoundaries\""));
    assert!(!text.contains("window_boundaries"));
    assert!(text.contains("\"maximized\": false"));
}

#[test]
fn no_saved_bounds_keeps_stock_geometry() {
    let state = WindowStateFile {
        window_boundaries: None,
        maximized: false,
    };
    assert_eq!(restored_placement(&state, &two_monitors()), None);
}

#[test]
fn maximized_without_bounds_still_requests_maximize() {
    let state = WindowStateFile {
        window_boundaries: None,
        maximized: true,
    };
    assert_eq!(
        restored_placement(&state, &two_monitors()),
        Some(Placement {
            bounds: None,
            maximized: true,
        })
    );
}

#[test]
fn oversized_bounds_are_clamped_to_the_nearest_monitor() {
    let placement = restored_placement(
        &saved(bounds(100.0, 100.0, 1e9, 1e9), false),
        &two_monitors(),
    );
    assert_eq!(
        placement,
        Some(Placement {
            bounds: Some(bounds(100.0, 100.0, 1920.0, 1080.0)),
            maximized: false,
        })
    );
}

#[test]
fn empty_monitor_list_keeps_saved_placement_unchanged() {
    let saved_bounds = bounds(5000.0, 100.0, 800.0, 600.0);
    assert_eq!(
        restored_placement(&saved(saved_bounds, false), &[]),
        Some(Placement {
            bounds: Some(saved_bounds),
            maximized: false,
        })
    );
    assert_eq!(
        restored_placement(&saved(saved_bounds, true), &[]),
        Some(Placement {
            bounds: Some(saved_bounds),
            maximized: true,
        })
    );
}

#[test]
fn rectangle_on_nearest_monitor_is_kept() {
    let saved_bounds = bounds(100.0, 50.0, 800.0, 600.0);
    assert_eq!(
        restored_placement(&saved(saved_bounds, false), &two_monitors()),
        Some(Placement {
            bounds: Some(saved_bounds),
            maximized: false,
        })
    );
}

#[test]
fn off_screen_rectangle_is_centered_on_nearest_monitor() {
    // Nearest monitor to (5000, 100) is the secondary at 1920,0 1280x1024.
    let state = saved(bounds(5000.0, 100.0, 800.0, 600.0), false);
    assert_eq!(
        restored_placement(&state, &two_monitors()),
        Some(Placement {
            bounds: Some(bounds(2160.0, 212.0, 800.0, 600.0)),
            maximized: false,
        })
    );
}

#[test]
fn off_screen_maximized_rectangle_keeps_saved_bounds() {
    let saved_bounds = bounds(5000.0, 100.0, 800.0, 600.0);
    assert_eq!(
        restored_placement(&saved(saved_bounds, true), &two_monitors()),
        Some(Placement {
            bounds: Some(saved_bounds),
            maximized: true,
        })
    );
}

#[test]
fn partial_overlap_counts_as_intersecting() {
    // The top-left corner (-50, -30) is outside the primary monitor, which is the
    // nearest one; the 200x200 rectangle still overlaps it at 0..150 by 0..170.
    let saved_bounds = bounds(-50.0, -30.0, 200.0, 200.0);
    assert_eq!(
        restored_placement(&saved(saved_bounds, false), &two_monitors()),
        Some(Placement {
            bounds: Some(saved_bounds),
            maximized: false,
        })
    );
}

#[test]
fn zero_negative_or_non_finite_size_is_rejected() {
    let sizes = [
        (0.0, 600.0),
        (800.0, 0.0),
        (-1.0, 600.0),
        (800.0, -1.0),
        (f64::NAN, 600.0),
        (800.0, f64::NAN),
        (f64::INFINITY, 600.0),
        (800.0, f64::NEG_INFINITY),
    ];
    for (width, height) in sizes {
        let state = saved(bounds(100.0, 50.0, width, height), false);
        assert_eq!(
            restored_placement(&state, &two_monitors()),
            None,
            "width={width} height={height}"
        );
    }
}

#[test]
fn equal_distance_keeps_first_monitor() {
    // (1500, 500) is 500 away from both monitors' edges, and the 100x100 rectangle
    // touches neither, so the first monitor's center is used.
    let monitors = [
        monitor(0.0, 0.0, 1000.0, 1000.0),
        monitor(2000.0, 0.0, 1000.0, 1000.0),
    ];
    let state = saved(bounds(1500.0, 500.0, 100.0, 100.0), false);
    assert_eq!(
        restored_placement(&state, &monitors),
        Some(Placement {
            bounds: Some(bounds(450.0, 450.0, 100.0, 100.0)),
            maximized: false,
        })
    );
}
