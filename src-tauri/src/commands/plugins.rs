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
    pub code: Option<String>,
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
    code: Option<String>,
    _message: String,
) {
    let phase = normalize_failure_phase(&phase);
    let code = code.as_deref().map(normalize_failure_code);
    state.safe_mode.failure_phase = Some(phase.into());
    state.safe_mode.failure_code = code.map(Into::into);
    state.safe_mode.failure_message = Some(match code {
        Some(code) => format!("Plugin bootstrap failed during {phase} ({code})"),
        None => format!("Plugin bootstrap failed during {phase}"),
    });
    if let Some(package_name) = package_name {
        if !state.safe_mode.suspected_plugins.contains(&package_name) {
            state.safe_mode.suspected_plugins.push(package_name);
        }
    } else if state.safe_mode.suspected_plugins.is_empty() {
        state.safe_mode.suspected_plugins = state.safe_mode.plugins.clone();
    }
}

fn normalize_failure_phase(phase: &str) -> &'static str {
    match phase {
        "discover" => "discover",
        "read" => "read",
        "evaluate" => "evaluate",
        "angular-bootstrap" => "angular-bootstrap",
        _ => "unknown",
    }
}

fn normalize_failure_code(code: &str) -> &'static str {
    match code {
        "missing-module" => "missing-module",
        "node-runtime-required" => "node-runtime-required",
        "invalid-export" => "invalid-export",
        "exception" => "exception",
        "discover" => "discover",
        "angular-bootstrap" => "angular-bootstrap",
        _ => "unknown",
    }
}

fn clear_bootstrap_attempt(state: &mut TabbyRsState) {
    state.safe_mode.attempt_id = None;
    state.safe_mode.started_at = None;
    state.safe_mode.plugins.clear();
    state.safe_mode.last_started_plugin = None;
    state.safe_mode.last_completed_plugin = None;
    state.safe_mode.failure_phase = None;
    state.safe_mode.failure_code = None;
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
            request.code,
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
        mark_bootstrap_succeeded, normalize_failure_code, normalize_failure_phase,
        prepare_bootstrap_retry,
    };
    use crate::commands::app::{bootstrap_mode, safe_mode_suspected_plugins};
    use crate::storage::state_file::{load_state, save_state, TabbyRsState};
    use tempfile::tempdir;

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
            Some("node-runtime-required".into()),
            "unsupported module".into(),
        );

        assert_eq!(
            state.safe_mode.last_started_plugin.as_deref(),
            Some("tabby-broken")
        );
        assert_eq!(state.safe_mode.last_completed_plugin, None);
        assert_eq!(state.safe_mode.failure_phase.as_deref(), Some("evaluate"));
        assert_eq!(
            state.safe_mode.failure_code.as_deref(),
            Some("node-runtime-required")
        );
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
        assert_eq!(state.safe_mode.failure_code, None);
    }

    #[test]
    fn global_failure_marks_pending_plugins_without_duplicates() {
        let mut state = TabbyRsState::default();
        state.safe_mode.plugins = vec!["tabby-one".into(), "tabby-two".into()];

        journal_plugin_failure(
            &mut state,
            None,
            "angular-bootstrap".into(),
            Some("angular-bootstrap".into()),
            "root module failed".into(),
        );
        journal_plugin_failure(
            &mut state,
            None,
            "angular-bootstrap".into(),
            Some("angular-bootstrap".into()),
            "root module failed again".into(),
        );

        assert_eq!(
            state.safe_mode.suspected_plugins,
            vec!["tabby-one", "tabby-two"]
        );
        assert_eq!(
            state.safe_mode.failure_message.as_deref(),
            Some("Plugin bootstrap failed during angular-bootstrap (angular-bootstrap)")
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

    #[test]
    fn persists_only_bounded_failure_metadata() {
        let mut state = TabbyRsState::default();

        journal_plugin_failure(
            &mut state,
            Some("tabby-broken".into()),
            "unexpected-phase".into(),
            Some("secret-value".into()),
            "secret-value from /Users/alice/.env".into(),
        );

        assert_eq!(state.safe_mode.failure_phase.as_deref(), Some("unknown"));
        assert_eq!(state.safe_mode.failure_code.as_deref(), Some("unknown"));
        assert_eq!(
            state.safe_mode.failure_message.as_deref(),
            Some("Plugin bootstrap failed during unknown (unknown)")
        );
        assert!(!state
            .safe_mode
            .failure_message
            .as_deref()
            .unwrap()
            .contains("secret-value"));
        assert_eq!(normalize_failure_phase("read"), "read");
        assert_eq!(normalize_failure_code("exception"), "exception");
    }

    #[test]
    fn safe_mode_restart_retry_returns_to_normal_boot() {
        let discovered = vec!["tabby-good".to_owned(), "tabby-broken".to_owned()];
        let mut state = TabbyRsState::default();
        let temp = tempdir().unwrap();
        let state_path = temp.path().join("tabby-rs.json");

        let normal = bootstrap_mode(&state, Ok(discovered.clone()));
        assert_eq!(normal, (false, None, discovered.clone()));

        state.safe_mode.attempt_id = Some("attempt-1".into());
        state.safe_mode.plugins = discovered.clone();
        journal_plugin_started(&mut state, "tabby-broken".into());
        journal_plugin_failure(
            &mut state,
            Some("tabby-broken".into()),
            "evaluate".into(),
            Some("exception".into()),
            "fixture failed".into(),
        );
        save_state(&state_path, &state).unwrap();
        state = load_state(&state_path).unwrap();

        let after_crash = bootstrap_mode(&state, Ok(discovered.clone()));
        assert_eq!(after_crash.0, true);
        assert_eq!(after_crash.2, Vec::<String>::new());
        assert_eq!(
            safe_mode_suspected_plugins(&state),
            vec!["tabby-broken", "tabby-good"]
        );

        prepare_bootstrap_retry(&mut state);
        save_state(&state_path, &state).unwrap();
        state = load_state(&state_path).unwrap();
        let after_retry = bootstrap_mode(&state, Ok(discovered.clone()));
        assert_eq!(after_retry, (false, None, discovered.clone()));

        state.safe_mode.attempt_id = Some("attempt-2".into());
        state.safe_mode.plugins = discovered;
        journal_plugin_started(&mut state, "tabby-good".into());
        journal_plugin_completed(&mut state, "tabby-good".into());
        mark_bootstrap_succeeded(&mut state);

        assert!(state.safe_mode.attempt_id.is_none());
        assert!(state.safe_mode.plugins.is_empty());
        assert!(state.safe_mode.suspected_plugins.is_empty());
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
pub struct PluginUpdateRequest {
    pub operation_id: String,
    pub package_name: String,
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
    let (node_path, npm_path) = match npm_toolchain(request.custom_node_path).await {
        Ok(toolchain) => toolchain,
        Err(error) => {
            state.plugin_operations().finish(&request.operation_id);
            return Err(error);
        }
    };
    let cancel = state.plugin_operations().register(&request.operation_id)?;
    let operation = running_operation(&request.operation_id, &request.package_name, "install");
    if let Err(error) = app.emit("plugins:operation", &operation) {
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
        let _ = app.emit("plugins:operation", operation);
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
    let (node_path, npm_path) = match npm_toolchain(request.custom_node_path).await {
        Ok(toolchain) => toolchain,
        Err(error) => {
            state.plugin_operations().finish(&request.operation_id);
            return Err(error);
        }
    };
    let cancel = state.plugin_operations().register(&request.operation_id)?;
    let operation = running_operation(&request.operation_id, &request.package_name, "uninstall");
    if let Err(error) = app.emit("plugins:operation", &operation) {
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
        let _ = app.emit("plugins:operation", operation);
    });
    Ok(operation)
}

#[tauri::command]
pub async fn plugins_update(
    app: tauri::AppHandle,
    request: PluginUpdateRequest,
    state: tauri::State<'_, AppState>,
) -> Result<npm::PluginOperation, AppError> {
    let root = state.paths().plugins_dir().clone();
    let (node_path, npm_path) = match npm_toolchain(request.custom_node_path).await {
        Ok(toolchain) => toolchain,
        Err(error) => {
            state.plugin_operations().finish(&request.operation_id);
            return Err(error);
        }
    };
    let cancel = state.plugin_operations().register(&request.operation_id)?;
    let operation = running_operation(&request.operation_id, &request.package_name, "update");
    if let Err(error) = app.emit("plugins:operation", &operation) {
        state.plugin_operations().finish(&request.operation_id);
        return Err(AppError::Io(error.to_string()));
    }
    let operations = state.plugin_operations().clone();
    let operation_id = request.operation_id;
    let package_name = request.package_name;
    let progress = operation_progress(&app, &operation_id, &package_name, "update");
    tauri::async_runtime::spawn(async move {
        let result = npm::update(
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
                "update",
                if matches!(error, AppError::Conflict(_)) {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(error.to_string()),
            ),
        };
        let _ = app.emit("plugins:operation", operation);
    });
    Ok(operation)
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
pub fn plugins_prepare_operation(
    request: PluginOperationRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.plugin_operations().reserve(&request.id)
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
            "plugins:operation",
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
