use super::normalize_bounds;
use tauri::{PhysicalPosition, PhysicalSize};

#[test]
fn native_client_bounds_enclose_negative_and_positive_fractional_origins() {
    for (x, expected_x, expected_width) in [(-1, -2, 1622), (1, 0, 1622), (0, 0, 1620)] {
        assert_eq!(
            normalize_bounds(
                PhysicalPosition::new(x, 181),
                PhysicalSize::new(1620, 1220),
                2.0
            ),
            (
                PhysicalPosition::new(expected_x, 180),
                PhysicalSize::new(expected_width, 1222)
            ),
        );
    }
    assert_eq!(
        normalize_bounds(
            PhysicalPosition::new(-1, 181),
            PhysicalSize::new(810, 610),
            1.0
        ),
        (PhysicalPosition::new(-1, 181), PhysicalSize::new(810, 610)),
    );
}

#[test]
fn physically_integral_far_edges_do_not_gain_a_logical_pixel() {
    // Integral far edges stay exact, including sums beyond the origin's i32 range.
    for (x, width, scale, expected_x, expected_width) in [
        (-767, 1202, 3.0, -768, 1203),
        (-998, 2003, 5.0, -1000, 2005),
        (i32::MAX - 2, 10, 1.0, i32::MAX - 2, 10),
    ] {
        assert_eq!(
            normalize_bounds(
                PhysicalPosition::new(x, x),
                PhysicalSize::new(width, width),
                scale
            ),
            (
                PhysicalPosition::new(expected_x, expected_x),
                PhysicalSize::new(expected_width, expected_width)
            ),
        );
    }
}
