use crate::{
    error::AppError,
    identity::AppPaths,
    launch::{LaunchContext, LaunchRequest},
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

fn state_at(root: &std::path::Path, initial: TabbyRsState) -> crate::state::AppState {
    let app = tauri::test::mock_app();
    let paths =
        AppPaths::detect(app.handle(), root.join("tabby-rs"), Some(root.to_owned())).unwrap();
    crate::state::AppState::new(
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
fn begin_seeds_from_persisted_window_state() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(-1200, 24, 920, 680)), true);
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    assert!(!paths.state_file().exists());

    let returned = state.begin_window_state("main").unwrap();
    assert_eq!(returned, seed);
    assert_eq!(state.persisted_state().window_state, Some(seed));
    assert!(!paths.state_file().exists());
}

#[test]
fn begin_defaults_to_empty_saved_state_when_no_persisted_window() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default());
    let paths = StoragePaths::from_app_paths(state.paths());

    let returned = state.begin_window_state("main").unwrap();
    assert_eq!(returned, SavedWindowState::default());
    assert_eq!(state.persisted_state().window_state, None);
    assert!(!paths.state_file().exists());
}

#[test]
fn record_before_ready_does_not_overwrite_seed() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(10, 20, 900, 700)), false);
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial);
    let moved = bounds(100, 120, 800, 600);

    state.begin_window_state("main").unwrap();
    state.record_window_state("main", saved(Some(moved), false));
    state.save_window_state("main", false).unwrap();

    assert_eq!(state.persisted_state().window_state, Some(seed));
}

#[test]
fn ready_then_normal_sample_updates_maximized_and_none_do_not() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(10, 20, 900, 700)), false);
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial);
    let normal = bounds(-40, 18, 1100, 740);
    let maximized_sample = bounds(0, 0, 1920, 1080);

    state.begin_window_state("main").unwrap();
    state.ready_window_state("main");
    state.record_window_state("main", saved(Some(normal), false));
    state.record_window_state("main", saved(Some(maximized_sample), true));
    state.record_window_state("main", saved(None, false));
    let paths = StoragePaths::from_app_paths(state.paths());
    assert!(!paths.state_file().exists());
    state.save_window_state("main", false).unwrap();

    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(normal), false))
    );
    assert_eq!(
        load_state(paths.state_file()).unwrap().window_state,
        Some(saved(Some(normal), false))
    );
}

#[test]
fn only_save_writes_state_file_and_close_max_controls_flag() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    std::fs::write(paths.state_file(), b"sentinel bytes").unwrap();
    let before_bytes = std::fs::read(paths.state_file()).unwrap();

    state.begin_window_state("main").unwrap();
    state.ready_window_state("main");
    state.record_window_state("main", saved(Some(bounds(5, 6, 700, 500)), false));
    assert_eq!(std::fs::read(paths.state_file()).unwrap(), before_bytes);

    state.save_window_state("main", true).unwrap();
    let persisted = state.persisted_state().window_state.unwrap();
    assert_eq!(persisted.bounds, Some(bounds(5, 6, 700, 500)));
    assert!(persisted.maximized);
    assert!(paths.state_file().exists());

    // untouched window does not invent bounds
    let root2 = tempfile::tempdir().unwrap();
    let state2 = state_at(root2.path(), TabbyRsState::default());
    state2.begin_window_state("fresh").unwrap();
    state2.ready_window_state("fresh");
    state2.save_window_state("fresh", false).unwrap();
    assert_eq!(
        state2.persisted_state().window_state,
        Some(saved(None, false))
    );
}

#[test]
fn two_windows_are_independent_and_last_successful_save_wins() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(1, 2, 700, 500)), false);
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial);
    let first_move = bounds(-120, 15, 900, 650);
    let second_move = bounds(500, -70, 1000, 750);

    state.begin_window_state("first").unwrap();
    state.begin_window_state("second").unwrap();
    state.ready_window_state("first");
    state.ready_window_state("second");
    state.record_window_state("first", saved(Some(first_move), false));
    state.record_window_state("second", saved(Some(second_move), false));

    state.save_window_state("second", false).unwrap();
    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(second_move), false))
    );

    state.save_window_state("first", true).unwrap();
    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(first_move), true))
    );
    assert_eq!(
        load_state(StoragePaths::from_app_paths(state.paths()).state_file())
            .unwrap()
            .window_state,
        Some(saved(Some(first_move), true))
    );
}

#[test]
fn duplicate_begin_returns_invalid_argument_and_preserves_ready_and_cache() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(8, 9, 640, 480)), false);
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial);
    let duplicate_seed = saved(Some(bounds(80, 90, 1280, 720)), true);
    let new_move = bounds(-5, 6, 1024, 768);

    state.begin_window_state("main").unwrap();
    state.ready_window_state("main");
    state.record_window_state("main", saved(Some(bounds(20, 21, 650, 490)), false));
    state
        .update_persisted_state(|next| next.window_state = Some(duplicate_seed))
        .unwrap();

    let duplicate = state.begin_window_state("main");
    assert!(matches!(duplicate, Err(AppError::InvalidArgument(_))));
    state.save_window_state("main", false).unwrap();
    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(bounds(20, 21, 650, 490)), false))
    );

    state.record_window_state("main", saved(Some(new_move), false));
    state.save_window_state("main", false).unwrap();
    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(new_move), false))
    );
}

#[test]
fn unknown_or_removed_label_save_does_not_write_or_clear_state() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(100, 101, 640, 480)), false);
    initial.window_state = Some(seed);
    let state = state_at(root.path(), initial);
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &state.persisted_state()).unwrap();
    let original_bytes = std::fs::read(paths.state_file()).unwrap();

    state.begin_window_state("keep").unwrap();
    state.ready_window_state("keep");
    state.record_window_state("keep", saved(Some(bounds(1, 2, 3, 4)), false));

    let before = state.persisted_state().window_state;
    assert!(state.save_window_state("missing", false).is_ok());
    assert_eq!(state.persisted_state().window_state, before);
    assert_eq!(std::fs::read(paths.state_file()).unwrap(), original_bytes);
    state.ready_window_state("missing");
    state.record_window_state("missing", saved(Some(bounds(9, 9, 9, 9)), false));
    state.remove_window_state("missing");

    state.begin_window_state("temp").unwrap();
    state.remove_window_state("temp");
    state.ready_window_state("temp");
    state.record_window_state("temp", saved(Some(bounds(9, 9, 9, 9)), false));
    assert!(state.save_window_state("temp", false).is_ok());
    assert_eq!(state.persisted_state().window_state, before);
    assert_eq!(std::fs::read(paths.state_file()).unwrap(), original_bytes);

    // valid label remains usable
    state.save_window_state("keep", true).unwrap();
    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(bounds(1, 2, 3, 4)), true))
    );

    let absent = tempfile::tempdir().unwrap();
    let empty = state_at(absent.path(), TabbyRsState::default());
    empty.save_window_state("missing", true).unwrap();
    assert!(empty.persisted_state().window_state.is_none());
    assert!(!StoragePaths::from_app_paths(empty.paths())
        .state_file()
        .exists());
}

#[test]
fn rebuild_same_label_uses_current_durable_cache_after_remove() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let first_seed = saved(Some(bounds(11, 12, 600, 450)), false);
    initial.window_state = Some(first_seed);
    let state = state_at(root.path(), initial);

    state.begin_window_state("label").unwrap();
    state.remove_window_state("label");

    let new_seed = saved(Some(bounds(31, 32, 800, 600)), true);
    state
        .update_persisted_state(|next| next.window_state = Some(new_seed))
        .unwrap();

    let returned = state.begin_window_state("label").unwrap();
    assert_eq!(returned, new_seed);
}

#[test]
fn save_failure_retains_metadata_and_tracker_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let seed = saved(Some(bounds(-1200, 24, 920, 680)), true);
    initial.window_state = Some(seed);
    initial.pending_plugins = vec!["tabby-owned".into()];
    initial
        .extra
        .insert("meta".into(), serde_json::json!({"k": "v"}));
    let state = state_at(root.path(), initial.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &initial).unwrap();
    let good_bytes = std::fs::read(paths.state_file()).unwrap();

    let new_bounds = bounds(50, 60, 1000, 700);
    state.begin_window_state("main").unwrap();
    state.ready_window_state("main");
    state.record_window_state("main", saved(Some(new_bounds), false));

    let held_file = root.path().join("held-state.json");
    std::fs::rename(paths.state_file(), &held_file).unwrap();
    std::fs::create_dir(paths.state_file()).unwrap();

    let result = state.save_window_state("main", true);
    assert!(result.is_err());
    assert_eq!(state.persisted_state().window_state, initial.window_state);
    assert_eq!(
        state.persisted_state().pending_plugins,
        initial.pending_plugins
    );
    assert_eq!(state.persisted_state().extra, initial.extra);
    assert_eq!(std::fs::read(&held_file).unwrap(), good_bytes);
    assert!(paths.state_file().is_dir());

    std::fs::remove_dir(paths.state_file()).unwrap();
    std::fs::rename(&held_file, paths.state_file()).unwrap();

    state.save_window_state("main", true).unwrap();
    let expected = saved(Some(new_bounds), true);
    assert_eq!(state.persisted_state().window_state, Some(expected));

    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(reloaded.window_state, Some(expected));
    assert_eq!(reloaded.pending_plugins, initial.pending_plugins);
    assert_eq!(reloaded.extra, initial.extra);
}

#[test]
fn direct_update_persisted_state_seeds_new_label_not_old() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    let old_seed = saved(Some(bounds(7, 8, 640, 480)), false);
    initial.window_state = Some(old_seed);
    let state = state_at(root.path(), initial);
    let old_move = bounds(-9, 10, 900, 650);
    let new_seed = saved(Some(bounds(31, 32, 800, 600)), true);

    state.begin_window_state("old").unwrap();
    state.ready_window_state("old");
    state.record_window_state("old", saved(Some(old_move), false));

    state
        .update_persisted_state(|next| next.window_state = Some(new_seed))
        .unwrap();

    let returned = state.begin_window_state("new").unwrap();
    assert_eq!(returned, new_seed);

    state.save_window_state("old", false).unwrap();
    assert_eq!(
        state.persisted_state().window_state,
        Some(saved(Some(old_move), false))
    );
    state.save_window_state("new", true).unwrap();
    assert_eq!(state.persisted_state().window_state, Some(new_seed));
    assert_eq!(
        load_state(StoragePaths::from_app_paths(state.paths()).state_file())
            .unwrap()
            .window_state,
        Some(new_seed)
    );
}
