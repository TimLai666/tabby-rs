use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};
use tauri::Config;

use super::{data_directory, prepare};

fn config_with_windows(windows: Value) -> Config {
    serde_json::from_value(json!({
        "productName": "Storage Test",
        "version": "1.0.0",
        "identifier": "dev.tabbyrs.storage-test",
        "app": { "windows": windows }
    }))
    .expect("fixture should deserialize as a real Tauri Config")
}

fn snapshot(config: &Config) -> Value {
    serde_json::to_value(config).expect("Tauri Config should serialize")
}

fn window(label: &str, title: &str, directory: &str, incognito: bool) -> Value {
    json!({
        "label": label,
        "title": title,
        "width": 913.0,
        "height": 617.0,
        "url": "index.html",
        "incognito": incognito,
        "dataDirectory": directory,
        "visible": false,
        "center": true
    })
}

fn configured_windows(value: &Value) -> &[Value] {
    value["app"]["windows"]
        .as_array()
        .expect("windows should serialize")
}

#[test]
fn installed_config_is_preserved_exactly() {
    let mut config = config_with_windows(json!([
        window("main", "Main", "custom/main-profile", true),
        window("secondary", "Secondary", "../other-profile", false)
    ]));
    let before = snapshot(&config);

    assert_eq!(prepare(&mut config, None).unwrap(), None);
    assert_eq!(snapshot(&config), before);
}

#[test]
fn portable_prepare_selects_directory_for_every_configured_window() {
    let temp = tempfile::tempdir().unwrap();
    let portable_root = temp.path().join("portable root");
    fs::create_dir(&portable_root).unwrap();
    let expected = portable_root.join("data").join("webview");
    let mut config = config_with_windows(json!([
        window("main", "Primary title", "old-main", true),
        window("secondary", "Secondary title", "old-secondary", false)
    ]));
    let before = snapshot(&config);

    assert_eq!(
        prepare(&mut config, Some(&portable_root)).unwrap(),
        Some(expected.clone())
    );
    assert!(expected.is_dir());

    let after = snapshot(&config);
    let before_windows = configured_windows(&before);
    let after_windows = configured_windows(&after);
    assert_eq!(after_windows.len(), 2);
    for (before, after) in before_windows.iter().zip(after_windows) {
        assert_eq!(after["dataDirectory"], json!(expected));
        for field in [
            "label",
            "title",
            "width",
            "height",
            "url",
            "incognito",
            "visible",
            "center",
        ] {
            assert_eq!(after[field], before[field], "window field {field} changed");
        }
    }
}

#[test]
fn portable_prepare_creates_directory_even_without_configured_windows() {
    let temp = tempfile::tempdir().unwrap();
    let expected = temp.path().join("data/webview");
    let mut config = config_with_windows(json!([]));

    assert_eq!(
        prepare(&mut config, Some(temp.path())).unwrap(),
        Some(expected.clone())
    );
    assert!(expected.is_dir());
    assert!(configured_windows(&snapshot(&config)).is_empty());
}

#[test]
fn repeated_prepare_preserves_owned_data_and_directory() {
    let temp = tempfile::tempdir().unwrap();
    let portable_root = temp.path();
    let expected = portable_root.join("data/webview");
    let mut config = config_with_windows(json!([window("main", "Main", "old", false)]));
    assert_eq!(
        prepare(&mut config, Some(portable_root)).unwrap(),
        Some(expected.clone())
    );

    let sentinel = expected.join("owned-sentinel");
    fs::write(&sentinel, b"keep this exact content").unwrap();
    assert_eq!(
        prepare(&mut config, Some(portable_root)).unwrap(),
        Some(expected.clone())
    );

    assert_eq!(fs::read(sentinel).unwrap(), b"keep this exact content");
    assert!(expected.is_dir());
}

#[test]
fn portable_prepare_rejects_files_at_data_and_webview_without_mutation() {
    for relative_file in [Path::new("data"), Path::new("data/webview")] {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(relative_file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"preserve these bytes").unwrap();
        let mut config = config_with_windows(json!([window("main", "Main", "custom", true)]));
        let before = snapshot(&config);

        assert!(
            prepare(&mut config, Some(temp.path())).is_err(),
            "{}",
            file.display()
        );
        assert_eq!(snapshot(&config), before);
        assert_eq!(fs::read(file).unwrap(), b"preserve these bytes");
    }
}

#[test]
fn secondary_window_resolver_matches_prepare_selection() {
    let temp = tempfile::tempdir().unwrap();
    let installed_snapshot = PathBuf::from("existing-secondary-profile");
    assert_eq!(data_directory(None), None);

    let root = temp.path().join("portable");
    fs::create_dir(&root).unwrap();
    let selected = data_directory(Some(&root));
    assert_eq!(selected, Some(root.join("data/webview")));
    assert_ne!(selected, Some(installed_snapshot));
}

#[test]
fn portable_paths_keep_spaces_and_non_ascii_filesystem_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("可攜式 資料");
    fs::create_dir(&root).unwrap();
    let expected = root.join("data").join("webview");
    let mut config = config_with_windows(json!([window("main", "Main", "old", false)]));

    assert_eq!(data_directory(Some(&root)), Some(expected.clone()));
    assert_eq!(
        prepare(&mut config, Some(&root)).unwrap(),
        Some(expected.clone())
    );
    assert!(expected.is_dir());
    assert_eq!(
        fs::canonicalize(&expected).unwrap(),
        fs::canonicalize(root.join("data/webview")).unwrap()
    );
    assert_eq!(
        configured_windows(&snapshot(&config))[0]["dataDirectory"],
        json!(expected)
    );
}
