use std::collections::BTreeMap;

use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    diagnostics::{bundle, logging::LogStatus},
    error::AppError,
    state::AppState,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsAppendRequest {
    pub level: String,
    #[serde(alias = "source")]
    pub target: String,
    pub message: String,
    #[serde(default)]
    pub fields: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub correlation_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsOptions {
    #[serde(default = "default_true")]
    pub include_logs: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExportRequest {
    pub destination: String,
    #[serde(default = "default_true")]
    pub include_logs: bool,
}

fn default_true() -> bool {
    true
}

#[tauri::command]
pub fn diagnostics_append(
    request: DiagnosticsAppendRequest,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if !state.persisted_state().diagnostics.local_logging_enabled {
        return Ok(());
    }
    crate::diagnostics::logging::LogWriter::from_environment(state.paths().logs_dir()).append(
        &request.level,
        &request.target,
        &request.message,
        &request.fields,
        request.correlation_id.as_deref(),
    )
}

#[tauri::command]
pub fn diagnostics_status(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<LogStatus, AppError> {
    let _ = request;
    let enabled = state.persisted_state().diagnostics.local_logging_enabled;
    let mut status =
        crate::diagnostics::logging::LogWriter::from_environment(state.paths().logs_dir())
            .status(enabled)?;
    status.crash_marker_present = crate::diagnostics::crash::exists(state.paths().logs_dir());
    Ok(status)
}

#[tauri::command]
pub fn diagnostics_clear_logs(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let _ = request;
    crate::diagnostics::logging::LogWriter::from_environment(state.paths().logs_dir()).clear()
}

#[tauri::command]
pub fn diagnostics_preview(
    request: DiagnosticsOptions,
    state: State<'_, AppState>,
) -> Result<bundle::DiagnosticsPreview, AppError> {
    bundle::preview(&state, request.include_logs)
}

#[tauri::command]
pub fn diagnostics_export(
    request: DiagnosticsExportRequest,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    bundle::export(&state, &request.destination, request.include_logs)
}
