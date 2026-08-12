use crate::{
    error::AppError,
    plugins::{manifest, node_detect, node_detect::NodeToolchainStatus, npm},
    state::AppState,
    storage::state_file::TabbyRsState,
};
use std::sync::Arc;
use tauri::Emitter;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPackageRequest {
    pub package_name: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginBootstrapFailureRequest {
    pub package_name: Option<String>,
    pub phase: String,
    pub message: String,
}

fn journal_plugin_started(state: &mut TabbyRsState, package_name: String) {
    state.safe_mode.last_started_plugin = Some(package_name);
}

fn journal_plugin_completed(state: &mut TabbyRsState, package_name: String) {
    state.safe_mode.last_completed_plugin = Some(package_name);
}

fn journal_plugin_failure(
    state: &mut TabbyRsState,
    package_name: Option<String>,
    phase: String,
    message: String,
) {
    state.safe_mode.failure_phase = Some(phase);
    state.safe_mode.failure_message = Some(message);
    if let Some(package_name) = package_name {
        if !state.safe_mode.suspected_plugins.contains(&package_name) {
            state.safe_mode.suspected_plugins.push(package_name);
        }
    } else if state.safe_mode.suspected_plugins.is_empty() {
        state.safe_mode.suspected_plugins = state.safe_mode.plugins.clone();
    }
}

fn clear_bootstrap_attempt(state: &mut TabbyRsState) {
    state.safe_mode.attempt_id = None;
    state.safe_mode.started_at = None;
    state.safe_mode.plugins.clear();
    state.safe_mode.last_started_plugin = None;
    state.safe_mode.last_completed_plugin = None;
    state.safe_mode.failure_phase = None;
    state.safe_mode.failure_message = None;
}

fn mark_bootstrap_succeeded(state: &mut TabbyRsState) {
    clear_bootstrap_attempt(state);
    state.safe_mode.suspected_plugins.clear();
}

fn prepare_bootstrap_retry(state: &mut TabbyRsState) {
    state.safe_mode.last_forced = false;
    clear_bootstrap_attempt(state);
}

#[tauri::command]
pub fn plugins_bootstrap_plugin_started(
    request: PluginPackageRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        journal_plugin_started(persisted, request.package_name);
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_plugin_completed(
    request: PluginPackageRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        journal_plugin_completed(persisted, request.package_name);
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_failed(
    request: PluginBootstrapFailureRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        journal_plugin_failure(
            persisted,
            request.package_name,
            request.phase,
            request.message,
        );
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_succeeded(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        mark_bootstrap_succeeded(persisted);
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_retry(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        prepare_bootstrap_retry(persisted);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        journal_plugin_completed, journal_plugin_failure, journal_plugin_started,
        mark_bootstrap_succeeded, prepare_bootstrap_retry,
    };
    use crate::storage::state_file::TabbyRsState;

    #[test]
    fn journal_preserves_crash_context_and_clears_after_success() {
        let mut state = TabbyRsState::default();
        state.safe_mode.attempt_id = Some("attempt-1".into());
        state.safe_mode.plugins = vec!["tabby-broken".into(), "tabby-good".into()];

        journal_plugin_started(&mut state, "tabby-broken".into());
        journal_plugin_failure(
            &mut state,
            Some("tabby-broken".into()),
            "evaluate".into(),
            "unsupported module".into(),
        );

        assert_eq!(
            state.safe_mode.last_started_plugin.as_deref(),
            Some("tabby-broken")
        );
        assert_eq!(state.safe_mode.last_completed_plugin, None);
        assert_eq!(state.safe_mode.failure_phase.as_deref(), Some("evaluate"));
        assert_eq!(state.safe_mode.suspected_plugins, vec!["tabby-broken"]);

        journal_plugin_started(&mut state, "tabby-good".into());
        journal_plugin_completed(&mut state, "tabby-good".into());
        assert_eq!(
            state.safe_mode.last_completed_plugin.as_deref(),
            Some("tabby-good")
        );

        mark_bootstrap_succeeded(&mut state);
        assert_eq!(state.safe_mode.attempt_id, None);
        assert!(state.safe_mode.plugins.is_empty());
        assert!(state.safe_mode.suspected_plugins.is_empty());
    }

    #[test]
    fn global_failure_marks_pending_plugins_without_duplicates() {
        let mut state = TabbyRsState::default();
        state.safe_mode.plugins = vec!["tabby-one".into(), "tabby-two".into()];

        journal_plugin_failure(
            &mut state,
            None,
            "angular-bootstrap".into(),
            "root module failed".into(),
        );
        journal_plugin_failure(
            &mut state,
            None,
            "angular-bootstrap".into(),
            "root module failed again".into(),
        );

        assert_eq!(
            state.safe_mode.suspected_plugins,
            vec!["tabby-one", "tabby-two"]
        );
        assert_eq!(
            state.safe_mode.failure_message.as_deref(),
            Some("root module failed again")
        );
    }

    #[test]
    fn retry_clears_pending_attempt_but_keeps_suspects_for_ui() {
        let mut state = TabbyRsState::default();
        state.safe_mode.last_forced = true;
        state.safe_mode.attempt_id = Some("attempt-1".into());
        state.safe_mode.suspected_plugins = vec!["tabby-broken".into()];

        prepare_bootstrap_retry(&mut state);

        assert!(!state.safe_mode.last_forced);
        assert_eq!(state.safe_mode.attempt_id, None);
        assert_eq!(state.safe_mode.suspected_plugins, vec!["tabby-broken"]);
    }
}

#[tauri::command]
pub fn plugins_discover(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<manifest::PluginDescriptor>, AppError> {
    manifest::discover(state.paths().plugins_dir())
}

#[tauri::command]
pub fn plugins_read_entry(
    request: PluginPackageRequest,
    state: tauri::State<'_, AppState>,
) -> Result<manifest::PluginSource, AppError> {
    manifest::read_entry(state.paths().plugins_dir(), &request.package_name)
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatusRequest {
    pub custom_node_path: Option<String>,
}

#[tauri::command]
pub async fn plugins_node_status(
    request: NodeStatusRequest,
) -> Result<NodeToolchainStatus, AppError> {
    node_detect::detect(request.custom_node_path).await
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallRequest {
    pub operation_id: String,
    pub package_name: String,
    pub version: String,
    pub custom_node_path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginUninstallRequest {
    pub operation_id: String,
    pub package_name: String,
    pub custom_node_path: Option<String>,
}

async fn npm_toolchain(
    custom_node_path: Option<String>,
) -> Result<(std::path::PathBuf, std::path::PathBuf), AppError> {
    let status = node_detect::detect(custom_node_path).await?;
    if !status.supported {
        return Err(AppError::Unsupported(status.reason.unwrap_or_else(|| {
            "Node.js and npm are required for plugin management".into()
        })));
    }
    let node_path = status
        .node_path
        .ok_or_else(|| AppError::Unsupported("Node.js was not found".into()))?;
    let npm_path = status
        .npm_path
        .ok_or_else(|| AppError::Unsupported("npm was not found".into()))?;
    Ok((node_path, npm_path))
}

#[tauri::command]
pub async fn plugins_install(
    app: tauri::AppHandle,
    request: PluginInstallRequest,
    state: tauri::State<'_, AppState>,
) -> Result<npm::PluginOperation, AppError> {
    let root = state.paths().plugins_dir().clone();
    let (node_path, npm_path) = npm_toolchain(request.custom_node_path).await?;
    let cancel = state.plugin_operations().register(&request.operation_id)?;
    let operation = running_operation(&request.operation_id, &request.package_name, "install");
    if let Err(error) = app.emit("plugins.operation", &operation) {
        state.plugin_operations().finish(&request.operation_id);
        return Err(AppError::Io(error.to_string()));
    }
    let operations = state.plugin_operations().clone();
    let operation_id = request.operation_id;
    let package_name = request.package_name;
    let version = request.version;
    let progress = operation_progress(&app, &operation_id, &package_name, "install");
    tauri::async_runtime::spawn(async move {
        let result = npm::install(
            root,
            node_path,
            npm_path,
            &operation_id,
            &package_name,
            &version,
            cancel,
            progress,
        )
        .await;
        operations.finish(&operation_id);
        let operation = match result {
            Ok(operation) => operation,
            Err(error) => completed_operation(
                &operation_id,
                &package_name,
                "install",
                if matches!(error, AppError::Conflict(_)) {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(error.to_string()),
            ),
        };
        let _ = app.emit("plugins.operation", operation);
    });
    Ok(operation)
}

#[tauri::command]
pub async fn plugins_uninstall(
    app: tauri::AppHandle,
    request: PluginUninstallRequest,
    state: tauri::State<'_, AppState>,
) -> Result<npm::PluginOperation, AppError> {
    let root = state.paths().plugins_dir().clone();
    let (node_path, npm_path) = npm_toolchain(request.custom_node_path).await?;
    let cancel = state.plugin_operations().register(&request.operation_id)?;
    let operation = running_operation(&request.operation_id, &request.package_name, "uninstall");
    if let Err(error) = app.emit("plugins.operation", &operation) {
        state.plugin_operations().finish(&request.operation_id);
        return Err(AppError::Io(error.to_string()));
    }
    let operations = state.plugin_operations().clone();
    let operation_id = request.operation_id;
    let package_name = request.package_name;
    let progress = operation_progress(&app, &operation_id, &package_name, "uninstall");
    tauri::async_runtime::spawn(async move {
        let result = npm::uninstall(
            root,
            node_path,
            npm_path,
            &operation_id,
            &package_name,
            cancel,
            progress,
        )
        .await;
        operations.finish(&operation_id);
        let operation = match result {
            Ok(operation) => operation,
            Err(error) => completed_operation(
                &operation_id,
                &package_name,
                "uninstall",
                if matches!(error, AppError::Conflict(_)) {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(error.to_string()),
            ),
        };
        let _ = app.emit("plugins.operation", operation);
    });
    Ok(operation)
}

#[tauri::command]
pub async fn plugins_update(
    app: tauri::AppHandle,
    request: PluginInstallRequest,
    state: tauri::State<'_, AppState>,
) -> Result<npm::PluginOperation, AppError> {
    plugins_install(app, request, state).await
}

#[tauri::command]
pub async fn plugins_remove(
    app: tauri::AppHandle,
    request: PluginUninstallRequest,
    state: tauri::State<'_, AppState>,
) -> Result<npm::PluginOperation, AppError> {
    plugins_uninstall(app, request, state).await
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOperationRequest {
    pub id: String,
}

#[tauri::command]
pub fn plugins_cancel_operation(
    request: PluginOperationRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.plugin_operations().cancel(&request.id)
}

fn running_operation(id: &str, package_name: &str, action: &str) -> npm::PluginOperation {
    completed_operation(id, package_name, action, "running", None)
}

fn completed_operation(
    id: &str,
    package_name: &str,
    action: &str,
    status: &str,
    message: Option<String>,
) -> npm::PluginOperation {
    npm::PluginOperation {
        id: id.into(),
        package_name: package_name.into(),
        action: action.into(),
        status: status.into(),
        message,
    }
}

fn operation_progress(
    app: &tauri::AppHandle,
    operation_id: &str,
    package_name: &str,
    action: &str,
) -> Arc<dyn Fn(String) + Send + Sync> {
    let app = app.clone();
    let operation_id = operation_id.to_owned();
    let package_name = package_name.to_owned();
    let action = action.to_owned();
    Arc::new(move |message| {
        let _ = app.emit(
            "plugins.operation",
            completed_operation(
                &operation_id,
                &package_name,
                &action,
                "running",
                Some(message),
            ),
        );
    })
}

#[tauri::command]
pub fn plugins_list_installed(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<manifest::InstalledPlugin>, AppError> {
    manifest::list_installed(state.paths().plugins_dir())
}
