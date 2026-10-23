use super::{LogicalBounds, SavedWindowState, WindowStates};

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
fn constructor_sample_arms_capture_without_becoming_close_bounds() {
    let constructor = bounds(120, 80, 900, 650);
    let mut states = WindowStates::default();

    assert!(states.begin("fresh", saved(None, false)));
    states.ready_with_sample("fresh", constructor);

    assert_eq!(states.for_close("fresh", false), Some(saved(None, false)));
    assert_eq!(states.sampling_bounds("fresh"), Some(constructor));
}

#[test]
fn restored_bounds_survive_constructor_sample_until_a_real_resize() {
    let saved_offscreen = bounds(-1800, 70, 1000, 700);
    let constructor = bounds(140, 90, 1000, 700);
    let resized = bounds(constructor.x, constructor.y, 1180, 760);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(saved_offscreen), false)));
    states.ready_with_sample("main", constructor);
    states.record("main", constructor, false);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(saved_offscreen), false))
    );
    assert_eq!(states.sampling_bounds("main"), Some(constructor));

    states.record("main", resized, false);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(resized), false))
    );
    assert_eq!(states.sampling_bounds("main"), Some(resized));
    assert_eq!(states.sampling_bounds("main").unwrap().x, constructor.x);
    assert_eq!(states.sampling_bounds("main").unwrap().y, constructor.y);
}

#[test]
fn pre_ready_records_and_maximized_samples_do_not_change_capture_state() {
    let seed = bounds(30, 40, 800, 600);
    let constructor = bounds(130, 140, 900, 650);
    let pre_ready = bounds(5, 6, 500, 400);
    let maximized = bounds(0, 0, 1920, 1080);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(seed), false)));
    states.record("main", pre_ready, false);
    states.ready_with_sample("main", constructor);
    states.record("main", maximized, true);

    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(seed), false))
    );
    assert_eq!(states.sampling_bounds("main"), Some(constructor));
}

#[test]
fn duplicate_ready_sample_preserves_the_armed_or_updated_state() {
    let seed = bounds(10, 20, 700, 500);
    let first_sample = bounds(100, 120, 900, 650);
    let updated = bounds(100, 120, 1100, 750);
    let duplicate_sample = bounds(900, 800, 400, 300);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(seed), false)));
    states.ready_with_sample("main", first_sample);
    states.ready_with_sample("main", duplicate_sample);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(seed), false))
    );
    assert_eq!(states.sampling_bounds("main"), Some(first_sample));

    states.record("main", updated, false);
    states.ready_with_sample("main", duplicate_sample);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(updated), false))
    );
    assert_eq!(states.sampling_bounds("main"), Some(updated));
}

#[test]
fn unknown_label_capture_and_removal_are_no_ops() {
    let sample = bounds(30, 40, 800, 600);
    let mut states = WindowStates::default();

    states.ready_with_sample("missing", sample);
    states.record("missing", sample, false);
    states.remove("missing");

    assert_eq!(states.for_close("missing", false), None);
    assert_eq!(states.sampling_bounds("missing"), None);
    assert_eq!(states.startup_maximized("missing"), None);
}

#[test]
fn removing_and_rebeginning_a_label_discards_its_old_sample() {
    let old_seed = bounds(1, 2, 600, 450);
    let old_sample = bounds(11, 12, 700, 500);
    let new_seed = bounds(21, 22, 800, 600);
    let new_sample = bounds(31, 32, 900, 650);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(old_seed), false)));
    states.ready_with_sample("main", old_sample);
    states.remove("main");

    assert!(states.begin("main", saved(Some(new_seed), true)));
    assert_eq!(
        states.for_close("main", true),
        Some(saved(Some(new_seed), true))
    );
    assert_eq!(states.sampling_bounds("main"), Some(new_seed));
    states.ready_with_sample("main", new_sample);
    assert_eq!(states.sampling_bounds("main"), Some(new_sample));
    assert_eq!(states.startup_maximized("main"), Some(true));
}

#[test]
fn capture_samples_and_recorded_bounds_are_isolated_by_label() {
    let first_seed = bounds(-100, 20, 700, 500);
    let second_seed = bounds(200, 30, 800, 600);
    let first_sample = bounds(10, 40, 900, 650);
    let second_sample = bounds(20, 50, 1000, 700);
    let first_resize = bounds(10, 40, 1100, 750);
    let mut states = WindowStates::default();

    assert!(states.begin("first", saved(Some(first_seed), false)));
    assert!(states.begin("second", saved(Some(second_seed), true)));
    states.ready_with_sample("first", first_sample);
    states.ready_with_sample("second", second_sample);
    states.record("first", first_resize, false);

    assert_eq!(
        states.for_close("first", false),
        Some(saved(Some(first_resize), false))
    );
    assert_eq!(states.sampling_bounds("first"), Some(first_resize));
    assert_eq!(
        states.for_close("second", true),
        Some(saved(Some(second_seed), true))
    );
    assert_eq!(states.sampling_bounds("second"), Some(second_sample));
}

#[test]
fn ordinary_ready_keeps_first_record_and_startup_maximized_semantics() {
    let seed = bounds(40, 50, 900, 700);
    let resized = bounds(60, 70, 1000, 750);
    let maximized_sample = bounds(0, 0, 1920, 1080);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(seed), true)));
    states.ready("main");
    states.record("main", seed, false);
    assert_eq!(
        states.for_close("main", true),
        Some(saved(Some(seed), true))
    );
    assert_eq!(states.sampling_bounds("main"), Some(seed));

    states.record("main", maximized_sample, true);
    assert_eq!(
        states.for_close("main", true),
        Some(saved(Some(seed), true))
    );
    states.record("main", resized, false);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(resized), false))
    );
    assert_eq!(states.startup_maximized("main"), Some(true));
}

#[test]
fn unmaximize_records_constructor_bounds_after_unchanged_startup_samples() {
    let constructor = bounds(140, 90, 1000, 700);
    let maximized_geometry = bounds(0, 0, 1920, 1080);

    for seed in [
        saved(None, false),
        saved(Some(bounds(-1800, 70, 1000, 700)), true),
    ] {
        let mut states = WindowStates::default();
        assert!(states.begin("main", seed));
        states.ready_with_sample("main", constructor);

        states.record("main", constructor, false);
        assert_eq!(states.for_close("main", seed.maximized), Some(seed));
        assert_eq!(states.sampling_bounds("main"), Some(constructor));

        states.record("main", maximized_geometry, true);
        assert_eq!(states.for_close("main", seed.maximized), Some(seed));
        assert_eq!(states.sampling_bounds("main"), Some(constructor));

        states.record("main", constructor, false);
        assert_eq!(
            states.for_close("main", seed.maximized),
            Some(saved(Some(constructor), seed.maximized))
        );
        assert_eq!(states.sampling_bounds("main"), Some(constructor));
        assert_eq!(states.startup_maximized("main"), Some(seed.maximized));
    }
}

#[test]
fn x11_origin_arming_retains_seed_until_first_normal_event_then_records_it() {
    let sample = bounds(120, 80, 800, 600);
    for seed in [
        saved(None, false),
        saved(Some(bounds(5000, 4000, 800, 600)), true),
    ] {
        let mut states = WindowStates::default();
        states.begin("main", seed);
        states.ready_with_origin("main", sample);
        assert_eq!(states.for_close("main", seed.maximized), Some(seed));
        assert_eq!(states.sampling_bounds("main"), Some(sample));
        states.record("main", sample, false);
        assert_eq!(
            states.for_close("main", seed.maximized),
            Some(saved(Some(sample), seed.maximized))
        );
        states.ready_with_origin("main", bounds(0, 0, 400, 300));
        assert_eq!(states.sampling_bounds("main"), Some(sample));
    }
}

#[test]
fn x11_origin_arming_ignores_unknown_removed_and_maximized_samples() {
    let sample = bounds(120, 80, 800, 600);
    let mut states = WindowStates::default();
    states.ready_with_origin("missing", sample);
    assert_eq!(states.sampling_bounds("missing"), None);
    states.begin("main", saved(None, true));
    states.ready_with_origin("main", sample);
    states.record("main", bounds(0, 0, 1920, 1080), true);
    assert_eq!(states.for_close("main", true), Some(saved(None, true)));
    states.remove("main");
    states.ready_with_origin("main", sample);
    assert_eq!(states.sampling_bounds("main"), None);
}
