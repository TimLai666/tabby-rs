//! Startup GPU workaround for `hacks.disableGPU`, matching upstream
//! `app/lib/app.ts`, which disables hardware acceleration when the raw
//! configuration asks for it.

use std::{collections::BTreeMap, ffi::OsString};

use serde_json::Value;

use crate::{identity::AppPaths, storage::config_file::read_config, storage::paths::StoragePaths};

/// WebKitGTK reads these variables when a WebView is created on Linux.
const DISABLE_GPU_VARIABLES: [(&str, &str); 2] = [
    ("WEBKIT_DISABLE_COMPOSITING_MODE", "1"),
    ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
];

/// Names the variables this process exported itself, so child processes
/// (PTY shells, `desktop_exec`) and an in-app restart can tell them apart
/// from values the user exported. Upstream's `disableHardwareAcceleration`
/// affects only the Electron process; these variables must not leak further.
pub(crate) const OWNED_VARIABLES_MARKER: &str = "TABBY_RS_GPU_ENV";

/// Variables to drop from an environment handed to another process: the
/// marker itself plus every variable it names.
pub(crate) fn owned_variables(marker: Option<&str>) -> Vec<String> {
    let mut names = vec![OWNED_VARIABLES_MARKER.to_owned()];
    if let Some(marker) = marker {
        names.extend(
            marker
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        );
    }
    names
}

/// Removes the process-owned GPU variables from an environment map before it
/// reaches a child process. The marker value inside the map decides which
/// names are owned, so a map built from another process is handled the same.
pub(crate) fn strip_owned_variables<V: AsRef<str>>(
    environment: &mut std::collections::BTreeMap<String, V>,
) {
    let marker = environment
        .get(OWNED_VARIABLES_MARKER)
        .map(|value| value.as_ref().to_owned());
    for name in owned_variables(marker.as_deref()) {
        environment.remove(&name);
    }
}

/// Returns the variables to export when `hacks.disableGPU` is exactly `true`.
pub(crate) fn environment_for_config(
    config: &BTreeMap<String, Value>,
) -> Option<Vec<(&'static str, &'static str)>> {
    // Upstream app/lib/app.ts checks `hacks?.disableGPU` for JavaScript
    // truthiness, so a non-boolean value written by hand also counts.
    let disabled = config
        .get("hacks")
        .and_then(Value::as_object)
        .and_then(|hacks| hacks.get("disableGPU"))
        .is_some_and(js_truthy);
    disabled.then(|| DISABLE_GPU_VARIABLES.to_vec())
}

/// Parses raw `config.yaml` text. Invalid YAML and encrypted wrappers without a
/// raw `hacks` section disable nothing; the bootstrap reports those errors later.
pub(crate) fn environment_for_yaml(yaml: &str) -> Option<Vec<(&'static str, &'static str)>> {
    let config: BTreeMap<String, Value> = serde_yaml::from_str(yaml).ok()?;
    environment_for_config(&config)
}

/// Keeps only the variables the user has not already exported, including
/// variables exported as an empty string.
fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub(crate) fn variables_to_set(
    variables: &[(&'static str, &'static str)],
    existing: impl Fn(&str) -> Option<OsString>,
) -> Vec<(&'static str, &'static str)> {
    variables
        .iter()
        .copied()
        .filter(|(name, _)| existing(name).is_none())
        .collect()
}

/// Exports the GPU workaround before any WebView is created. Only Linux
/// applies it; other platforms return without reading the configuration.
pub(crate) fn apply_startup_environment(paths: &AppPaths) {
    if !cfg!(target_os = "linux") {
        return;
    }
    // An in-app restart inherits this process's environment. Drop what the
    // previous process exported itself so a disabled setting takes effect and
    // the user's own exports stay untouched.
    if let Ok(marker) = std::env::var(OWNED_VARIABLES_MARKER) {
        for name in owned_variables(Some(&marker)) {
            std::env::remove_var(name);
        }
    }
    let config = match read_config(StoragePaths::from_app_paths(paths).config_file()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("could not read config for the GPU startup setting: {error}");
            return;
        }
    };
    let Some(variables) = environment_for_yaml(&config.yaml) else {
        return;
    };
    let owned = variables_to_set(&variables, |name| std::env::var_os(name));
    if owned.is_empty() {
        return;
    }
    for (name, value) in &owned {
        std::env::set_var(name, value);
    }
    let names: Vec<&str> = owned.iter().map(|(name, _)| *name).collect();
    std::env::set_var(OWNED_VARIABLES_MARKER, names.join(","));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn config_with_hacks(value: Value) -> BTreeMap<String, Value> {
        BTreeMap::from([("hacks".to_owned(), json!({ "disableGPU": value }))])
    }

    fn both() -> Vec<(&'static str, &'static str)> {
        DISABLE_GPU_VARIABLES.to_vec()
    }

    #[test]
    fn disable_gpu_true_exports_both_webkit_variables() {
        assert_eq!(
            environment_for_config(&config_with_hacks(json!(true))),
            Some(both())
        );
    }

    #[test]
    fn disable_gpu_false_missing_or_non_object_hacks_export_nothing() {
        let configs = [
            config_with_hacks(json!(false)),
            BTreeMap::new(),
            BTreeMap::from([("hacks".to_owned(), json!({}))]),
            BTreeMap::from([("hacks".to_owned(), json!(true))]),
            BTreeMap::from([("hacks".to_owned(), Value::Null)]),
        ];
        for config in configs {
            assert_eq!(environment_for_config(&config), None, "config={config:?}");
        }
    }

    #[test]
    fn disable_gpu_follows_javascript_truthiness_like_upstream() {
        for value in [json!("true"), json!(1), json!([]), json!({})] {
            let config = config_with_hacks(value.clone());
            assert_eq!(
                environment_for_config(&config),
                Some(both()),
                "value={value:?}"
            );
        }
        for value in [json!(false), json!(0), json!(""), Value::Null] {
            let config = config_with_hacks(value.clone());
            assert_eq!(environment_for_config(&config), None, "value={value:?}");
        }
    }

    #[test]
    fn owned_variables_come_from_the_marker_and_strip_from_child_environments() {
        assert_eq!(
            owned_variables(None),
            vec![OWNED_VARIABLES_MARKER.to_owned()]
        );
        assert_eq!(
            owned_variables(Some(
                "WEBKIT_DISABLE_COMPOSITING_MODE, ,WEBKIT_DISABLE_DMABUF_RENDERER"
            )),
            vec![
                OWNED_VARIABLES_MARKER.to_owned(),
                "WEBKIT_DISABLE_COMPOSITING_MODE".to_owned(),
                "WEBKIT_DISABLE_DMABUF_RENDERER".to_owned(),
            ]
        );
        // Only the variables the marker names are removed; a user export of the
        // other WebKit variable survives, as does unrelated environment.
        let mut environment = BTreeMap::from([
            (
                OWNED_VARIABLES_MARKER.to_owned(),
                "WEBKIT_DISABLE_COMPOSITING_MODE".to_owned(),
            ),
            ("WEBKIT_DISABLE_COMPOSITING_MODE".to_owned(), "1".to_owned()),
            ("WEBKIT_DISABLE_DMABUF_RENDERER".to_owned(), "1".to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
        ]);
        strip_owned_variables(&mut environment);
        assert_eq!(
            environment,
            BTreeMap::from([
                ("WEBKIT_DISABLE_DMABUF_RENDERER".to_owned(), "1".to_owned()),
                ("TERM".to_owned(), "xterm-256color".to_owned()),
            ])
        );
        // Without a marker nothing is owned, so a user's own export stays.
        let mut unmarked =
            BTreeMap::from([("WEBKIT_DISABLE_COMPOSITING_MODE".to_owned(), "1".to_owned())]);
        strip_owned_variables(&mut unmarked);
        assert_eq!(unmarked.len(), 1);
    }

    #[test]
    fn raw_yaml_with_disable_gpu_true_exports_both_variables() {
        assert_eq!(
            environment_for_yaml("hacks:\n  disableGPU: true\n"),
            Some(both())
        );
    }

    #[test]
    fn invalid_empty_or_encrypted_yaml_exports_nothing() {
        assert_eq!(environment_for_yaml("hacks: [\n"), None);
        assert_eq!(environment_for_yaml(""), None);
        assert_eq!(
            environment_for_yaml("encrypted:\n  version: 1\n  ciphertext: opaque\n"),
            None
        );
    }

    #[test]
    fn exported_variables_are_not_overridden() {
        let existing = |name: &str| match name {
            "WEBKIT_DISABLE_COMPOSITING_MODE" => Some(OsString::from("0")),
            _ => None,
        };
        assert_eq!(
            variables_to_set(&both(), existing),
            vec![("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]
        );
    }

    #[test]
    fn empty_exported_value_counts_as_set() {
        let existing = |_: &str| Some(OsString::new());
        assert!(variables_to_set(&both(), existing).is_empty());
    }

    #[test]
    fn unset_variables_are_all_exported() {
        assert_eq!(variables_to_set(&both(), |_| None), both());
    }
}
