use super::AppState;
use crate::{
    error::AppError,
    identity::AppPaths,
    launch::{LaunchContext, LaunchRequest},
    storage::{
        backup::{create_backup, list_backups, BackupRequest},
        paths::StoragePaths,
        state_file::{load_state, save_state, TabbyRsState, STATE_SCHEMA_VERSION},
    },
    window_geometry::persistence::{LogicalBounds, SavedWindowState},
};

fn initial_window() -> SavedWindowState {
    SavedWindowState {
        bounds: Some(LogicalBounds {
            x: -1200,
            y: 24,
            width: 920,
            height: 680,
        }),
        maximized: true,
    }
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
fn failed_state_write_keeps_last_saved_window_and_metadata_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    initial.window_state = Some(initial_window());
    initial.pending_plugins = vec!["tabby-owned".into()];
    let state = state_at(root.path(), initial.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &initial).unwrap();
    let good_bytes = std::fs::read(paths.state_file()).unwrap();
    let held_file = root.path().join("held-state.json");
    std::fs::rename(paths.state_file(), &held_file).unwrap();
    std::fs::create_dir(paths.state_file()).unwrap();

    let result = state.update_persisted_state(|next| {
        next.window_state = Some(SavedWindowState {
            bounds: None,
            maximized: false,
        });
        next.pending_plugins.push("must-not-commit".into());
    });
    assert!(result.is_err());
    assert_eq!(state.persisted_state().window_state, initial.window_state);
    assert_eq!(
        state.persisted_state().pending_plugins,
        initial.pending_plugins
    );
    assert_eq!(std::fs::read(&held_file).unwrap(), good_bytes);
    assert!(paths.state_file().is_dir());

    std::fs::remove_dir(paths.state_file()).unwrap();
    std::fs::rename(&held_file, paths.state_file()).unwrap();
    state
        .update_persisted_state(|next| next.diagnostics.local_logging_enabled = false)
        .unwrap();
    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(reloaded.window_state, initial.window_state);
    assert_eq!(reloaded.pending_plugins, initial.pending_plugins);
    assert!(!reloaded.diagnostics.local_logging_enabled);
}

#[test]
fn successful_window_save_survives_new_host_state_and_keeps_unrelated_metadata() {
    let root = tempfile::tempdir().unwrap();
    let mut initial = TabbyRsState::default();
    initial.pending_plugins = vec!["tabby-retained".into()];
    let state = state_at(root.path(), initial);
    state
        .update_persisted_state(|next| next.window_state = Some(initial_window()))
        .unwrap();
    let paths = StoragePaths::from_app_paths(state.paths());
    let loaded = load_state(paths.state_file()).unwrap();
    let reopened = state_at(root.path(), loaded);
    assert_eq!(
        reopened.persisted_state().window_state,
        Some(initial_window())
    );
    assert_eq!(
        reopened.persisted_state().pending_plugins,
        vec!["tabby-retained"]
    );
}

#[test]
fn restore_backup_restores_old_state_and_cache_without_being_overwritten_by_later_update() {
    let root = tempfile::tempdir().unwrap();
    let mut old_state = TabbyRsState::default();
    old_state.window_state = Some(initial_window());
    old_state.pending_plugins = vec!["tabby-owned".into()];
    old_state
        .extra
        .insert("meta".into(), serde_json::json!({"k": "v"}));
    let state = state_at(root.path(), old_state.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &old_state).unwrap();
    std::fs::write(paths.config_file(), b"version: 1\nold: true\n").unwrap();

    let backup = create_backup(
        &paths,
        &BackupRequest {
            reason: "pre-restore".into(),
            source_version: None,
            channel: None,
        },
        "1.0.231-tabbyrs.1",
    )
    .unwrap();

    state
        .update_persisted_state(|next| {
            next.window_state = Some(SavedWindowState {
                bounds: None,
                maximized: false,
            });
            next.pending_plugins = vec!["newp".into()];
            next.extra.clear();
            next.extra.insert("other".into(), serde_json::json!(1));
        })
        .unwrap();
    std::fs::write(paths.config_file(), b"version: 9\n").unwrap();

    let err = state.restore_backup("does-not-exist").unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
    assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 9\n");
    assert_eq!(
        state.persisted_state().pending_plugins,
        vec!["newp".to_owned()]
    );

    let report = state.restore_backup(&backup.backup_id).unwrap();
    assert!(report.restored.contains(&"config.yaml".to_string()));
    assert!(report.restored.contains(&"tabby-rs.json".to_string()));
    assert_eq!(
        std::fs::read(paths.config_file()).unwrap(),
        b"version: 1\nold: true\n"
    );
    assert_eq!(state.persisted_state().window_state, old_state.window_state);
    assert_eq!(
        state.persisted_state().pending_plugins,
        old_state.pending_plugins
    );
    assert_eq!(state.persisted_state().extra, old_state.extra);

    state
        .update_persisted_state(|next| next.diagnostics.local_logging_enabled = false)
        .unwrap();
    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(reloaded.window_state, old_state.window_state);
    assert_eq!(reloaded.pending_plugins, old_state.pending_plugins);
    assert_eq!(reloaded.extra, old_state.extra);
    assert!(!reloaded.diagnostics.local_logging_enabled);
}

#[test]
fn restore_backup_when_state_missing_at_backup_restores_default_and_does_not_revive_old_fields() {
    let root = tempfile::tempdir().unwrap();
    let initial = TabbyRsState::default();
    let state = state_at(root.path(), initial);
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    assert!(!paths.state_file().exists());
    std::fs::write(paths.config_file(), b"version: 0\n").unwrap();

    let backup = create_backup(
        &paths,
        &BackupRequest {
            reason: "empty-snapshot".into(),
            source_version: None,
            channel: None,
        },
        "1.0.231-tabbyrs.1",
    )
    .unwrap();

    let mut new_state = TabbyRsState::default();
    new_state.window_state = Some(initial_window());
    new_state.pending_plugins = vec!["old-plugin".into()];
    new_state
        .extra
        .insert("stale".into(), serde_json::json!({"keep": false}));
    state
        .update_persisted_state(|next| *next = new_state)
        .unwrap();

    let report = state.restore_backup(&backup.backup_id).unwrap();
    assert!(report.removed.contains(&"tabby-rs.json".to_string()));
    assert!(!paths.state_file().exists());
    assert_eq!(
        serde_json::to_value(state.persisted_state()).unwrap(),
        serde_json::to_value(TabbyRsState::default()).unwrap()
    );

    state
        .update_persisted_state(|next| next.diagnostics.local_logging_enabled = false)
        .unwrap();
    assert_eq!(state.persisted_state().window_state, None);
    assert!(state.persisted_state().pending_plugins.is_empty());
    assert!(state.persisted_state().extra.is_empty());
    let persisted = load_state(paths.state_file()).unwrap();
    assert_eq!(persisted.window_state, None);
    assert!(persisted.pending_plugins.is_empty());
    assert!(persisted.extra.is_empty());
}

#[test]
fn restore_backup_rolls_back_on_invalid_json_backup_via_safety_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let mut active_state = TabbyRsState::default();
    active_state.window_state = Some(initial_window());
    active_state.pending_plugins = vec!["active".into()];
    active_state
        .extra
        .insert("active".into(), serde_json::json!(true));
    let state = state_at(root.path(), active_state.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &active_state).unwrap();
    std::fs::write(paths.config_file(), b"version: 1\nactive: true\n").unwrap();

    // create backup of invalid JSON state file
    std::fs::write(paths.state_file(), b"{not valid json").unwrap();
    std::fs::write(paths.config_file(), b"version: 9\nbad_snapshot: true\n").unwrap();
    let bad_backup = create_backup(
        &paths,
        &BackupRequest {
            reason: "bad-json".into(),
            source_version: None,
            channel: None,
        },
        "1.0.231-tabbyrs.1",
    )
    .unwrap();

    // restore active valid state
    save_state(paths.state_file(), &active_state).unwrap();
    std::fs::write(paths.config_file(), b"version: 1\nactive: true\n").unwrap();

    let err = state.restore_backup(&bad_backup.backup_id).unwrap_err();
    assert!(err.to_string().contains("restore failed"));

    assert_eq!(
        std::fs::read(paths.config_file()).unwrap(),
        b"version: 1\nactive: true\n"
    );
    let restored_state = load_state(paths.state_file()).unwrap();
    assert_eq!(restored_state.window_state, active_state.window_state);
    assert_eq!(restored_state.pending_plugins, active_state.pending_plugins);
    assert_eq!(restored_state.extra, active_state.extra);
    assert_eq!(
        state.persisted_state().window_state,
        active_state.window_state
    );
    assert_eq!(
        state.persisted_state().pending_plugins,
        active_state.pending_plugins
    );
    assert_eq!(state.persisted_state().extra, active_state.extra);

    state
        .update_persisted_state(|next| next.diagnostics.local_logging_enabled = false)
        .unwrap();
    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(reloaded.pending_plugins, vec!["active".to_owned()]);
    assert_eq!(reloaded.extra.get("active"), Some(&serde_json::json!(true)));
}

#[test]
fn restore_backup_rolls_back_on_future_schema_version_backup_via_safety_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let mut active_state = TabbyRsState::default();
    active_state.window_state = Some(initial_window());
    active_state.pending_plugins = vec!["active".into()];
    active_state
        .extra
        .insert("active".into(), serde_json::json!(true));
    let state = state_at(root.path(), active_state.clone());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();
    save_state(paths.state_file(), &active_state).unwrap();
    std::fs::write(paths.config_file(), b"version: 1\nactive: true\n").unwrap();

    // create backup of future schema version
    let mut future_state = active_state.clone();
    future_state.schema_version = STATE_SCHEMA_VERSION + 1;
    std::fs::write(
        paths.state_file(),
        serde_json::to_vec(&future_state).unwrap(),
    )
    .unwrap();
    std::fs::write(paths.config_file(), b"version: 9\nbad_snapshot: true\n").unwrap();
    let bad_backup = create_backup(
        &paths,
        &BackupRequest {
            reason: "future-schema".into(),
            source_version: None,
            channel: None,
        },
        "1.0.231-tabbyrs.1",
    )
    .unwrap();

    // restore active valid state
    save_state(paths.state_file(), &active_state).unwrap();
    std::fs::write(paths.config_file(), b"version: 1\nactive: true\n").unwrap();

    let err = state.restore_backup(&bad_backup.backup_id).unwrap_err();
    assert!(err.to_string().contains("restore failed"));

    assert_eq!(
        std::fs::read(paths.config_file()).unwrap(),
        b"version: 1\nactive: true\n"
    );
    let restored_state = load_state(paths.state_file()).unwrap();
    assert_eq!(restored_state.window_state, active_state.window_state);
    assert_eq!(restored_state.pending_plugins, active_state.pending_plugins);
    assert_eq!(restored_state.extra, active_state.extra);
    assert_eq!(
        state.persisted_state().window_state,
        active_state.window_state
    );
    assert_eq!(
        state.persisted_state().pending_plugins,
        active_state.pending_plugins
    );
    assert_eq!(state.persisted_state().extra, active_state.extra);

    state
        .update_persisted_state(|next| next.diagnostics.local_logging_enabled = false)
        .unwrap();
    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(reloaded.pending_plugins, vec!["active".to_owned()]);
    assert_eq!(reloaded.extra.get("active"), Some(&serde_json::json!(true)));
}

#[test]
fn restore_backup_at_retention_limit_restores_oldest_selected_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let state = state_at(root.path(), TabbyRsState::default());
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout().unwrap();

    let mut first_backup_id = String::new();
    let mut first_state = None;
    let mut first_config = Vec::new();
    for index in 0..20 {
        let reason = format!("ordered-{index:02}");
        let mut snapshot = TabbyRsState::default();
        snapshot.window_state = Some(SavedWindowState {
            bounds: Some(LogicalBounds {
                x: -1200 - index,
                y: 24 + index,
                width: 920 + index as u32,
                height: 680 + index as u32,
            }),
            maximized: index % 2 == 0,
        });
        snapshot.pending_plugins = vec![format!("plugin-{index:02}")];
        snapshot.extra.insert(
            "snapshot".into(),
            serde_json::json!({"index": index, "reason": reason.clone()}),
        );
        state
            .update_persisted_state(|current| *current = snapshot.clone())
            .unwrap();
        let config = format!("version: {index}\nsnapshot: {reason}\n").into_bytes();
        std::fs::write(paths.config_file(), &config).unwrap();

        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason,
                source_version: None,
                channel: None,
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        if index == 0 {
            first_backup_id = backup.backup_id;
            first_state = Some(snapshot);
            first_config = config;
        }
    }

    let first_state = first_state.expect("first snapshot was created");
    let backups_before_restore = list_backups(&paths).unwrap();
    assert_eq!(backups_before_restore.len(), 20);
    let first_manifest = backups_before_restore
        .iter()
        .find(|manifest| manifest.backup_id == first_backup_id)
        .expect("first snapshot remains before restore");
    assert_eq!(first_manifest.reason, "ordered-00");
    // Retention sorts backup directory names, whose timestamp prefix and reason
    // make this the oldest directory even when snapshots share a millisecond.
    let oldest_backup = backups_before_restore
        .iter()
        .min_by(|left, right| left.backup_id.cmp(&right.backup_id))
        .expect("there are retained snapshots");
    assert_eq!(oldest_backup.backup_id, first_backup_id);
    assert!(paths.backup_dir(&first_backup_id).unwrap().is_dir());

    let mut active_state = TabbyRsState::default();
    active_state.window_state = Some(initial_window());
    active_state.pending_plugins = vec!["active-after-snapshots".to_owned()];
    active_state
        .extra
        .insert("active-marker".into(), serde_json::json!(true));
    state
        .update_persisted_state(|current| *current = active_state)
        .unwrap();
    std::fs::write(paths.config_file(), b"version: 999\nsnapshot: active\n").unwrap();

    let report = state.restore_backup(&first_backup_id).unwrap();
    assert_eq!(report.backup_id, first_backup_id);
    assert_eq!(std::fs::read(paths.config_file()).unwrap(), first_config);

    let restored_cache = state.persisted_state();
    assert_eq!(restored_cache.window_state, first_state.window_state);
    assert_eq!(restored_cache.pending_plugins, first_state.pending_plugins);
    assert_eq!(restored_cache.extra, first_state.extra);
    let restored_disk = load_state(paths.state_file()).unwrap();
    assert_eq!(restored_disk.window_state, first_state.window_state);
    assert_eq!(restored_disk.pending_plugins, first_state.pending_plugins);
    assert_eq!(restored_disk.extra, first_state.extra);

    let backups_after_restore = list_backups(&paths).unwrap();
    assert_eq!(backups_after_restore.len(), 20);
    assert!(backups_after_restore
        .iter()
        .any(|manifest| manifest.reason == "before-restore"));
    assert!(!paths.backup_dir(&first_backup_id).unwrap().exists());
    assert!(backups_after_restore
        .iter()
        .all(|manifest| manifest.backup_id != first_backup_id));

    state
        .update_persisted_state(|next| next.diagnostics.local_logging_enabled = false)
        .unwrap();
    let after_diagnostic_update = state.persisted_state();
    assert_eq!(
        after_diagnostic_update.window_state,
        first_state.window_state
    );
    assert_eq!(
        after_diagnostic_update.pending_plugins,
        first_state.pending_plugins
    );
    assert_eq!(after_diagnostic_update.extra, first_state.extra);
    assert!(!after_diagnostic_update.diagnostics.local_logging_enabled);
    let reloaded = load_state(paths.state_file()).unwrap();
    assert_eq!(reloaded.window_state, first_state.window_state);
    assert_eq!(reloaded.pending_plugins, first_state.pending_plugins);
    assert_eq!(reloaded.extra, first_state.extra);
    assert!(!reloaded.diagnostics.local_logging_enabled);
}
