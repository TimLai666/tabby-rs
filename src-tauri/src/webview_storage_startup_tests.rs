use super::defer_portable_startup;
use serde_json::{json, Value};
use std::path::Path;

fn configured() -> tauri::Config {
    serde_json::from_value(json!({
        "productName": "Portable storage startup test",
        "version": "1.0.0",
        "identifier": "dev.tabbyrs.portable-startup-test",
        "app": { "windows": [
            { "label": "main", "title": "Keep primary title", "width": 913, "height": 617,
              "transparent": true, "visible": false, "incognito": true, "create": true,
              "url": "index.html", "additionalBrowserArgs": "--diagnostic-marker", "dataDirectory": "old-main" },
            { "label": "configured-second", "title": "Keep second title", "create": true,
              "width": 811, "height": 603, "dataDirectory": "old-second" },
            { "label": "manual", "title": "Do not create", "create": false, "dataDirectory": "manual-store" }
        ] }
    })).unwrap()
}
fn snapshot(config: &tauri::Config) -> Value {
    serde_json::to_value(config).unwrap()
}

#[test]
fn installed_startup_remains_untouched() {
    let mut config = configured();
    let before = snapshot(&config);
    assert!(defer_portable_startup(&mut config, None).is_empty());
    assert_eq!(snapshot(&config), before);
}

#[test]
fn portable_startup_retains_all_configured_fields_and_defers_only_automatic_windows() {
    let mut config = configured();
    let before = config.app.windows.clone();
    let directory = Path::new("portable data/webview");
    let deferred = defer_portable_startup(&mut config, Some(directory));
    assert_eq!(deferred.len(), 2);
    for (index, window) in deferred.iter().enumerate() {
        let mut expected = before[index].clone();
        expected.data_directory = Some(directory.to_owned());
        assert_eq!(
            serde_json::to_value(window).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        expected.create = false;
        assert_eq!(
            serde_json::to_value(&config.app.windows[index]).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
    }
    assert_eq!(
        serde_json::to_value(&config.app.windows[2]).unwrap(),
        serde_json::to_value(&before[2]).unwrap()
    );
}

#[test]
fn deferred_windows_cannot_be_claimed_twice() {
    let mut config = configured();
    let directory = Path::new("portable data/webview");
    assert_eq!(
        defer_portable_startup(&mut config, Some(directory)).len(),
        2
    );
    let after = snapshot(&config);
    assert!(defer_portable_startup(&mut config, Some(directory)).is_empty());
    assert_eq!(snapshot(&config), after);
}

#[test]
fn empty_config_does_not_invent_a_primary_window() {
    let mut config = configured();
    config.app.windows.clear();
    let before = snapshot(&config);
    assert!(defer_portable_startup(&mut config, Some(Path::new("portable/webview"))).is_empty());
    assert_eq!(snapshot(&config), before);
}
