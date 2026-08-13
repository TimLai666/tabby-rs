use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    error::AppError,
    identity::AppIdentity,
    plugins::manifest,
    state::AppState,
    storage::{
        config_file::{read_config, write_config, ConfigWriteRequest},
        paths::StoragePaths,
        state_file::TabbyRsState,
    },
};

const INITIAL_CONFIG_YAML: &str = "version: 1\n";

#[derive(Debug, Default, serde::Deserialize)]
pub struct EmptyRequest {}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub host: &'static str,
    pub platform: String,
    pub arch: String,
    pub version: String,
    pub benchmark_ready_file: Option<String>,
    pub benchmark_frame_report_file: Option<String>,
    pub installer_smoke_ready_file: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkFrameReport {
    pub method: String,
    pub samples: u32,
    pub p95_frame_time_ms: f64,
    pub dropped_frame_count: u32,
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
        benchmark_ready_file: benchmark_ready_path()
            .map(|path| path.to_string_lossy().into_owned()),
        benchmark_frame_report_file: benchmark_frame_report_path()
            .map(|path| path.to_string_lossy().into_owned()),
        installer_smoke_ready_file: installer_smoke_ready_path()
            .map(|path| path.to_string_lossy().into_owned()),
    }
}

fn benchmark_ready_path() -> Option<PathBuf> {
    std::env::var_os("TABBY_RS_BENCHMARK_READY_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn benchmark_frame_report_path() -> Option<PathBuf> {
    std::env::var_os("TABBY_RS_BENCHMARK_FRAME_REPORT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn installer_smoke_ready_path() -> Option<PathBuf> {
    std::env::var_os("TABBY_RS_INSTALLER_SMOKE_READY_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn write_benchmark_ready(path: &Path) -> Result<(), AppError> {
    write_ready_marker(path)
}

fn write_installer_smoke_ready(path: &Path, identity: &AppIdentity) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let content = serde_json::json!({
        "schemaVersion": 1,
        "ready": true,
        "identity": identity,
    });
    fs::write(&temporary, serde_json::to_vec_pretty(&content)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn write_ready_marker(path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, "ready\n")?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn write_benchmark_frame_report(
    path: &Path,
    report: &BenchmarkFrameReport,
) -> Result<(), AppError> {
    if report.method.trim().is_empty()
        || report.samples == 0
        || !report.p95_frame_time_ms.is_finite()
        || report.p95_frame_time_ms < 0.0
    {
        return Err(AppError::InvalidArgument(
            "benchmark frame report is invalid".to_owned(),
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let content = serde_json::to_vec_pretty(report)
        .map_err(|_| AppError::InvalidData("benchmark frame report is invalid".into()))?;
    fs::write(&temporary, content)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
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
    parse_bootstrap_config(&ensure_config_file(paths.config_file())?)
}

fn ensure_config_file(path: &Path) -> Result<String, AppError> {
    let config = read_config(path)?;
    if config.revision.is_some() {
        return Ok(config.yaml);
    }

    let request = ConfigWriteRequest {
        yaml: INITIAL_CONFIG_YAML.to_owned(),
        expected_revision: None,
        require_missing: true,
    };
    match write_config(path, &request) {
        Ok(_) => Ok(request.yaml),
        Err(AppError::Conflict(_)) => Ok(read_config(path)?.yaml),
        Err(error) => Err(error),
    }
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

fn safe_mode_suspected_plugins(previous: &TabbyRsState) -> Vec<String> {
    let mut suspected = previous.safe_mode.suspected_plugins.clone();
    if let Some(last_started) = &previous.safe_mode.last_started_plugin {
        if !suspected.contains(last_started) {
            suspected.insert(0, last_started.clone());
        }
    }
    for package_name in &previous.safe_mode.plugins {
        if !suspected.contains(package_name) {
            suspected.push(package_name.clone());
        }
    }
    suspected
}

#[tauri::command]
pub fn app_bootstrap(
    window: WebviewWindow,
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
        safe_mode_suspected_plugins(&previous)
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
        persisted.safe_mode.failure_code = safe_mode.then_some("discover".into());
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
        is_main_window: window.label() == "main",
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
pub fn app_benchmark_ready(request: EmptyRequest) -> Result<(), AppError> {
    let _ = request;
    let path = benchmark_ready_path()
        .ok_or_else(|| AppError::Unsupported("benchmark ready marker is disabled".to_owned()))?;
    write_benchmark_ready(&path)
}

#[tauri::command]
pub fn app_installer_smoke_ready(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let _ = request;
    let path = installer_smoke_ready_path().ok_or_else(|| {
        AppError::Unsupported("installer smoke ready marker is disabled".to_owned())
    })?;
    let identity = state.paths().identity();
    write_installer_smoke_ready(&path, &identity)
}

#[tauri::command]
pub fn app_benchmark_frame_report(request: BenchmarkFrameReport) -> Result<(), AppError> {
    let path = benchmark_frame_report_path()
        .ok_or_else(|| AppError::Unsupported("benchmark frame reporting is disabled".to_owned()))?;
    write_benchmark_frame_report(&path, &request)
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
    use super::{
        bootstrap_mode, current_runtime_info, ensure_config_file, safe_mode_suspected_plugins,
        write_benchmark_frame_report, write_benchmark_ready, write_installer_smoke_ready,
        BenchmarkFrameReport, BootstrapData,
    };
    use crate::identity::AppIdentity;
    use crate::storage::state_file::{load_state, save_state, TabbyRsState};
    use tempfile::tempdir;

    #[test]
    fn reports_tauri_runtime() {
        let info = current_runtime_info();
        assert_eq!(info.host, "tauri");
        assert!(!info.platform.is_empty());
        assert!(!info.arch.is_empty());
        assert!(info.version.starts_with("1.0.231-tabbyrs."));
    }

    #[test]
    fn writes_benchmark_ready_marker_atomically() {
        let directory = tempdir().unwrap();
        let marker = directory.path().join("nested").join("ready.marker");

        write_benchmark_ready(&marker).unwrap();

        assert_eq!(std::fs::read_to_string(marker).unwrap(), "ready\n");
    }

    #[test]
    fn writes_installer_smoke_ready_marker_atomically() {
        let directory = tempdir().unwrap();
        let marker = directory.path().join("nested").join("ready.marker");
        let identity = AppIdentity {
            product_name: "Tabby RS".into(),
            app_identifier: "io.tabbyrs.app".into(),
            cli_name: "tabby-rs".into(),
            url_scheme: "tabby-rs".into(),
            data_dir_name: "tabby-rs".into(),
            credential_service: "tabby-rs".into(),
            executable: "/tmp/tabby-rs".into(),
            data_dir: directory.path().join("data").to_string_lossy().into_owned(),
            plugins_dir: directory
                .path()
                .join("data/plugins")
                .to_string_lossy()
                .into_owned(),
            logs_dir: directory
                .path()
                .join("data/logs")
                .to_string_lossy()
                .into_owned(),
            portable: false,
            portable_root: None,
        };

        write_installer_smoke_ready(&marker, &identity).unwrap();

        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(marker).unwrap()).unwrap();
        assert_eq!(report["schemaVersion"], 1);
        assert_eq!(report["ready"], true);
        assert_eq!(report["identity"]["appIdentifier"], "io.tabbyrs.app");
        assert_eq!(report["identity"]["dataDirName"], "tabby-rs");
    }

    #[test]
    fn creates_initial_config_without_overwriting_existing_config() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("config.yaml");

        assert_eq!(ensure_config_file(&path).unwrap(), "version: 1\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version: 1\n");

        std::fs::write(&path, "version: 1\ncustom: true\n").unwrap();
        assert_eq!(
            ensure_config_file(&path).unwrap(),
            "version: 1\ncustom: true\n"
        );
    }

    #[test]
    fn writes_benchmark_frame_report_replacing_existing_output() {
        let directory = tempdir().unwrap();
        let report_path = directory.path().join("nested").join("frames.json");
        let report = BenchmarkFrameReport {
            method: "requestAnimationFrame trace".into(),
            samples: 120,
            p95_frame_time_ms: 16.7,
            dropped_frame_count: 0,
        };

        write_benchmark_frame_report(&report_path, &report).unwrap();
        let updated_report = BenchmarkFrameReport {
            p95_frame_time_ms: 18.2,
            ..report
        };
        write_benchmark_frame_report(&report_path, &updated_report).unwrap();

        let content: serde_json::Value =
            serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
        assert_eq!(content["p95FrameTimeMs"], 18.2);
        assert_eq!(content["droppedFrameCount"], 0);
    }

    #[test]
    fn rejects_invalid_benchmark_frame_report() {
        let directory = tempdir().unwrap();
        let report_path = directory.path().join("frames.json");
        let report = BenchmarkFrameReport {
            method: String::new(),
            samples: 0,
            p95_frame_time_ms: f64::NAN,
            dropped_frame_count: 0,
        };

        assert!(write_benchmark_frame_report(&report_path, &report).is_err());
        assert!(!report_path.exists());
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
    fn unfinished_bootstrap_recovers_suspected_plugins_after_crash() {
        let mut previous = TabbyRsState::default();
        previous.safe_mode.attempt_id = Some("attempt-1".into());
        previous.safe_mode.plugins = vec!["tabby-good".into(), "tabby-broken".into()];
        previous.safe_mode.last_started_plugin = Some("tabby-broken".into());

        assert_eq!(
            safe_mode_suspected_plugins(&previous),
            vec!["tabby-broken", "tabby-good"]
        );
    }

    #[test]
    fn persisted_crash_simulation_enters_safe_mode_for_every_bootstrap_phase() {
        for phase in ["discover", "read", "evaluate", "angular-bootstrap"] {
            let temp = tempdir().unwrap();
            let state_path = temp.path().join("tabby-rs.json");
            let mut crashed = TabbyRsState::default();
            crashed.safe_mode.attempt_id = Some(format!("attempt-{phase}"));
            crashed.safe_mode.plugins = vec!["tabby-broken".into(), "tabby-good".into()];
            crashed.safe_mode.last_started_plugin = Some("tabby-broken".into());
            crashed.safe_mode.failure_phase = Some(phase.into());
            crashed.safe_mode.failure_message = Some(format!("simulated {phase} crash"));
            save_state(&state_path, &crashed).unwrap();

            let restarted = load_state(&state_path).unwrap();
            let mode = bootstrap_mode(&restarted, Ok(vec!["tabby-good".into()]));

            assert_eq!(mode.0, true, "phase {phase} did not force safe mode");
            assert_eq!(mode.1, Some(format!("simulated {phase} crash")));
            assert!(mode.2.is_empty(), "phase {phase} loaded user plugins");
        }
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
