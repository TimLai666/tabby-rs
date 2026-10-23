use super::{load_state, save_state, TabbyRsState};
use crate::window_geometry::persistence::{LogicalBounds, SavedWindowState};

fn saved(x: i32, maximized: bool) -> SavedWindowState {
    SavedWindowState {
        bounds: Some(LogicalBounds {
            x,
            y: -32,
            width: 971,
            height: 723,
        }),
        maximized,
    }
}

#[test]
fn loads_saved_normal_bounds_and_maximized_state_alongside_other_metadata() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tabby-rs.json");
    std::fs::write(&path, r#"{
        "schemaVersion": 1,
        "windowState": { "bounds": { "x": -1920, "y": -32, "width": 971, "height": 723 }, "maximized": true },
        "pendingPlugins": ["tabby-demo"],
        "futureFeature": { "enabled": true }
    }"#).unwrap();
    let mut state = load_state(&path).unwrap();
    assert_eq!(state.window_state, Some(saved(-1920, true)));
    state.diagnostics.local_logging_enabled = false;
    save_state(&path, &state).unwrap();
    let loaded = load_state(&path).unwrap();
    assert_eq!(loaded.window_state, Some(saved(-1920, true)));
    assert_eq!(loaded.pending_plugins, vec!["tabby-demo"]);
    assert_eq!(loaded.extra["futureFeature"]["enabled"], true);
    assert!(!loaded.diagnostics.local_logging_enabled);
}

#[test]
fn writes_independent_roots_and_reloads_the_latest_saved_window() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first_path = first.path().join("tabby-rs.json");
    let second_path = second.path().join("tabby-rs.json");
    let mut first_state = TabbyRsState::default();
    let mut second_state = TabbyRsState::default();
    first_state.window_state = Some(saved(12, false));
    second_state.window_state = Some(saved(-1440, true));
    save_state(&first_path, &first_state).unwrap();
    save_state(&second_path, &second_state).unwrap();
    assert_eq!(
        load_state(&first_path).unwrap().window_state,
        first_state.window_state
    );
    assert_eq!(
        load_state(&second_path).unwrap().window_state,
        second_state.window_state
    );
    first_state.window_state = Some(saved(400, true));
    save_state(&first_path, &first_state).unwrap();
    assert_eq!(
        load_state(&first_path).unwrap().window_state,
        Some(saved(400, true))
    );
    assert_eq!(
        load_state(&second_path).unwrap().window_state,
        Some(saved(-1440, true))
    );
}

#[test]
fn old_and_missing_state_have_no_saved_window() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tabby-rs.json");
    assert_eq!(load_state(&path).unwrap().window_state, None);
    std::fs::write(
        &path,
        r#"{"schemaVersion":1,"pendingPlugins":["tabby-demo"]}"#,
    )
    .unwrap();
    let state = load_state(&path).unwrap();
    assert_eq!(state.window_state, None);
    save_state(&path, &state).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(value.get("windowState").is_none());
    assert_eq!(value["pendingPlugins"], serde_json::json!(["tabby-demo"]));
}
