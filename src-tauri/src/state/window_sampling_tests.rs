use crate::{
    identity::AppPaths,
    launch::{LaunchContext, LaunchRequest},
    state::AppState,
    storage::{
        paths::StoragePaths,
        state_file::{load_state, save_state, TabbyRsState},
    },
    window_geometry::persistence::{LogicalBounds, SavedWindowState},
};

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

fn state_at(root: &std::path::Path, initial: TabbyRsState) -> AppState {
    let app = tauri::test::mock_app();
    let paths =
        AppPaths::detect(app.handle(), root.join("tabby-rs"), Some(root.to_owned())).unwrap();
    AppState::new(
        paths,
        LaunchContext {
            request: LaunchRequest::default(),
            cwd: root.display().to_string(),
            second_instance: false,
            parse_error: None,
        },
        initial,
    )
}

#[test]
fn constructor_sampling_stays_transient_until_approved_close_saves_resize() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default());
    let paths = StoragePaths::from_app_paths(state.paths());
    let constructor = bounds(120, 80, 900, 650);
    let resized = bounds(120, 80, 1120, 760);

    state.begin_window_state("fresh").unwrap();
    state.ready_window_state_with_sample("fresh", constructor);
    assert_eq!(state.window_sampling_bounds("fresh"), Some(constructor));
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());

    // Repeated native events at the construction size must not promote it to saved state.
    state.record_window_state("fresh", saved(Some(constructor), false));
    state.record_window_state("fresh", saved(Some(constructor), false));
    assert_eq!(state.window_sampling_bounds("fresh"), Some(constructor));
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());

    state.save_window_state("fresh", false).unwrap();
    let before_resize = saved(None, false);
    assert_eq!(state.persisted_state().window_state, Some(before_resize));
    assert_eq!(
        load_state(paths.state_file()).unwrap().window_state,
        Some(before_resize)
    );
    let before_resize_bytes = std::fs::read(paths.state_file()).unwrap();

    state.record_window_state("fresh", saved(Some(resized), false));
    assert_eq!(state.window_sampling_bounds("fresh"), Some(resized));
    assert_eq!(state.persisted_state().window_state, Some(before_resize));
    assert_eq!(
        std::fs::read(paths.state_file()).unwrap(),
        before_resize_bytes
    );

    state.save_window_state("fresh", false).unwrap();
    let expected = saved(Some(resized), false);
    assert_eq!(state.persisted_state().window_state, Some(expected));
    assert_eq!(
        load_state(paths.state_file()).unwrap().window_state,
        Some(expected)
    );
}

#[test]
fn restored_offscreen_seed_survives_constructor_sample_until_close_and_keeps_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let offscreen_seed = saved(Some(bounds(-1800, 70, 1000, 700)), true);
    initial.window_state = Some(offscreen_seed);
    initial.extra.insert(
        "futureFeature".into(),
        serde_json::json!({"enabled": true, "revision": 7}),
    );
    let state = state_at(root.path(), initial.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &initial).unwrap();
    let original_bytes = std::fs::read(paths.state_file()).unwrap();
    let constructor = bounds(140, 90, 1000, 700);
    let resized = bounds(140, 90, 1180, 760);

    state.begin_window_state("main").unwrap();
    state.ready_window_state_with_sample("main", constructor);
    assert_eq!(state.window_sampling_bounds("main"), Some(constructor));
    state.record_window_state("main", saved(Some(constructor), false));
    state.record_window_state("main", saved(Some(constructor), false));

    assert_eq!(state.persisted_state().window_state, Some(offscreen_seed));
    assert_eq!(std::fs::read(paths.state_file()).unwrap(), original_bytes);
    assert_eq!(
        load_state(paths.state_file()).unwrap().window_state,
        Some(offscreen_seed)
    );

    state.record_window_state("main", saved(Some(resized), false));
    assert_eq!(state.window_sampling_bounds("main"), Some(resized));
    assert_eq!(state.persisted_state().window_state, Some(offscreen_seed));
    assert_eq!(std::fs::read(paths.state_file()).unwrap(), original_bytes);

    state.save_window_state("main", true).unwrap();
    let expected = saved(Some(resized), true);
    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(state.persisted_state().window_state, Some(expected));
    assert_eq!(reloaded.window_state, Some(expected));
    assert_eq!(
        reloaded.extra["futureFeature"],
        serde_json::json!({"enabled": true, "revision": 7})
    );
}

#[test]
fn unknown_labels_are_no_ops_and_duplicate_arming_does_not_reset_capture() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default());
    let paths = StoragePaths::from_app_paths(state.paths());
    let sample = bounds(30, 40, 800, 600);

    state.ready_window_state_with_sample("missing", sample);
    assert_eq!(state.window_sampling_bounds("missing"), None);
    state.record_window_state("missing", saved(Some(sample), false));
    state.remove_window_state("missing");
    assert_eq!(state.window_sampling_bounds("missing"), None);
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());
    assert_eq!(
        state.begin_window_state("missing").unwrap(),
        SavedWindowState::default()
    );
    state.remove_window_state("missing");
    assert!(!paths.state_file().exists());

    let initial_sample = bounds(100, 120, 900, 650);
    let duplicate_sample = bounds(500, 520, 500, 400);
    let resized = bounds(100, 120, 1100, 750);
    state.begin_window_state("active").unwrap();
    state.ready_window_state_with_sample("active", initial_sample);
    state.ready_window_state_with_sample("active", duplicate_sample);
    assert_eq!(state.window_sampling_bounds("active"), Some(initial_sample));

    state.record_window_state("active", saved(Some(resized), false));
    state.ready_window_state_with_sample("active", duplicate_sample);
    assert_eq!(state.window_sampling_bounds("active"), Some(resized));
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());
}

#[test]
fn two_labels_and_remove_rebegin_keep_sampling_state_isolated() {
    let root = tempfile::tempdir().unwrap();
    let seed = saved(Some(bounds(-200, 25, 800, 600)), false);
    let mut initial = TabbyRsState::default();
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial);
    let first_sample = bounds(10, 40, 900, 650);
    let second_sample = bounds(20, 50, 1000, 700);
    let first_resize = bounds(10, 40, 1100, 750);
    let replacement_sample = bounds(310, 320, 850, 620);

    state.begin_window_state("first").unwrap();
    state.begin_window_state("second").unwrap();
    state.ready_window_state_with_sample("first", first_sample);
    state.ready_window_state_with_sample("second", second_sample);
    state.record_window_state("first", saved(Some(first_resize), false));

    assert_eq!(state.window_sampling_bounds("first"), Some(first_resize));
    assert_eq!(state.window_sampling_bounds("second"), Some(second_sample));
    state.remove_window_state("first");
    assert_eq!(state.window_sampling_bounds("first"), None);
    assert_eq!(state.window_sampling_bounds("second"), Some(second_sample));

    assert_eq!(state.begin_window_state("first").unwrap(), seed);
    assert_eq!(
        state.window_sampling_bounds("first"),
        Some(seed.bounds.unwrap())
    );
    state.ready_window_state_with_sample("first", replacement_sample);
    assert_eq!(
        state.window_sampling_bounds("first"),
        Some(replacement_sample)
    );
    assert_eq!(state.window_sampling_bounds("second"), Some(second_sample));
    assert_eq!(state.persisted_state().window_state, Some(seed));
}

#[test]
fn x11_first_display_sample_is_saved_only_after_approved_close() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default());
    let paths = StoragePaths::from_app_paths(state.paths());
    let sample = bounds(120, 80, 800, 600);
    state.begin_window_state("main").unwrap();
    state.ready_window_state_with_origin("main", sample);
    assert_eq!(state.window_sampling_bounds("main"), Some(sample));
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());
    state.record_window_state("main", saved(Some(sample), false));
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());
    state.save_window_state("main", false).unwrap();
    assert_eq!(
        load_state(paths.state_file()).unwrap().window_state,
        Some(saved(Some(sample), false))
    );
}
