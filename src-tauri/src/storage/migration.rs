use std::{
    collections::{BTreeSet, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde_yaml::Value;

use crate::error::AppError;

use super::{
    atomic_file::{atomic_write, read_optional_regular_file, sha256_hex},
    backup::{create_backup, restore_backup, BackupRequest},
    paths::StoragePaths,
    state_file::{load_state, save_state, FirstRunImportState},
};

const MAX_SOURCE_CONFIG_BYTES: usize = 16 * 1024 * 1024;
const MIGRATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretReference {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub source_data_dir: String,
    pub config: bool,
    pub profiles: usize,
    pub plugins: Vec<String>,
    pub secret_references: Vec<SecretReference>,
    pub source_revision: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSelection {
    pub source_data_dir: String,
    pub config: bool,
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReportItem {
    pub kind: String,
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: Vec<ImportReportItem>,
    pub skipped: Vec<ImportReportItem>,
    pub failed: Vec<ImportReportItem>,
    pub requires_secret_reentry: Vec<String>,
    pub report_path: String,
    pub backup_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MigrationJournal {
    schema_version: u32,
    source_data_dir: String,
    started_at: DateTime<Utc>,
    steps: Vec<JournalStep>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct JournalStep {
    name: String,
    status: String,
    at: DateTime<Utc>,
    detail: String,
}

pub fn detect_import_plans(paths: &StoragePaths) -> Result<Vec<ImportPlan>, AppError> {
    detect_import_plans_from_candidates(paths, candidate_data_dirs())
}

pub fn execute_import(
    paths: &StoragePaths,
    selection: &ImportSelection,
    app_version: &str,
) -> Result<ImportReport, AppError> {
    let plans = detect_import_plans(paths)?;
    execute_import_with_plans(paths, selection, app_version, plans)
}

fn execute_import_with_plans(
    paths: &StoragePaths,
    selection: &ImportSelection,
    app_version: &str,
    plans: Vec<ImportPlan>,
) -> Result<ImportReport, AppError> {
    paths.ensure_layout()?;
    let selected_source = fs::canonicalize(&selection.source_data_dir)
        .map_err(|_| AppError::InvalidArgument("import source is unavailable".into()))?;
    let selected_source_text = selected_source.to_string_lossy();
    let plan = plans
        .iter()
        .find(|plan| plan.source_data_dir == selected_source_text)
        .ok_or_else(|| AppError::PermissionDenied("import source was not detected".into()))?;

    let allowed_plugins = plan.plugins.iter().cloned().collect::<HashSet<_>>();
    let mut selected_plugins = BTreeSet::new();
    for plugin in &selection.plugins {
        if !allowed_plugins.contains(plugin) {
            return Err(AppError::InvalidArgument(format!(
                "plugin is not present in the import plan: {plugin}"
            )));
        }
        selected_plugins.insert(plugin.clone());
    }
    if !selection.config && selected_plugins.is_empty() {
        return Err(AppError::InvalidArgument(
            "at least one import item must be selected".into(),
        ));
    }

    let backup = create_backup(
        paths,
        &BackupRequest {
            reason: "before-first-import".into(),
            source_version: Some(app_version.into()),
            channel: None,
        },
        app_version,
    )?;
    let started_at = Utc::now();
    let journal_name = format!("import-{}-journal.json", started_at.timestamp_millis());
    let report_name = format!("import-{}-report.json", started_at.timestamp_millis());
    let journal_path = paths.migration_file(&journal_name)?;
    let report_path = paths.migration_file(&report_name)?;
    let mut journal = MigrationJournal {
        schema_version: MIGRATION_SCHEMA_VERSION,
        source_data_dir: plan.source_data_dir.clone(),
        started_at,
        steps: Vec::new(),
    };
    push_journal_step(
        &journal_path,
        &mut journal,
        "plan-validated",
        "completed",
        "The selected source and plugin list match the detected plan.",
    )?;

    let mut state = load_state(paths.state_file())?;
    state.first_run_import = FirstRunImportState::Running {
        started_at,
        journal: journal_path.to_string_lossy().into_owned(),
    };
    save_state(paths.state_file(), &state)?;

    let result = (|| {
        let mut imported = Vec::new();
        let mut skipped = Vec::new();
        let source_config = selected_source.join("config.yaml");

        if selection.config {
            let source_before = read_optional_regular_file(&source_config)?
                .ok_or_else(|| AppError::NotFound("source config.yaml".into()))?;
            if source_before.len() > MAX_SOURCE_CONFIG_BYTES {
                return Err(AppError::InvalidData("source config.yaml is too large".into()));
            }
            if sha256_hex(&source_before) != plan.source_revision {
                return Err(AppError::Conflict(
                    "source config.yaml changed after the import plan was shown".into(),
                ));
            }
            atomic_write(paths.config_file(), &source_before)?;
            let source_after = read_optional_regular_file(&source_config)?
                .ok_or_else(|| AppError::NotFound("source config.yaml".into()))?;
            if sha256_hex(&source_after) != plan.source_revision {
                return Err(AppError::Conflict(
                    "source config.yaml changed during import".into(),
                ));
            }
            imported.push(ImportReportItem {
                kind: "config".into(),
                name: "config.yaml".into(),
                detail: format!("Imported {} bytes without rewriting YAML.", source_before.len()),
            });
            push_journal_step(
                &journal_path,
                &mut journal,
                "config-copied",
                "completed",
                "config.yaml was copied byte-for-byte and the source checksum stayed unchanged.",
            )?;
        } else {
            skipped.push(ImportReportItem {
                kind: "config".into(),
                name: "config.yaml".into(),
                detail: "Not selected by the user.".into(),
            });
        }

        for plugin in &plan.plugins {
            if selected_plugins.contains(plugin) {
                imported.push(ImportReportItem {
                    kind: "plugin".into(),
                    name: plugin.clone(),
                    detail: "Added to the pending plugin installation list; plugin files were not copied."
                        .into(),
                });
            } else {
                skipped.push(ImportReportItem {
                    kind: "plugin".into(),
                    name: plugin.clone(),
                    detail: "Not selected by the user.".into(),
                });
            }
        }

        let requires_secret_reentry = if selection.config {
            plan.secret_references
                .iter()
                .map(|reference| reference.path.clone())
                .collect()
        } else {
            Vec::new()
        };
        Ok((imported, skipped, requires_secret_reentry))
    })();

    match result {
        Ok((imported, skipped, requires_secret_reentry)) => {
            let mut report = ImportReport {
                imported,
                skipped,
                failed: Vec::new(),
                requires_secret_reentry,
                report_path: report_path.to_string_lossy().into_owned(),
                backup_id: backup.backup_id,
            };
            write_json(&report_path, &report)?;
            push_journal_step(
                &journal_path,
                &mut journal,
                "report-written",
                "completed",
                "The import report was written before marking the migration complete.",
            )?;

            let mut state = load_state(paths.state_file())?;
            state.pending_plugins = selected_plugins.into_iter().collect();
            state.first_run_import = FirstRunImportState::Completed {
                completed_at: Utc::now(),
                report: report.report_path.clone(),
            };
            save_state(paths.state_file(), &state)?;
            report.backup_id = backup.backup_id;
            Ok(report)
        }
        Err(error) => {
            let failure_detail = error.to_string();
            let failure_report = ImportReport {
                imported: Vec::new(),
                skipped: Vec::new(),
                failed: vec![ImportReportItem {
                    kind: "migration".into(),
                    name: "first-run-import".into(),
                    detail: failure_detail.clone(),
                }],
                requires_secret_reentry: Vec::new(),
                report_path: report_path.to_string_lossy().into_owned(),
                backup_id: backup.backup_id.clone(),
            };
            let _ = write_json(&report_path, &failure_report);
            let _ = push_journal_step(
                &journal_path,
                &mut journal,
                "rollback",
                "failed",
                &failure_detail,
            );
            let _ = restore_backup(paths, &backup.backup_id);
            let mut state = load_state(paths.state_file()).unwrap_or_default();
            state.first_run_import = FirstRunImportState::Failed {
                failed_at: Utc::now(),
                report: report_path.to_string_lossy().into_owned(),
            };
            let _ = save_state(paths.state_file(), &state);
            Err(error)
        }
    }
}

fn detect_import_plans_from_candidates(
    paths: &StoragePaths,
    candidates: Vec<PathBuf>,
) -> Result<Vec<ImportPlan>, AppError> {
    let target = fs::canonicalize(paths.data_dir()).unwrap_or_else(|_| paths.data_dir().into());
    let mut seen = HashSet::new();
    let mut plans = Vec::new();
    for candidate in candidates {
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(candidate) = fs::canonicalize(candidate) else {
            continue;
        };
        if candidate == target || !seen.insert(candidate.clone()) {
            continue;
        }
        let config_path = candidate.join("config.yaml");
        let Some(config_bytes) = (match read_optional_regular_file(&config_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        }) else {
            continue;
        };
        if config_bytes.len() > MAX_SOURCE_CONFIG_BYTES {
            continue;
        }
        let yaml = String::from_utf8_lossy(&config_bytes);
        let (profiles, secret_references) = inspect_config(&yaml);
        plans.push(ImportPlan {
            source_data_dir: candidate.to_string_lossy().into_owned(),
            config: true,
            profiles,
            plugins: detect_plugins(&candidate.join("plugins"))?,
            secret_references,
            source_revision: sha256_hex(&config_bytes),
        });
    }
    plans.sort_by(|left, right| left.source_data_dir.cmp(&right.source_data_dir));
    Ok(plans)
}

fn candidate_data_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("TABBY_CONFIG_DIRECTORY") {
        candidates.push(path.into());
    }

    #[cfg(windows)]
    if let Some(app_data) = env::var_os("APPDATA") {
        let app_data = PathBuf::from(app_data);
        candidates.push(app_data.join("tabby"));
        candidates.push(app_data.join("Tabby"));
        candidates.push(app_data.join("terminus"));
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME") {
        let base = PathBuf::from(home).join("Library/Application Support");
        candidates.push(base.join("tabby"));
        candidates.push(base.join("Tabby"));
        candidates.push(base.join("terminus"));
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
            let base = PathBuf::from(config_home);
            candidates.push(base.join("tabby"));
            candidates.push(base.join("terminus"));
        }
        if let Some(home) = env::var_os("HOME") {
            let base = PathBuf::from(home).join(".config");
            candidates.push(base.join("tabby"));
            candidates.push(base.join("terminus"));
        }
    }

    candidates
}

fn detect_plugins(plugin_dir: &Path) -> Result<Vec<String>, AppError> {
    let Ok(metadata) = fs::symlink_metadata(plugin_dir) else {
        return Ok(Vec::new());
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(Vec::new());
    }
    let mut plugins = BTreeSet::new();
    for entry in fs::read_dir(plugin_dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let fallback = entry.file_name().to_string_lossy().into_owned();
        let package_path = entry.path().join("package.json");
        let package_name = read_optional_regular_file(&package_path)?
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("name")?.as_str().map(str::to_owned))
            .unwrap_or(fallback);
        if !package_name.is_empty()
            && package_name.len() <= 214
            && !package_name.chars().any(char::is_control)
        {
            plugins.insert(package_name);
        }
    }
    Ok(plugins.into_iter().collect())
}

fn inspect_config(yaml: &str) -> (usize, Vec<SecretReference>) {
    let Ok(value) = serde_yaml::from_str::<Value>(yaml) else {
        return (0, Vec::new());
    };
    let mut profiles = 0;
    let mut secrets = Vec::new();
    inspect_value(&value, "$", &mut profiles, &mut secrets);
    secrets.sort_by(|left, right| left.path.cmp(&right.path));
    secrets.dedup_by(|left, right| left.path == right.path);
    (profiles, secrets)
}

fn inspect_value(
    value: &Value,
    path: &str,
    profiles: &mut usize,
    secrets: &mut Vec<SecretReference>,
) {
    match value {
        Value::Mapping(mapping) => {
            for (key, child) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let child_path = format!("{path}.{}", key.replace('.', "\\."));
                if key.eq_ignore_ascii_case("profiles") {
                    if let Value::Sequence(items) = child {
                        *profiles += items
                            .iter()
                            .filter(|item| matches!(item, Value::Mapping(_)))
                            .count();
                    }
                }
                if is_secret_key(key) && !matches!(child, Value::Null) {
                    secrets.push(SecretReference {
                        path: child_path.clone(),
                        kind: secret_kind(key).into(),
                    });
                }
                inspect_value(child, &child_path, profiles, secrets);
            }
        }
        Value::Sequence(items) => {
            for (index, child) in items.iter().enumerate() {
                inspect_value(child, &format!("{path}[{index}]"), profiles, secrets);
            }
        }
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    ["password", "passphrase", "privatekey", "token", "secret"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn secret_kind(key: &str) -> &'static str {
    let normalized = key.to_ascii_lowercase();
    if normalized.contains("private") && normalized.contains("key") {
        "privateKey"
    } else if normalized.contains("passphrase") {
        "passphrase"
    } else if normalized.contains("token") {
        "token"
    } else {
        "password"
    }
}

fn push_journal_step(
    path: &Path,
    journal: &mut MigrationJournal,
    name: &str,
    status: &str,
    detail: &str,
) -> Result<(), AppError> {
    journal.steps.push(JournalStep {
        name: name.into(),
        status: status.into(),
        at: Utc::now(),
        detail: detail.into(),
    });
    write_json(path, journal)
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), AppError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        detect_import_plans_from_candidates, execute_import_with_plans, ImportSelection,
    };
    use crate::storage::{paths::StoragePaths, state_file::load_state};

    #[test]
    fn detection_does_not_write_target_data() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let target = StoragePaths::from_data_dir(temp.path().join("target"));
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("config.yaml"),
            "profiles:\n  - name: shell\n    password: hidden\n",
        )
        .unwrap();
        let plans = detect_import_plans_from_candidates(&target, vec![source]).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].profiles, 1);
        assert_eq!(plans[0].secret_references.len(), 1);
        assert!(!target.data_dir().exists());
    }

    #[test]
    fn import_is_one_way_and_preserves_source_checksum() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let target = StoragePaths::from_data_dir(temp.path().join("target"));
        std::fs::create_dir_all(source.join("plugins/example")).unwrap();
        let source_yaml = "version: 1\nprofiles:\n  - name: shell\nunknown: keep\n";
        std::fs::write(source.join("config.yaml"), source_yaml).unwrap();
        std::fs::write(
            source.join("plugins/example/package.json"),
            r#"{"name":"tabby-plugin-example"}"#,
        )
        .unwrap();
        let plans = detect_import_plans_from_candidates(&target, vec![source.clone()]).unwrap();
        let source_dir = plans[0].source_data_dir.clone();
        let source_revision = plans[0].source_revision.clone();
        let report = execute_import_with_plans(
            &target,
            &ImportSelection {
                source_data_dir: source_dir,
                config: true,
                plugins: vec!["tabby-plugin-example".into()],
            },
            "test",
            plans,
        )
        .unwrap();
        assert!(report.failed.is_empty());
        assert_eq!(std::fs::read_to_string(target.config_file()).unwrap(), source_yaml);
        assert_eq!(
            crate::storage::atomic_file::file_revision(&source.join("config.yaml"))
                .unwrap()
                .unwrap(),
            source_revision
        );
        let state = load_state(target.state_file()).unwrap();
        assert_eq!(state.pending_plugins, ["tabby-plugin-example"]);
    }
}
