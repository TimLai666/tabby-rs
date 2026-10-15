use super::*;

fn metrics() -> CaptionMetrics {
    CaptionMetrics {
        client_width: 800,
        scale: 1.0,
        caption_height_dip: 23,
        border_thickness_dip: 8,
        maximized: false,
        fullscreen: false,
        minimizable: true,
        maximizable: true,
        closable: true,
    }
}

fn button(layout: &CaptionLayout, kind: CaptionButton) -> &CaptionButtonLayout {
    layout
        .buttons
        .iter()
        .find(|item| item.button == kind)
        .unwrap()
}

#[test]
fn restored_layout_matches_observed_physical_pixel_geometry() {
    let layout = layout(metrics()).unwrap();
    assert_eq!(
        layout.clip,
        CaptionRect {
            x: 663.0,
            y: 1.0,
            width: 136.0,
            height: 31.0
        }
    );
    assert_eq!(
        button(&layout, CaptionButton::Minimize).bounds,
        CaptionRect {
            x: 662.0,
            y: 1.0,
            width: 45.0,
            height: 31.0
        }
    );
    assert_eq!(
        button(&layout, CaptionButton::Maximize).bounds,
        CaptionRect {
            x: 707.0,
            y: 1.0,
            width: 46.0,
            height: 31.0
        }
    );
    assert_eq!(
        button(&layout, CaptionButton::Close).bounds,
        CaptionRect {
            x: 753.0,
            y: 1.0,
            width: 46.0,
            height: 31.0
        }
    );
}

#[test]
fn hit_test_uses_clip_and_button_half_open_edges() {
    let layout = layout(metrics()).unwrap();
    assert_eq!(layout.hit_test(662, 10), None);
    assert_eq!(layout.hit_test(663, 1), Some(CaptionButton::Minimize));
    assert_eq!(layout.hit_test(706, 31), Some(CaptionButton::Minimize));
    assert_eq!(layout.hit_test(707, 10), Some(CaptionButton::Maximize));
    assert_eq!(layout.hit_test(752, 10), Some(CaptionButton::Maximize));
    assert_eq!(layout.hit_test(753, 10), Some(CaptionButton::Close));
    assert_eq!(layout.hit_test(798, 31), Some(CaptionButton::Close));
    for (x, y) in [(799, 10), (700, 0), (700, 32)] {
        assert_eq!(layout.hit_test(x, y), None, "unexpected hit at ({x}, {y})");
    }
}

#[test]
fn maximized_layout_and_clip_keep_right_edge_inside_client() {
    let mut input = metrics();
    input.client_width = 1024;
    input.maximized = true;
    let layout = layout(input).unwrap();
    assert_eq!(
        layout.clip,
        CaptionRect {
            x: 887.0,
            y: 0.0,
            width: 137.0,
            height: 32.0
        }
    );
    assert_eq!(button(&layout, CaptionButton::Minimize).bounds.x, 887.0);
    assert_eq!(button(&layout, CaptionButton::Maximize).bounds.x, 932.0);
    assert_eq!(button(&layout, CaptionButton::Close).bounds.x, 978.0);
    assert_eq!(layout.hit_test(1023, 31), Some(CaptionButton::Close));
    assert_eq!(layout.hit_test(1024, 1), None);
}

#[test]
fn disabled_close_remains_hit_target_but_other_disabled_controls_are_omitted() {
    let mut input = metrics();
    input.closable = false;
    let close_disabled_layout = layout(input).unwrap();
    assert!(!button(&close_disabled_layout, CaptionButton::Close).enabled);
    assert_eq!(
        close_disabled_layout.hit_test(753, 10),
        Some(CaptionButton::Close)
    );

    input.minimizable = false;
    input.maximizable = false;
    let reduced = layout(input).unwrap();
    assert!(!reduced
        .buttons
        .iter()
        .any(|item| item.button == CaptionButton::Minimize));
    assert!(!reduced
        .buttons
        .iter()
        .any(|item| item.button == CaptionButton::Maximize));
    assert!(reduced
        .buttons
        .iter()
        .any(|item| item.button == CaptionButton::Close));
}

#[test]
fn fullscreen_hides_restored_controls_but_keeps_maximized_overlay() {
    let mut input = metrics();
    input.fullscreen = true;
    let hidden = layout(input).unwrap();
    assert!(hidden.buttons.is_empty());
    assert_eq!(hidden.hit_test(780, 10), None);

    input.maximized = true;
    let overlay = layout(input).unwrap();
    assert!(!overlay.buttons.is_empty());
    assert_eq!(overlay.hit_test(799, 10), Some(CaptionButton::Close));
}

#[test]
fn custom_caption_height_only_grows_system_caption_and_maximized_uses_system_height() {
    let mut input = metrics();
    input.caption_height_dip = 32;
    let restored = layout(input).unwrap();
    assert_eq!(restored.clip.height, 39.0);
    input.maximized = true;
    assert_eq!(layout(input).unwrap().clip.height, 32.0);

    input.maximized = false;
    input.caption_height_dip = 40;
    assert_eq!(layout(input).unwrap().clip.height, 47.0);
    input.maximized = true;
    assert_eq!(layout(input).unwrap().clip.height, 40.0);
}

#[test]
fn rejects_nonfinite_or_nonpositive_scale_and_negative_caption_metrics() {
    for scale in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        let mut input = metrics();
        input.scale = scale;
        assert!(
            matches!(layout(input), Err(AppError::InvalidArgument(_))),
            "scale {scale:?}"
        );
    }
    for (caption, border) in [(-1, 8), (23, -1)] {
        let mut input = metrics();
        input.caption_height_dip = caption;
        input.border_thickness_dip = border;
        assert!(matches!(layout(input), Err(AppError::InvalidArgument(_))));
    }
}

#[test]
fn empty_and_narrow_clients_never_hit_outside_the_client() {
    let mut input = metrics();
    input.client_width = 0;
    let empty = layout(input).unwrap();
    assert!(empty.buttons.is_empty());
    assert_eq!(empty.hit_test(0, 0), None);

    input.client_width = 10;
    let narrow = layout(input).unwrap();
    for x in -1..=10 {
        if !(0..10).contains(&x) {
            assert_eq!(narrow.hit_test(x, 1), None);
        }
    }
}

#[test]
fn narrow_client_close_button_hits_visible_pixels_but_not_resize_margin() {
    let mut input = metrics();
    input.client_width = 10;
    let layout = layout(input).unwrap();

    for x in 0..=8 {
        assert_eq!(layout.hit_test(x, 10), Some(CaptionButton::Close));
    }
    assert_eq!(layout.hit_test(9, 10), None);
}

#[test]
fn fractional_scales_preserve_ordered_nonoverlapping_visible_hits() {
    for scale in [1.25, 1.5, 2.0] {
        let mut input = metrics();
        input.scale = scale;
        let layout = layout(input).unwrap();
        assert!(layout.clip.width >= 0.0 && layout.clip.height >= 0.0);
        let mut previous_visible_right: Option<f64> = None;
        for item in &layout.buttons {
            let visible_left = item.bounds.x.max(layout.clip.x);
            let visible_right =
                (item.bounds.x + item.bounds.width).min(layout.clip.x + layout.clip.width);
            if visible_left < visible_right {
                if let Some(previous_right) = previous_visible_right {
                    assert!(
                        visible_left >= previous_right,
                        "visible button bounds overlap at scale {scale}"
                    );
                }
                previous_visible_right = Some(visible_right);

                let visible_top = item.bounds.y.max(layout.clip.y);
                let visible_bottom =
                    (item.bounds.y + item.bounds.height).min(layout.clip.y + layout.clip.height);
                let x = visible_left.ceil() as i32;
                let y = visible_top.ceil() as i32;
                if (x as f64) < visible_right && (y as f64) < visible_bottom {
                    assert_eq!(layout.hit_test(x, y), Some(item.button));
                }
            }
        }
    }
}

#[test]
fn pointer_move_press_and_matching_release_update_and_activate_once() {
    let mut pointer = CaptionPointerState::default();
    assert_eq!(
        pointer.update(CaptionPointerEvent::Move(Some(CaptionButton::Maximize))),
        None
    );
    assert_eq!(pointer.hovered, Some(CaptionButton::Maximize));
    assert_eq!(
        pointer.update(CaptionPointerEvent::Press(CaptionButton::Maximize)),
        None
    );
    assert_eq!(pointer.pressed, Some(CaptionButton::Maximize));
    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Maximize))),
        Some(CaptionButton::Maximize)
    );
    assert_eq!(pointer.pressed, None);
    assert_eq!(pointer.hovered, Some(CaptionButton::Maximize));
    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Maximize))),
        None
    );
}

#[test]
fn pointer_release_requires_same_button_and_current_hit() {
    let mut pointer = CaptionPointerState::default();
    pointer.update(CaptionPointerEvent::Press(CaptionButton::Close));
    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Maximize))),
        None
    );
    assert_eq!(pointer.pressed, None);
    assert_eq!(pointer.hovered, Some(CaptionButton::Maximize));

    pointer.update(CaptionPointerEvent::Press(CaptionButton::Minimize));
    assert_eq!(pointer.update(CaptionPointerEvent::Release(None)), None);
    assert_eq!(pointer.pressed, None);
    assert_eq!(pointer.hovered, None);
    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Minimize))),
        None
    );
}

#[test]
fn repeated_press_keeps_first_owner_until_a_release_clears_it() {
    let mut pointer = CaptionPointerState::default();
    pointer.update(CaptionPointerEvent::Press(CaptionButton::Minimize));
    pointer.update(CaptionPointerEvent::Press(CaptionButton::Close));
    assert_eq!(pointer.pressed, Some(CaptionButton::Minimize));

    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Close))),
        None
    );
    assert_eq!(pointer.pressed, None);
    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Minimize))),
        None
    );
}

#[test]
fn pointer_can_leave_and_return_to_pressed_button_before_release() {
    let mut pointer = CaptionPointerState::default();
    pointer.update(CaptionPointerEvent::Press(CaptionButton::Close));
    pointer.update(CaptionPointerEvent::Move(None));
    assert_eq!(pointer.hovered, None);
    assert_eq!(pointer.pressed, Some(CaptionButton::Close));
    pointer.update(CaptionPointerEvent::Move(Some(CaptionButton::Close)));
    assert_eq!(
        pointer.update(CaptionPointerEvent::Release(Some(CaptionButton::Close))),
        Some(CaptionButton::Close)
    );
}

#[test]
fn pointer_cancel_clears_both_states_and_states_are_independent() {
    let mut first = CaptionPointerState::default();
    let mut second = CaptionPointerState::default();
    first.update(CaptionPointerEvent::Move(Some(CaptionButton::Minimize)));
    first.update(CaptionPointerEvent::Press(CaptionButton::Minimize));
    second.update(CaptionPointerEvent::Move(Some(CaptionButton::Close)));
    assert_eq!(second.hovered, Some(CaptionButton::Close));
    assert_eq!(second.pressed, None);
    assert_eq!(first.update(CaptionPointerEvent::Cancel), None);
    assert_eq!(first.hovered, None);
    assert_eq!(first.pressed, None);
    assert_eq!(
        first.update(CaptionPointerEvent::Release(Some(CaptionButton::Minimize))),
        None
    );
    assert_eq!(second.hovered, Some(CaptionButton::Close));
}
