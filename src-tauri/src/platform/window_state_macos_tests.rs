use super::{nearest_screen_in_frames, AppError, LogicalBounds, NSRect};
use objc2_foundation::{NSPoint, NSSize};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

#[test]
fn empty_screens_returns_io_error() {
    let screens: [NSRect; 0] = [];
    assert!(matches!(
        nearest_screen_in_frames(&screens, 0, 0),
        Err(AppError::Io(_))
    ));
}

#[test]
fn single_screen_returns_itself_with_toward_zero_casts() {
    let screens = [rect(-3.2, 4.7, 10.3, 20.6)];
    let bounds = nearest_screen_in_frames(&screens, 999, -999).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: -3,
            y: 4,
            width: 10,
            height: 20
        }
    );
}

#[test]
fn primary_and_adjacent_selects_second() {
    let screens = [rect(0.0, 0.0, 100.0, 100.0), rect(100.0, 0.0, 100.0, 100.0)];
    let bounds = nearest_screen_in_frames(&screens, 150, 20).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: 100,
            y: 0,
            width: 100,
            height: 100
        }
    );
}

#[test]
fn left_negative_and_above_screen_positions() {
    let screens = [
        rect(0.0, 0.0, 100.0, 100.0),
        rect(-100.0, 0.0, 100.0, 100.0),
        rect(0.0, 100.0, 100.0, 100.0),
    ];
    assert_eq!(
        nearest_screen_in_frames(&screens, -50, 50).unwrap(),
        LogicalBounds {
            x: -100,
            y: 0,
            width: 100,
            height: 100
        }
    );
    assert_eq!(
        nearest_screen_in_frames(&screens, 50, -50).unwrap(),
        LogicalBounds {
            x: 0,
            y: -100,
            width: 100,
            height: 100
        }
    );
}

#[test]
fn overlapping_rectangles_first_containment_wins() {
    let screens = [rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 100.0, 100.0)];
    let bounds = nearest_screen_in_frames(&screens, 60, 20).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100
        }
    );
}

#[test]
fn equal_outside_corner_distances_preserve_first() {
    let screens = [rect(0.0, 0.0, 100.0, 100.0), rect(200.0, 0.0, 100.0, 100.0)];
    let bounds = nearest_screen_in_frames(&screens, 150, 50).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100
        }
    );
}

#[test]
fn corner_metric_not_rectangle_distance() {
    let screens = [
        rect(0.0, 0.0, 100.0, 1000.0),
        rect(150.0, 450.0, 100.0, 100.0),
    ];
    let bounds = nearest_screen_in_frames(&screens, 110, 500).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: 150,
            y: 450,
            width: 100,
            height: 100
        }
    );
}

#[test]
fn secondary_uses_flipped_origin() {
    let screens = [
        rect(0.0, 10.0, 100.0, 100.0),
        rect(100.0, 10.0, 100.0, 100.0),
    ];
    let bounds = nearest_screen_in_frames(&screens, 150, 30).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: 100,
            y: 0,
            width: 100,
            height: 100
        }
    );
}

#[test]
fn fractional_screen_toward_zero_casts() {
    let screens = [rect(0.0, 0.0, 100.0, 100.0), rect(-10.7, 5.3, 20.4, 30.6)];
    let bounds = nearest_screen_in_frames(&screens, -5, 90).unwrap();
    assert_eq!(
        bounds,
        LogicalBounds {
            x: -10,
            y: 64,
            width: 20,
            height: 30
        }
    );
}
