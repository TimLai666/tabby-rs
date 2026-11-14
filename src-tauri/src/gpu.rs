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

/// Returns the variables to export when `hacks.disableGPU` is exactly `true`.
pub(crate) fn environment_for_config(
    config: &BTreeMap<String, Value>,
) -> Option<Vec<(&'static str, &'static str)>> {
    let disabled = config
        .get("hacks")
        .and_then(Value::as_object)
        .and_then(|hacks| hacks.get("disableGPU"))
        .and_then(Value::as_bool)
        == Some(true);
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
    for (name, value) in variables_to_set(&variables, |name| std::env::var_os(name)) {
        std::env::set_var(name, value);
    }
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
    fn non_boolean_disable_gpu_values_export_nothing() {
        for value in [
            json!("true"),
            json!(1),
            json!("false"),
            Value::Null,
            json!([]),
        ] {
            let config = config_with_hacks(value.clone());
            assert_eq!(environment_for_config(&config), None, "value={value:?}");
        }
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
