use std::{collections::BTreeMap, fs};

use chrono::Utc;
use tauri::{AppHandle, State};

use crate::{
    error::AppError,
    plugins::manifest,
    state::AppState,
    storage::{config_file::read_config, paths::StoragePaths, state_file::TabbyRsState},
};

#[derive(Debug, Default, serde::Deserialize)]
pub struct EmptyRequest {}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub host: &'static str,
    pub platform: String,
    pub arch: String,
    pub version: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub package_name: String,
    pub is_builtin: bool,
    pub is_legacy: bool,
    pub version: String,
    pub author: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapData {
    pub config: BTreeMap<String, serde_json::Value>,
    pub executable: String,
    pub is_main_window: bool,
    #[serde(rename = "windowID")]
    pub window_id: u64,
    pub installed_plugins: Vec<PluginInfo>,
    pub user_plugins_path: String,
    pub safe_mode: bool,
    pub safe_mode_reason: Option<String>,
    pub safe_mode_suspected_plugins: Vec<String>,
}

fn current_runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        host: "tauri",
        platform: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

fn built_in_plugin(name: &str, package_name: &str, description: &str) -> PluginInfo {
    PluginInfo {
        name: name.to_owned(),
        description: description.to_owned(),
        package_name: package_name.to_owned(),
        is_builtin: true,
        is_legacy: false,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        author: "Tabby Developers and Tabby RS Contributors".to_owned(),
    }
}

fn user_plugin(plugin: manifest::InstalledPlugin) -> PluginInfo {
    PluginInfo {
        name: plugin.name,
        description: plugin.description,
        package_name: plugin.package_name,
        is_builtin: plugin.is_builtin,
        is_legacy: plugin.is_legacy,
        version: plugin.version,
        author: plugin.author,
    }
}

fn bootstrap_attempt_id() -> String {
    format!(
        "{}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    )
}

fn bootstrap_config(state: &AppState) -> Result<BTreeMap<String, serde_json::Value>, AppError> {
    let paths = StoragePaths::from_app_paths(state.paths());
    let config = read_config(paths.config_file())?;
    parse_bootstrap_config(&config.yaml)
}

fn parse_bootstrap_config(yaml: &str) -> Result<BTreeMap<String, serde_json::Value>, AppError> {
    if yaml.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_yaml::from_str(yaml)
        .map_err(|error| AppError::InvalidData(format!("config.yaml is invalid: {error}")))
}

fn bootstrap_mode(
    previous: &TabbyRsState,
    discovered: Result<Vec<String>, String>,
) -> (bool, Option<String>, Vec<String>) {
    if previous.safe_mode.attempt_id.is_some() {
        return (
            true,
            previous
                .safe_mode
                .failure_message
                .clone()
                .or_else(|| Some("The previous plugin startup did not complete.".into())),
            Vec::new(),
        );
    }

    match discovered {
        Ok(packages) => (false, None, packages),
        Err(error) => (true, Some(error), Vec::new()),
    }
}

#[tauri::command]
pub fn app_bootstrap(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<BootstrapData, AppError> {
    let _ = request;
    fs::create_dir_all(state.paths().plugins_dir())?;
    let config = bootstrap_config(&state)?;

    let previous = state.persisted_state();
    let discovered = if previous.safe_mode.attempt_id.is_some() {
        Ok(Vec::new())
    } else {
        manifest::discover(state.paths().plugins_dir())
            .map(|plugins| {
                plugins
                    .into_iter()
                    .map(|plugin| plugin.package_name)
                    .collect()
            })
            .map_err(|error| error.to_string())
    };
    let (safe_mode, safe_mode_reason, plugin_packages) = bootstrap_mode(&previous, discovered);
    let suspected_plugins = if safe_mode {
        previous.safe_mode.suspected_plugins.clone()
    } else {
        Vec::new()
    };
    let failure_message = safe_mode_reason.clone();
    state.update_persisted_state(|persisted| {
        persisted.safe_mode.last_forced = safe_mode;
        persisted.safe_mode.suspected_plugins = suspected_plugins.clone();
        persisted.safe_mode.attempt_id = Some(bootstrap_attempt_id());
        persisted.safe_mode.started_at = Some(Utc::now());
        persisted.safe_mode.plugins = plugin_packages.clone();
        persisted.safe_mode.last_started_plugin = None;
        persisted.safe_mode.last_completed_plugin = None;
        persisted.safe_mode.failure_phase = safe_mode.then_some("discover".into());
        persisted.safe_mode.failure_message = failure_message.clone();
    })?;

    let user_plugins = manifest::list_installed(state.paths().plugins_dir())
        .unwrap_or_default()
        .into_iter()
        .map(user_plugin)
        .collect::<Vec<_>>();
    let mut installed_plugins = vec![
        built_in_plugin("core", "tabby-core", "Tabby core UI"),
        built_in_plugin("settings", "tabby-settings", "Tabby settings UI"),
        built_in_plugin("tauri", "tabby-tauri", "Tabby RS Tauri host providers"),
        built_in_plugin(
            "plugin-manager",
            "tabby-plugin-manager",
            "Tabby plugin manager",
        ),
    ];
    installed_plugins.extend(user_plugins);

    Ok(BootstrapData {
        config,
        executable: state.paths().executable().to_string_lossy().into_owned(),
        is_main_window: true,
        window_id: state.next_window_id(),
        installed_plugins,
        user_plugins_path: state.paths().plugins_dir().to_string_lossy().into_owned(),
        safe_mode,
        safe_mode_reason,
        safe_mode_suspected_plugins: suspected_plugins,
    })
}

#[tauri::command]
pub fn app_runtime_info(request: EmptyRequest) -> Result<RuntimeInfo, AppError> {
    let _ = request;
    Ok(current_runtime_info())
}

#[tauri::command]
pub fn app_quit(
    request: EmptyRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let _ = request;
    let _ = crate::diagnostics::crash::clear(state.paths().logs_dir());
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bootstrap_mode, current_runtime_info, BootstrapData};
    use crate::storage::state_file::TabbyRsState;

    #[test]
    fn reports_tauri_runtime() {
        let info = current_runtime_info();
        assert_eq!(info.host, "tauri");
        assert!(!info.platform.is_empty());
        assert!(!info.arch.is_empty());
        assert!(info.version.starts_with("1.0.231-tabbyrs."));
    }

    #[test]
    fn preserves_legacy_window_id_field_name() {
        let data = BootstrapData {
            config: Default::default(),
            executable: "tabby-rs".into(),
            is_main_window: true,
            window_id: 1,
            installed_plugins: Vec::new(),
            user_plugins_path: "plugins".into(),
            safe_mode: false,
            safe_mode_reason: None,
            safe_mode_suspected_plugins: Vec::new(),
        };
        let value = serde_json::to_value(data).unwrap();
        assert_eq!(value["windowID"], 1);
        assert!(value.get("windowId").is_none());
    }

    #[test]
    fn parses_plugin_blacklist_from_bootstrap_config() {
        let config = super::parse_bootstrap_config("pluginBlacklist: [tabby-broken]\n").unwrap();
        assert_eq!(config["pluginBlacklist"][0], "tabby-broken");
    }

    #[test]
    fn unfinished_bootstrap_forces_builtin_only_restart() {
        let mut previous = TabbyRsState::default();
        previous.safe_mode.attempt_id = Some("attempt-1".into());
        previous.safe_mode.failure_message = Some("plugin evaluation failed".into());

        let mode = bootstrap_mode(&previous, Ok(vec!["tabby-good".into()]));

        assert_eq!(
            mode,
            (true, Some("plugin evaluation failed".into()), Vec::new())
        );
    }

    #[test]
    fn clean_bootstrap_uses_discovered_packages() {
        let mode = bootstrap_mode(
            &TabbyRsState::default(),
            Ok(vec!["tabby-good".into(), "tabby-legacy".into()]),
        );

        assert_eq!(
            mode,
            (
                false,
                None,
                vec!["tabby-good".to_owned(), "tabby-legacy".to_owned()]
            )
        );
    }
}
