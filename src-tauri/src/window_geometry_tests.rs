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
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    let expected = {
        let mut expected = expected;
        assert_eq!(DEFAULT_WINDOW_SIZE, (800.0, 600.0));
        assert_eq!(MIN_WINDOW_SIZE, (400.0, 300.0));
        expected["app"]["windows"][0]["width"] = json!(800.0);
        expected["app"]["windows"][0]["height"] = json!(600.0);
        expected["app"]["windows"][0]["minWidth"] = json!(400.0);
        expected["app"]["windows"][0]["minHeight"] = json!(300.0);
        #[cfg(target_os = "linux")]
        if config.app.windows[0].x.is_none() && config.app.windows[0].y.is_none() {
            expected["app"]["windows"][0]["center"] = json!(true);
        }
        expected
    };
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
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

#[test]
fn windows_and_linux_stock_primary_prepares_original_construction_geometry() {
    let mut config = stock_config();
    let mut expected = to_value(&config).unwrap();
    expected["app"]["windows"][0]["width"] = json!(800.0);
    expected["app"]["windows"][0]["height"] = json!(600.0);
    expected["app"]["windows"][0]["minWidth"] = json!(400.0);
    expected["app"]["windows"][0]["minHeight"] = json!(300.0);
    #[cfg(target_os = "linux")]
    {
        expected["app"]["windows"][0]["center"] = json!(true);
    }
    let prepared = super::prepare_stock_startup(&mut config);
    assert_eq!(
        (
            config.app.windows[0].width,
            config.app.windows[0].height,
            config.app.windows[0].min_width,
            config.app.windows[0].min_height
        ),
        (800.0, 600.0, Some(400.0), Some(300.0)),
    );
    assert!(prepared);
    assert_eq!(to_value(&config).unwrap(), expected);
    assert!(!super::prepare_stock_startup(&mut config));
    assert_eq!(to_value(config).unwrap(), expected);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_stock_centering_requires_both_coordinates_to_be_absent() {
    for (x, y) in [
        (Some(0.0), None),
        (None, Some(0.0)),
        (Some(-12.0), None),
        (None, Some(-7.0)),
    ] {
        let mut config = stock_config();
        let window = &mut config.app.windows[0];
        window.x = x;
        window.y = y;
        let before = to_value(&config).unwrap();
        assert!(super::prepare_stock_startup(&mut config));
        let after = to_value(&config).unwrap();
        assert_eq!(
            after["app"]["windows"][0]["x"],
            before["app"]["windows"][0]["x"]
        );
        assert_eq!(
            after["app"]["windows"][0]["y"],
            before["app"]["windows"][0]["y"]
        );
        assert_eq!(
            after["app"]["windows"][0]["center"],
            before["app"]["windows"][0]["center"]
        );
        assert!(!super::prepare_stock_startup(&mut config));
        assert_eq!(to_value(config).unwrap(), after);
    }
}

#[test]
fn windows_and_linux_custom_primary_geometry_and_creation_state_are_preserved() {
    for (field, value) in [
        ("width", json!(900.0)),
        ("height", json!(700.0)),
        ("minWidth", json!(420.0)),
        ("minHeight", json!(320.0)),
        ("minWidth", Value::Null),
        ("minHeight", Value::Null),
        ("maxWidth", json!(1500.0)),
        ("maxHeight", json!(1000.0)),
        ("titleBarStyle", json!("Visible")),
        ("create", json!(false)),
        ("center", json!(true)),
        ("visible", json!(true)),
        ("maximized", json!(true)),
        ("fullscreen", json!(true)),
    ] {
        let mut declared = to_value(stock_config()).unwrap();
        declared["app"]["windows"][0][field] = value;
        let mut config: Config = serde_json::from_value(declared).unwrap();
        let expected = to_value(&config).unwrap();
        assert!(!super::prepare_stock_startup(&mut config), "custom {field}");
        assert_eq!(to_value(config).unwrap(), expected, "custom {field}");
    }
}
