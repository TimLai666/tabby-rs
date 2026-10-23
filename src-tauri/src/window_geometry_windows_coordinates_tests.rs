use super::{Monitor, Rect, WindowsCoordinates};

// Expected bounds follow the fixed Chromium ScreenWin display-layout rules.

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn monitor(id: u32, physical: Rect, scale: f32) -> Monitor {
    Monitor {
        id,
        physical,
        scale,
    }
}

fn layout(monitors: Vec<Monitor>) -> WindowsCoordinates {
    WindowsCoordinates::new(monitors).expect("valid monitor layout")
}

fn large_unit_scale_layout() -> WindowsCoordinates {
    layout(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(2, rect(33_554_434, 0, 100, 100), 1.0),
    ])
}

#[test]
fn validates_monitor_input_and_reports_unknown_ids() {
    assert!(WindowsCoordinates::new(vec![]).is_err());
    assert!(WindowsCoordinates::new(vec![monitor(1, rect(10, 10, 100, 100), 1.0)]).is_err());
    assert!(WindowsCoordinates::new(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(1, rect(100, 0, 100, 100), 1.0),
    ])
    .is_err());

    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(WindowsCoordinates::new(vec![monitor(1, rect(0, 0, 100, 100), scale)]).is_err());
    }
    for physical in [
        rect(0, 0, 0, 100),
        rect(0, 0, 100, 0),
        rect(0, 0, -1, 100),
        rect(0, 0, 100, -1),
    ] {
        assert!(WindowsCoordinates::new(vec![monitor(1, physical, 1.0)]).is_err());
    }

    let coordinates = layout(vec![monitor(7, rect(0, 0, 100, 100), 1.0)]);
    assert_eq!(coordinates.logical_bounds(99), None);
    assert_eq!(coordinates.physical_to_logical(rect(0, 0, 1, 1), 99), None);
}

#[test]
fn lays_out_right_monitor_and_scales_global_far_edges() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 1920, 1080), 1.0),
        monitor(2, rect(1920, 0, 1920, 1080), 2.0),
    ]);
    assert_eq!(
        coordinates.logical_bounds(2),
        Some(rect(1920, 540, 960, 540))
    );
    assert_eq!(
        coordinates.physical_to_logical(rect(2000, 20, 100, 100), 2),
        Some(rect(1960, 550, 50, 50))
    );
    assert_eq!(
        coordinates.physical_to_logical(rect(2001, 21, 100, 100), 2),
        Some(rect(1960, 550, 51, 51))
    );
    assert_eq!(
        coordinates.logical_to_physical(rect(1960, 550, 51, 51)),
        rect(2000, 20, 102, 102)
    );
    assert_eq!(
        coordinates.physical_to_logical(rect(2000, 20, 0, 0), 2),
        Some(rect(1960, 550, 0, 0))
    );
}

#[test]
fn lays_out_left_monitor_with_negative_fractional_edges() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 1920, 1080), 1.0),
        monitor(2, rect(-1920, 0, 1920, 1080), 2.0),
    ]);
    assert_eq!(
        coordinates.logical_bounds(2),
        Some(rect(-960, 540, 960, 540))
    );
    assert_eq!(
        coordinates.physical_to_logical(rect(-101, 21, 100, 100), 2),
        Some(rect(-51, 550, 51, 51))
    );
    assert_eq!(
        coordinates.logical_to_physical(rect(-51, 550, 51, 51)),
        rect(-102, 20, 102, 102)
    );
}

#[test]
fn preserves_mixed_scale_and_odd_primary_edge_alignment() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 1920, 1200), 2.0),
        monitor(2, rect(1920, 0, 800, 600), 1.0),
    ]);
    assert_eq!(coordinates.logical_bounds(2), Some(rect(960, 0, 800, 600)));
    assert_eq!(
        coordinates.physical_to_logical(rect(2000, 496, 100, 100), 2),
        Some(rect(1040, 496, 100, 100))
    );

    let odd = layout(vec![
        monitor(1, rect(0, 0, 1921, 1080), 1.0),
        monitor(2, rect(1921, 0, 1920, 1080), 2.0),
    ]);
    assert_eq!(odd.logical_bounds(2), Some(rect(1921, 540, 961, 540)));
}

#[test]
fn fractional_scale_uses_floor_for_origins_and_encloses_far_edges() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 150, 150), 1.0),
        monitor(2, rect(-150, 0, 150, 150), 1.5),
    ]);
    assert_eq!(
        coordinates.logical_bounds(2),
        Some(rect(-100, 50, 100, 100))
    );
    assert_eq!(
        coordinates.physical_to_logical(rect(-149, 1, 10, 10), 2),
        Some(rect(-100, 50, 8, 8))
    );

    let quarter = layout(vec![
        monitor(1, rect(0, 0, 125, 125), 1.0),
        monitor(2, rect(-125, 0, 125, 125), 1.25),
    ]);
    assert_eq!(quarter.logical_bounds(2), Some(rect(-100, 25, 100, 100)));
    assert_eq!(
        quarter.physical_to_logical(rect(-124, 1, 10, 10), 2),
        Some(rect(-100, 25, 9, 9))
    );
}

#[test]
fn aligns_right_monitor_to_primary_far_edge_and_supports_vertical_neighbors() {
    let horizontal = layout(vec![
        monitor(1, rect(0, 0, 1920, 1080), 1.0),
        monitor(2, rect(1920, 480, 800, 600), 2.0),
    ]);
    assert_eq!(
        horizontal.logical_bounds(2),
        Some(rect(1920, 780, 400, 300))
    );

    let vertical = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(2, rect(0, 100, 100, 100), 2.0),
        monitor(3, rect(0, -100, 100, 100), 2.0),
    ]);
    assert_eq!(vertical.logical_bounds(2), Some(rect(50, 100, 50, 50)));
    assert_eq!(vertical.logical_bounds(3), Some(rect(50, -50, 50, 50)));
}

#[test]
fn point_and_rectangle_choose_screens_by_their_distinct_rules() {
    let primary_first = layout(vec![
        monitor(1, rect(0, 0, 100, 200), 1.0),
        monitor(2, rect(100, 0, 100, 100), 2.0),
    ]);
    assert_eq!(primary_first.logical_point_to_physical(180, 20), (180, 20));
    assert_eq!(
        primary_first.logical_to_physical(rect(180, 20, 10, 10)),
        rect(260, 40, 20, 20)
    );
    assert_eq!(
        primary_first.logical_to_physical(rect(90, 0, 20, 20)),
        rect(90, 0, 20, 20)
    );
    assert_eq!(
        primary_first.logical_to_physical(rect(95, 0, 20, 20)),
        rect(90, 0, 40, 40)
    );

    let right_first = layout(vec![
        monitor(2, rect(100, 0, 100, 100), 2.0),
        monitor(1, rect(0, 0, 100, 200), 1.0),
    ]);
    assert_eq!(
        right_first.logical_to_physical(rect(90, 0, 20, 20)),
        rect(80, 0, 40, 40)
    );
    assert_eq!(right_first.logical_point_to_physical(100, 20), (100, 40));
}

#[test]
fn physical_to_logical_uses_native_selected_monitor_id() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 200), 1.0),
        monitor(2, rect(100, 0, 100, 100), 2.0),
    ]);
    assert_eq!(
        coordinates.physical_to_logical(rect(90, 20, 20, 20), 2),
        Some(rect(95, 10, 10, 10))
    );
}

#[test]
fn zero_size_rectangle_selection_uses_chromiums_rotated_rect_metric() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 200), 1.0),
        monitor(2, rect(100, 0, 100, 100), 2.0),
    ]);
    assert_eq!(coordinates.logical_bounds(2), Some(rect(100, 0, 50, 50)));
    assert_eq!(
        coordinates.logical_to_physical(rect(90, 20, 0, 0)),
        rect(80, 40, 0, 0)
    );
}

#[test]
fn equal_deintersection_choices_follow_full_monitor_id_order() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 2.0),
        monitor(30, rect(100, 0, 100, 50), 1.0),
        monitor(20, rect(100, 50, 100, 50), 1.0),
    ]);
    assert_eq!(coordinates.logical_bounds(1), Some(rect(0, 0, 50, 50)));
    assert_eq!(coordinates.logical_bounds(30), Some(rect(50, 50, 100, 50)));
    assert_eq!(coordinates.logical_bounds(20), Some(rect(50, 0, 100, 50)));
}

#[test]
fn touching_tree_uses_bidirectional_partition_and_lifo_order() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 4.0),   // A
        monitor(2, rect(100, 0, 100, 50), 1.0),  // B
        monitor(4, rect(200, 0, 100, 100), 1.0), // D
        monitor(3, rect(100, 50, 100, 50), 1.0), // C
        monitor(5, rect(300, 0, 100, 100), 1.0), // E
    ]);
    assert_eq!(coordinates.logical_bounds(1), Some(rect(0, 0, 25, 25)));
    assert_eq!(coordinates.logical_bounds(2), Some(rect(25, 0, 100, 50)));
    assert_eq!(coordinates.logical_bounds(3), Some(rect(25, -50, 100, 50)));
    assert_eq!(coordinates.logical_bounds(4), Some(rect(125, 0, 100, 100)));
    assert_eq!(coordinates.logical_bounds(5), Some(rect(225, 0, 100, 100)));
}

#[test]
fn disconnected_screens_keep_their_independently_scaled_origins() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(2, rect(300, 200, 100, 100), 2.0),
    ]);
    assert_eq!(coordinates.logical_bounds(1), Some(rect(0, 0, 100, 100)));
    assert_eq!(coordinates.logical_bounds(2), Some(rect(150, 100, 50, 50)));
}

#[test]
fn covers_parent_end_alignment_keeps_the_expected_direction() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(2, rect(100, -50, 100, 200), 2.0),
    ]);
    assert_eq!(coordinates.logical_bounds(2), Some(rect(100, -25, 50, 100)));
}

#[test]
fn large_offset_preserves_chromiums_float32_operation_order() {
    let coordinates = layout(vec![
        monitor(1, rect(0, 0, 100, 20_000_001), 1.3),
        monitor(2, rect(100, 1_234_567, 100, 1), 1.0),
    ]);
    assert_eq!(
        coordinates.logical_bounds(2),
        Some(rect(77, 949_667, 100, 1))
    );
}

#[test]
fn large_scale_layout_preserves_chromium_float32_rounding() {
    let half_scale = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(2, rect(16_777_217, 0, 100, 100), 2.0),
    ]);
    assert_eq!(
        half_scale.logical_bounds(2),
        Some(rect(8_388_608, 0, 50, 50))
    );
}

#[test]
fn large_scale_physical_to_logical_uses_float32_relative_arithmetic() {
    let half_scale = layout(vec![
        monitor(1, rect(0, 0, 100, 100), 1.0),
        monitor(2, rect(16_777_217, 0, 100, 100), 2.0),
    ]);
    assert_eq!(
        half_scale.physical_to_logical(rect(16_777_219, 0, 4, 4), 2),
        Some(rect(8_388_610, 0, 2, 2))
    );
}

#[test]
fn large_unit_scale_physical_to_logical_uses_float32_relative_arithmetic() {
    let unit_scale = large_unit_scale_layout();
    let bounds = rect(33_554_435, 1, 4, 4);
    assert_eq!(
        unit_scale.physical_to_logical(bounds, 2),
        Some(rect(33_554_436, 1, 4, 4))
    );
}

#[test]
fn large_unit_scale_logical_to_physical_uses_float32_relative_arithmetic() {
    let unit_scale = large_unit_scale_layout();
    let bounds = rect(33_554_435, 1, 4, 4);
    assert_eq!(
        unit_scale.logical_to_physical(bounds),
        rect(33_554_436, 1, 4, 4)
    );
}

#[test]
fn large_unit_scale_point_conversion_uses_float32_relative_arithmetic() {
    let unit_scale = large_unit_scale_layout();
    assert_eq!(
        unit_scale.logical_point_to_physical(33_554_435, 1),
        (33_554_436, 1)
    );
}
