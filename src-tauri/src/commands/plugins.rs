use crate::{
    error::AppError,
    plugins::{manifest, node_detect, node_detect::NodeToolchainStatus, npm},
    state::AppState,
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

#[tauri::command]
pub fn plugins_bootstrap_plugin_started(
    request: PluginPackageRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        persisted.safe_mode.last_started_plugin = Some(request.package_name);
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_plugin_completed(
    request: PluginPackageRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        persisted.safe_mode.last_completed_plugin = Some(request.package_name);
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_failed(
    request: PluginBootstrapFailureRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        persisted.safe_mode.failure_phase = Some(request.phase);
        persisted.safe_mode.failure_message = Some(request.message);
        if let Some(package_name) = request.package_name {
            if !persisted
                .safe_mode
                .suspected_plugins
                .contains(&package_name)
            {
                persisted.safe_mode.suspected_plugins.push(package_name);
            }
        } else if persisted.safe_mode.suspected_plugins.is_empty() {
            persisted.safe_mode.suspected_plugins = persisted.safe_mode.plugins.clone();
        }
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_succeeded(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        persisted.safe_mode.attempt_id = None;
        persisted.safe_mode.started_at = None;
        persisted.safe_mode.plugins.clear();
        persisted.safe_mode.last_started_plugin = None;
        persisted.safe_mode.last_completed_plugin = None;
        persisted.safe_mode.failure_phase = None;
        persisted.safe_mode.failure_message = None;
    })?;
    Ok(())
}

#[tauri::command]
pub fn plugins_bootstrap_retry(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    state.update_persisted_state(|persisted| {
        persisted.safe_mode.last_forced = false;
        persisted.safe_mode.attempt_id = None;
        persisted.safe_mode.started_at = None;
        persisted.safe_mode.plugins.clear();
        persisted.safe_mode.last_started_plugin = None;
        persisted.safe_mode.last_completed_plugin = None;
        persisted.safe_mode.failure_phase = None;
        persisted.safe_mode.failure_message = None;
    })?;
    Ok(())
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
