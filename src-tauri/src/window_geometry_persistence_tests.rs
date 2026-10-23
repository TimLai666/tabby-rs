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
fn begin_seeds_normal_bounds_and_waits_until_ready_to_capture() {
    let seed = bounds(24, 36, 960, 720);
    let moved_while_restoring = bounds(100, 120, 800, 600);
    let moved_after_ready = bounds(-40, 18, 1100, 740);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(seed), false)));
    states.record("main", moved_while_restoring, false);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(seed), false))
    );

    states.ready("main");
    states.record("main", moved_after_ready, false);
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(moved_after_ready), false))
    );
}

#[test]
fn maximized_events_are_ignored_and_close_snapshot_uses_live_state_without_consuming() {
    let normal = bounds(10, 20, 900, 700);
    let maximized_event_bounds = bounds(0, 0, 1920, 1080);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(normal), false)));
    states.ready("main");
    states.record("main", maximized_event_bounds, true);

    assert_eq!(
        states.for_close("main", true),
        Some(saved(Some(normal), true))
    );
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(normal), false))
    );
    assert_eq!(
        states.for_close("main", true),
        Some(saved(Some(normal), true))
    );
}

#[test]
fn untouched_new_window_has_no_invented_default_bounds() {
    let mut states = WindowStates::default();

    assert!(states.begin("fresh", saved(None, false)));
    states.ready("fresh");

    assert_eq!(states.for_close("fresh", false), Some(saved(None, false)));
}

#[test]
fn windows_keep_seed_and_recorded_bounds_independent() {
    let first_seed = bounds(1, 2, 700, 500);
    let second_seed = bounds(30, 40, 800, 600);
    let first_move = bounds(-120, 15, 900, 650);
    let second_move = bounds(500, -70, 1000, 750);
    let mut states = WindowStates::default();

    assert!(states.begin("first", saved(Some(first_seed), false)));
    assert!(states.begin("second", saved(Some(second_seed), true)));
    states.ready("first");
    states.ready("second");
    states.record("first", first_move, false);
    states.record("second", second_move, false);

    assert_eq!(
        states.for_close("first", false),
        Some(saved(Some(first_move), false))
    );
    assert_eq!(
        states.for_close("second", true),
        Some(saved(Some(second_move), true))
    );
}

#[test]
fn duplicate_begin_preserves_bounds_and_ready_state() {
    let original_seed = bounds(8, 9, 640, 480);
    let duplicate_seed = bounds(80, 90, 1280, 720);
    let post_ready_move = bounds(-5, 6, 1024, 768);
    let mut states = WindowStates::default();

    assert!(states.begin("main", saved(Some(original_seed), false)));
    states.ready("main");
    assert!(!states.begin("main", saved(Some(duplicate_seed), true)));
    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(original_seed), false))
    );
    states.record("main", post_ready_move, false);

    assert_eq!(
        states.for_close("main", false),
        Some(saved(Some(post_ready_move), false))
    );
}

#[test]
fn unknown_labels_are_no_ops_and_close_has_no_snapshot() {
    let mut states = WindowStates::default();
    let candidate = bounds(3, 4, 500, 400);

    states.ready("missing");
    states.record("missing", candidate, false);
    states.remove("missing");
    assert_eq!(states.for_close("missing", false), None);
}

#[test]
fn remove_forgets_only_its_window_and_recreated_label_uses_only_new_seed() {
    let first_seed = bounds(11, 12, 600, 450);
    let second_seed = bounds(21, 22, 700, 550);
    let second_move = bounds(-21, -22, 900, 650);
    let new_seed = bounds(31, 32, 800, 600);
    let mut states = WindowStates::default();

    assert!(states.begin("first", saved(Some(first_seed), false)));
    assert!(states.begin("second", saved(Some(second_seed), false)));
    states.ready("first");
    states.ready("second");
    states.remove("first");
    assert_eq!(states.for_close("first", false), None);
    states.record("second", second_move, false);
    assert_eq!(
        states.for_close("second", false),
        Some(saved(Some(second_move), false))
    );

    assert!(states.begin("first", saved(Some(new_seed), false)));
    let before_ready_move = bounds(45, -54, 1234, 987);
    states.record("first", before_ready_move, false);
    assert_eq!(
        states.for_close("first", false),
        Some(saved(Some(new_seed), false))
    );
    states.ready("first");
    assert_eq!(
        states.for_close("first", false),
        Some(saved(Some(new_seed), false))
    );
}

#[test]
fn logical_bounds_retain_negative_zero_and_unusual_dimensions() {
    let edge = bounds(-1920, 0, 0, 0);
    let mut states = WindowStates::default();

    assert!(states.begin("edge", saved(Some(edge), false)));
    states.ready("edge");
    states.record("edge", edge, false);

    assert_eq!(
        states.for_close("edge", false),
        Some(saved(Some(edge), false))
    );
}

#[test]
fn saved_state_json_round_trip_preserves_normal_bounds_and_maximized_flag() {
    let original = saved(Some(bounds(-1440, 32, 0, 810)), true);
    let encoded = serde_json::to_string(&original).expect("saved state serializes");
    let decoded: SavedWindowState =
        serde_json::from_str(&encoded).expect("saved state deserializes");
    let mut states = WindowStates::default();

    assert_eq!(decoded, original);
    assert!(states.begin("restored", decoded));
    states.ready("restored");
    assert_eq!(
        states.for_close("restored", decoded.maximized),
        Some(original)
    );
}

#[test]
fn close_snapshot_reopens_as_seed_then_tracks_next_normal_resize() {
    let original_normal = bounds(50, 60, 1000, 700);
    let changed_normal = bounds(-30, 45, 1200, 800);
    let maximized_event = bounds(0, 0, 1920, 1080);
    let restored_resize = bounds(9, -12, 1150, 780);
    let mut states = WindowStates::default();

    assert!(states.begin("first", saved(Some(original_normal), false)));
    states.ready("first");
    states.record("first", changed_normal, false);
    states.record("first", maximized_event, true);
    let close_snapshot = states
        .for_close("first", true)
        .expect("registered window has a close snapshot");
    assert_eq!(close_snapshot, saved(Some(changed_normal), true));

    states.remove("first");
    assert!(states.begin("reopened", close_snapshot));
    states.ready("reopened");
    assert_eq!(
        states.for_close("reopened", true),
        Some(saved(Some(changed_normal), true))
    );
    states.record("reopened", restored_resize, false);
    assert_eq!(
        states.for_close("reopened", false),
        Some(saved(Some(restored_resize), false))
    );
}

#[test]
fn reading_first_close_and_removing_it_does_not_change_second_window() {
    let first_seed = bounds(100, 101, 640, 480);
    let second_seed = bounds(200, 201, 800, 600);
    let first_move = bounds(-100, 30, 900, 650);
    let second_move = bounds(300, -40, 1024, 768);
    let mut states = WindowStates::default();

    assert!(states.begin("first", saved(Some(first_seed), false)));
    assert!(states.begin("second", saved(Some(second_seed), false)));
    states.ready("first");
    states.ready("second");
    states.record("first", first_move, false);
    states.record("second", second_move, false);

    let first_snapshot = states.for_close("first", true);
    assert_eq!(first_snapshot, Some(saved(Some(first_move), true)));
    assert_eq!(
        states.for_close("second", false),
        Some(saved(Some(second_move), false))
    );

    states.remove("first");
    assert_eq!(states.for_close("first", false), None);
    let next_second_move = bounds(-300, 41, 1100, 780);
    states.record("second", next_second_move, false);
    assert_eq!(
        states.for_close("second", true),
        Some(saved(Some(next_second_move), true))
    );
}

#[test]
fn startup_maximized_retains_the_seed_across_samples_and_duplicate_registration() {
    for initial in [false, true] {
        let mut states = WindowStates::default();
        assert_eq!(states.startup_maximized("main"), None);
        assert!(states.begin("main", saved(Some(bounds(10, 20, 800, 600)), initial)));
        assert_eq!(states.startup_maximized("main"), Some(initial));
        states.ready("main");
        states.record("main", bounds(30, 40, 900, 700), false);
        assert!(!states.begin("main", saved(None, !initial)));
        assert_eq!(states.startup_maximized("main"), Some(initial));
        assert_eq!(
            states.for_close("main", !initial).unwrap().maximized,
            !initial
        );
        assert_eq!(states.startup_maximized("main"), Some(initial));
        states.remove("main");
        assert_eq!(states.startup_maximized("main"), None);
        assert!(states.begin("main", saved(None, !initial)));
        assert_eq!(states.startup_maximized("main"), Some(!initial));
    }
}
