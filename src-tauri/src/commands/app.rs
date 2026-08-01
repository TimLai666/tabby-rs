use std::{collections::BTreeMap, fs};

use tauri::{AppHandle, State};

use crate::{error::AppError, state::AppState};

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

#[tauri::command]
pub fn app_bootstrap(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<BootstrapData, AppError> {
    let _ = request;
    fs::create_dir_all(state.paths().plugins_dir())?;

    Ok(BootstrapData {
        config: BTreeMap::new(),
        executable: state.paths().executable().to_string_lossy().into_owned(),
        is_main_window: true,
        window_id: state.next_window_id(),
        installed_plugins: vec![
            built_in_plugin("core", "tabby-core", "Tabby core UI"),
            built_in_plugin("settings", "tabby-settings", "Tabby settings UI"),
            built_in_plugin("tauri", "tabby-tauri", "Tabby RS Tauri host providers"),
        ],
        user_plugins_path: state.paths().plugins_dir().to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn app_runtime_info(request: EmptyRequest) -> Result<RuntimeInfo, AppError> {
    let _ = request;
    Ok(current_runtime_info())
}

#[tauri::command]
pub fn app_quit(request: EmptyRequest, app: AppHandle) -> Result<(), AppError> {
    let _ = request;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{current_runtime_info, BootstrapData};

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
        };
        let value = serde_json::to_value(data).unwrap();
        assert_eq!(value["windowID"], 1);
        assert!(value.get("windowId").is_none());
    }
}
