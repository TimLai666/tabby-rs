use std::{collections::BTreeMap, fs, path::Path};

use crate::{
    error::AppError,
    identity::AppPaths,
    launch::{LaunchContext, LaunchRequest},
    state::AppState,
    storage::{paths::StoragePaths, state_file::TabbyRsState},
    window_geometry::persistence::{LogicalBounds, SavedWindowState},
};

fn state_at(root: &Path, initial: TabbyRsState, hidden: bool) -> AppState {
    let app = tauri::test::mock_app();
    let paths =
        AppPaths::detect(app.handle(), root.join("tabby-rs"), Some(root.to_owned())).unwrap();
    let mut request = LaunchRequest::default();
    request.argv.hidden = hidden;
    AppState::new(
        paths,
        LaunchContext {
            request,
            cwd: root.display().to_string(),
            second_instance: false,
            parse_error: None,
        },
        initial,
    )
}

fn state_snapshot(state: &AppState) -> serde_json::Value {
    serde_json::to_value(state.persisted_state()).unwrap()
}

#[test]
fn startup_window_config_missing_and_blank_configs_are_read_without_writes() {
    let missing_root = tempfile::tempdir().unwrap();
    let missing_state = state_at(missing_root.path(), TabbyRsState::default(), false);
    let missing_paths = StoragePaths::from_app_paths(missing_state.paths());
    let before_state = state_snapshot(&missing_state);

    assert_eq!(
        super::startup_window_config(&missing_state).unwrap(),
        BTreeMap::new()
    );
    assert!(!missing_paths.config_file().exists());
    assert!(!missing_paths.state_file().exists());
    assert_eq!(state_snapshot(&missing_state), before_state);

    let blank_root = tempfile::tempdir().unwrap();
    let blank_state = state_at(blank_root.path(), TabbyRsState::default(), false);
    let blank_paths = StoragePaths::from_app_paths(blank_state.paths());
    fs::create_dir_all(blank_paths.data_dir()).unwrap();
    let blank = b" \n\t\n";
    fs::write(blank_paths.config_file(), blank).unwrap();
    let before_state = state_snapshot(&blank_state);

    assert_eq!(
        super::startup_window_config(&blank_state).unwrap(),
        BTreeMap::new()
    );
    assert_eq!(fs::read(blank_paths.config_file()).unwrap(), blank);
    assert!(!blank_paths.state_file().exists());
    assert_eq!(state_snapshot(&blank_state), before_state);
}

#[test]
fn startup_window_config_parses_appearance_and_preserves_unknown_values() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default(), false);
    let paths = StoragePaths::from_app_paths(state.paths());
    fs::create_dir_all(paths.data_dir()).unwrap();
    let yaml = b"version: 1\nappearance:\n  frame: full\n  colorSchemeMode: dark\npluginSpecific:\n  unknownField: [one, 2, true]\n";
    fs::write(paths.config_file(), yaml).unwrap();
    let before_state = state_snapshot(&state);

    let config = super::startup_window_config(&state).unwrap();
    assert_eq!(config["appearance"]["frame"], "full");
    assert_eq!(config["appearance"]["colorSchemeMode"], "dark");
    assert_eq!(
        config["pluginSpecific"]["unknownField"],
        serde_json::json!(["one", 2, true])
    );
    assert_eq!(fs::read(paths.config_file()).unwrap(), yaml);
    assert!(!paths.state_file().exists());
    assert_eq!(state_snapshot(&state), before_state);
}

#[test]
fn startup_window_config_rejects_invalid_yaml_without_changing_config_or_state() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default(), false);
    let paths = StoragePaths::from_app_paths(state.paths());
    fs::create_dir_all(paths.data_dir()).unwrap();
    let invalid = b"appearance: [unterminated\n";
    fs::write(paths.config_file(), invalid).unwrap();
    let before_state = state_snapshot(&state);

    assert!(matches!(
        super::startup_window_config(&state),
        Err(AppError::InvalidData(_))
    ));
    assert_eq!(fs::read(paths.config_file()).unwrap(), invalid);
    assert!(!paths.state_file().exists());
    assert_eq!(state_snapshot(&state), before_state);
}

#[test]
fn startup_window_config_returns_original_error_for_directory_and_leaves_it_intact() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default(), false);
    let paths = StoragePaths::from_app_paths(state.paths());
    fs::create_dir_all(paths.data_dir()).unwrap();
    fs::create_dir(paths.config_file()).unwrap();
    fs::write(paths.config_file().join("sentinel"), b"keep").unwrap();
    let before_state = state_snapshot(&state);

    assert!(matches!(
        super::startup_window_config(&state),
        Err(AppError::InvalidData(_))
    ));
    assert!(paths.config_file().is_dir());
    assert_eq!(
        fs::read(paths.config_file().join("sentinel")).unwrap(),
        b"keep"
    );
    assert!(!paths.state_file().exists());
    assert_eq!(state_snapshot(&state), before_state);
}

#[test]
fn startup_window_hidden_tracks_only_the_initial_main_window_request() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default(), true);

    assert!(state.startup_window_hidden("main"));
    assert!(!state.startup_window_hidden("window-1"));
    assert!(state.launches().take("main").unwrap().request.argv.hidden);
    assert!(state.startup_window_hidden("main"));

    let ordinary_root = tempfile::tempdir().unwrap();
    let ordinary = state_at(ordinary_root.path(), TabbyRsState::default(), false);
    assert!(!ordinary.startup_window_hidden("main"));
    assert!(!ordinary.startup_window_hidden("window-2"));
}

fn saved(maximized: bool) -> SavedWindowState {
    SavedWindowState {
        bounds: Some(LogicalBounds {
            x: -120,
            y: 24,
            width: 920,
            height: 680,
        }),
        maximized,
    }
}

#[test]
fn startup_window_maximized_keeps_each_labels_initial_persisted_seed() {
    let root = tempfile::tempdir().unwrap();
    let first_seed = saved(true);
    let mut initial = TabbyRsState::default();
    initial.window_state = Some(first_seed);
    let state = state_at(root.path(), initial, false);

    assert_eq!(state.startup_window_maximized("main"), None);
    assert_eq!(state.begin_window_state("main").unwrap(), first_seed);
    assert_eq!(state.startup_window_maximized("main"), Some(true));
    state.ready_window_state("main");
    state.record_window_state("main", saved(false));
    assert_eq!(state.startup_window_maximized("main"), Some(true));
    assert!(matches!(
        state.begin_window_state("main"),
        Err(AppError::InvalidArgument(_))
    ));
    assert_eq!(state.startup_window_maximized("main"), Some(true));

    state
        .update_persisted_state(|persisted| persisted.window_state = Some(saved(false)))
        .unwrap();
    assert_eq!(state.startup_window_maximized("main"), Some(true));
    assert_eq!(state.begin_window_state("window-2").unwrap(), saved(false));
    assert_eq!(state.startup_window_maximized("window-2"), Some(false));

    state.remove_window_state("main");
    assert_eq!(state.startup_window_maximized("main"), None);
    state.remove_window_state("window-2");
    assert_eq!(state.startup_window_maximized("window-2"), None);
}

#[test]
fn startup_window_maximized_captures_false_seed() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    initial.window_state = Some(saved(false));
    let state = state_at(root.path(), initial, false);

    assert_eq!(state.begin_window_state("main").unwrap(), saved(false));
    assert_eq!(state.startup_window_maximized("main"), Some(false));
}
