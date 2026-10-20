use super::{prepare_startup, DEFAULT_WINDOW_SIZE, MIN_WINDOW_SIZE};
use serde_json::{json, to_value, Value};
use tauri::utils::config::Config;

fn stock_config() -> Config {
    serde_json::from_str(include_str!("../tauri.conf.json")).unwrap()
}

#[test]
fn stock_primary_geometry_is_prepared_before_construction() {
    let mut config = stock_config();
    let expected = to_value(&config).unwrap();
    #[cfg(target_os = "macos")]
    let expected = {
        let mut expected = expected;
        assert_eq!(DEFAULT_WINDOW_SIZE, (800.0, 600.0));
        assert_eq!(MIN_WINDOW_SIZE, (400.0, 300.0));
        expected["app"]["windows"][0]["width"] = json!(800.0);
        expected["app"]["windows"][0]["height"] = json!(600.0);
        expected["app"]["windows"][0]["minWidth"] = json!(400.0);
        expected["app"]["windows"][0]["minHeight"] = json!(300.0);
        expected
    };
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(DEFAULT_WINDOW_SIZE, (1100.0, 720.0));
        assert_eq!(MIN_WINDOW_SIZE, (640.0, 480.0));
    }
    prepare_startup(&mut config);
    assert_eq!(to_value(&config).unwrap(), expected);
    prepare_startup(&mut config);
    assert_eq!(
        to_value(&config).unwrap(),
        expected,
        "preparation is idempotent"
    );
}

#[test]
fn custom_primary_geometry_and_title_style_are_preserved() {
    for (field, value) in [
        ("width", json!(900.0)),
        ("height", json!(700.0)),
        ("minWidth", json!(420.0)),
        ("minHeight", json!(320.0)),
        ("minWidth", Value::Null),
        ("minHeight", Value::Null),
        ("titleBarStyle", json!("Visible")),
    ] {
        let mut declared = to_value(stock_config()).unwrap();
        declared["app"]["windows"][0][field] = value;
        let mut config: Config = serde_json::from_value(declared).unwrap();
        let expected = to_value(&config).unwrap();
        prepare_startup(&mut config);
        assert_eq!(to_value(config).unwrap(), expected, "custom {field}");
    }
}

#[test]
fn additional_windows_keep_all_configuration_and_creation_policy() {
    let mut config = stock_config();
    let mut other = config.app.windows[0].clone();
    other.label = "tools".into();
    other.create = false;
    other.x = Some(42.0);
    other.y = Some(73.0);
    config.app.windows.insert(0, other.clone());
    prepare_startup(&mut config);
    assert_eq!(
        to_value(&config.app.windows[0]).unwrap(),
        to_value(other).unwrap()
    );
    assert_eq!(config.app.windows[1].label, "main");
    assert_eq!(config.app.windows[1].width, DEFAULT_WINDOW_SIZE.0);
}

#[test]
fn absent_primary_window_does_not_create_or_relabel_windows() {
    let mut config = stock_config();
    config.app.windows[0].label = "tools".into();
    let expected = to_value(&config).unwrap();
    prepare_startup(&mut config);
    assert_eq!(to_value(&config).unwrap(), expected);
    config.app.windows.clear();
    prepare_startup(&mut config);
    assert!(config.app.windows.is_empty());
}
