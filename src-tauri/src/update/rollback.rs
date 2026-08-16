use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sysinfo::{Pid, System};

use crate::{
    error::AppError,
    storage::{
        atomic_file::{atomic_write, read_optional_regular_file},
        backup::restore_backup,
        paths::StoragePaths,
        state_file::{load_state, save_state, PendingUpdateState, TabbyRsState},
    },
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateJournal {
    pub target_version: String,
    pub backup_id: String,
    pub channel: crate::storage::state_file::UpdateChannel,
    #[serde(default)]
    pub binary_rollback: Option<BinaryRollbackJournal>,
}

const HELPER_ARG: &str = "--tabby-rs-update-helper";
const TARGET_ARG: &str = "--tabby-rs-update-target";
const BACKUP_ARG: &str = "--tabby-rs-update-backup";
const SUCCESS_ARG: &str = "--tabby-rs-update-success";
const FAILURE_ARG: &str = "--tabby-rs-update-failure";
const PARENT_ARG: &str = "--tabby-rs-update-parent";
const DEADLINE_ARG: &str = "--tabby-rs-update-deadline-ms";
const HELPER_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BinaryRollbackJournal {
    pub target_path: String,
    pub backup_path: String,
    pub success_marker: String,
    pub failure_marker: String,
    pub parent_pid: u32,
    pub deadline_ms: u64,
}

pub struct BinaryRollbackHandle {
    journal: BinaryRollbackJournal,
    child: Option<Child>,
}

impl BinaryRollbackHandle {
    pub fn journal(&self) -> &BinaryRollbackJournal {
        &self.journal
    }

    pub fn abort(mut self) {
        stop_helper(&mut self.child);
        cleanup_rollback_artifacts(&self.journal);
    }

    pub fn fail(mut self) -> Result<(), AppError> {
        atomic_write(Path::new(&self.journal.failure_marker), b"failed\n")?;
        match restore_binary_backup(&self.journal) {
            Ok(()) => {
                stop_helper(&mut self.child);
                cleanup_rollback_artifacts(&self.journal);
                Ok(())
            }
            Err(error) => {
                // Keep the helper alive. It will retry after the application exits,
                // when platform installers have released any file locks.
                self.child.take();
                Err(error)
            }
        }
    }
}

pub fn prepare_binary_rollback(
    paths: &StoragePaths,
    executable: &Path,
    backup_id: &str,
) -> Result<BinaryRollbackHandle, AppError> {
    paths.ensure_layout()?;
    let target = installation_target(executable);
    let target_metadata = fs::symlink_metadata(&target)?;
    if target_metadata.file_type().is_symlink() {
        return Err(AppError::PermissionDenied(
            "refusing to update through a symbolic-link installation target".into(),
        ));
    }

    let staging_dir = rollback_staging_dir(paths, &target);
    fs::create_dir_all(&staging_dir)?;
    let staging_metadata = fs::symlink_metadata(&staging_dir)?;
    if staging_metadata.file_type().is_symlink() || !staging_metadata.is_dir() {
        return Err(AppError::PermissionDenied(
            "refusing to use an unsafe update rollback staging directory".into(),
        ));
    }
    let prefix = format!("tabby-rs-update-{backup_id}");
    let backup_path = staging_dir.join(format!("{prefix}-binary"));
    let success_marker = staging_dir.join(format!("{prefix}-success"));
    let failure_marker = staging_dir.join(format!("{prefix}-failure"));

    if let Err(error) = copy_path(&target, &backup_path) {
        let _ = remove_path(&backup_path);
        return Err(error);
    }
    let journal = BinaryRollbackJournal {
        target_path: target.to_string_lossy().into_owned(),
        backup_path: backup_path.to_string_lossy().into_owned(),
        success_marker: success_marker.to_string_lossy().into_owned(),
        failure_marker: failure_marker.to_string_lossy().into_owned(),
        parent_pid: std::process::id(),
        deadline_ms: HELPER_TIMEOUT.as_millis() as u64,
    };

    let helper_executable = match helper_executable(executable, &staging_dir, backup_id) {
        Ok(path) => path,
        Err(error) => {
            let _ = remove_path(&backup_path);
            return Err(error);
        }
    };
    let child = Command::new(&helper_executable)
        .arg(HELPER_ARG)
        .arg(TARGET_ARG)
        .arg(&journal.target_path)
        .arg(BACKUP_ARG)
        .arg(&journal.backup_path)
        .arg(SUCCESS_ARG)
        .arg(&journal.success_marker)
        .arg(FAILURE_ARG)
        .arg(&journal.failure_marker)
        .arg(PARENT_ARG)
        .arg(journal.parent_pid.to_string())
        .arg(DEADLINE_ARG)
        .arg(journal.deadline_ms.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match child {
        Ok(child) => Ok(BinaryRollbackHandle {
            journal,
            child: Some(child),
        }),
        Err(error) => {
            let _ = remove_path(&backup_path);
            let _ = remove_path(&helper_executable);
            Err(error.into())
        }
    }
}

pub fn write_pending_update_journal(
    paths: &StoragePaths,
    pending: &PendingUpdateState,
) -> Result<(), AppError> {
    write_pending_update_journal_with_binary(paths, pending, None)
}

pub fn write_pending_update_journal_with_binary(
    paths: &StoragePaths,
    pending: &PendingUpdateState,
    binary_rollback: Option<BinaryRollbackJournal>,
) -> Result<(), AppError> {
    let journal = PendingUpdateJournal {
        target_version: pending.target_version.clone(),
        backup_id: pending.backup_id.clone(),
        channel: pending.channel.clone(),
        binary_rollback,
    };
    let mut bytes = serde_json::to_vec_pretty(&journal)?;
    bytes.push(b'\n');
    atomic_write(paths.pending_update_file(), &bytes)
}

pub fn mark_update_startup_success(
    paths: &StoragePaths,
    current_version: &str,
) -> Result<(), AppError> {
    let Some(bytes) = read_optional_regular_file(paths.pending_update_file())? else {
        return Ok(());
    };
    let journal: PendingUpdateJournal = serde_json::from_slice(&bytes)?;
    if journal.target_version != current_version {
        return Ok(());
    }
    let Some(binary) = journal.binary_rollback else {
        return Ok(());
    };
    validate_helper_paths(&binary)?;
    let marker = Path::new(&binary.success_marker);
    if !is_managed_marker(paths, marker, "-success") {
        return Err(AppError::PermissionDenied(
            "refusing to write an unmanaged update success marker".into(),
        ));
    }
    atomic_write(marker, b"ready\n")
}

pub fn maybe_run_update_rollback_helper() -> bool {
    let args = std::env::args_os().collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == HELPER_ARG) {
        return false;
    }
    if let Err(error) = run_update_rollback_helper(&args) {
        eprintln!("Tabby RS update rollback helper failed: {error}");
        std::process::exit(1);
    }
    true
}

fn run_update_rollback_helper(args: &[OsString]) -> Result<(), AppError> {
    let target = required_arg(args, TARGET_ARG)?;
    let backup = required_arg(args, BACKUP_ARG)?;
    let success = required_arg(args, SUCCESS_ARG)?;
    let failure = required_arg(args, FAILURE_ARG)?;
    let parent_pid = required_arg(args, PARENT_ARG)?
        .parse::<u32>()
        .map_err(|_| AppError::InvalidArgument("invalid update helper parent pid".into()))?;
    let deadline_ms = required_arg(args, DEADLINE_ARG)?
        .parse::<u64>()
        .map_err(|_| AppError::InvalidArgument("invalid update helper deadline".into()))?;
    let journal = BinaryRollbackJournal {
        target_path: target,
        backup_path: backup,
        success_marker: success,
        failure_marker: failure,
        parent_pid,
        deadline_ms,
    };
    validate_helper_paths(&journal)?;

    let deadline = Instant::now() + Duration::from_millis(journal.deadline_ms);
    loop {
        if marker_matches(Path::new(&journal.success_marker), &[b"ready\n"]) {
            cleanup_rollback_artifacts(&journal);
            return Ok(());
        }
        let parent_alive = process_exists(journal.parent_pid);
        if !parent_alive
            && marker_matches(
                Path::new(&journal.failure_marker),
                &[b"failed\n", b"startup-incompatible\n"],
            )
        {
            restore_binary_backup(&journal)?;
            cleanup_rollback_artifacts(&journal);
            return Ok(());
        }
        if !parent_alive && Instant::now() >= deadline {
            restore_binary_backup(&journal)?;
            cleanup_rollback_artifacts(&journal);
            return Ok(());
        }
        // Do not touch a live installation. Once the deadline passes, wait for
        // the parent to exit, then perform the same recovery on the next iteration.
        thread::sleep(Duration::from_millis(100));
    }
}

fn required_arg(args: &[OsString], name: &str) -> Result<String, AppError> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .and_then(|pair| pair[1].to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::InvalidArgument(format!("missing update helper argument {name}")))
}

fn validate_helper_paths(journal: &BinaryRollbackJournal) -> Result<(), AppError> {
    for (label, path) in [
        ("target", &journal.target_path),
        ("backup", &journal.backup_path),
        ("success marker", &journal.success_marker),
        ("failure marker", &journal.failure_marker),
    ] {
        if !Path::new(path).is_absolute() {
            return Err(AppError::InvalidArgument(format!(
                "update helper {label} path must be absolute"
            )));
        }
    }
    if journal.success_marker == journal.failure_marker {
        return Err(AppError::InvalidArgument(
            "update helper markers must be different".into(),
        ));
    }
    if Path::new(&journal.target_path) == Path::new(&journal.backup_path) {
        return Err(AppError::InvalidArgument(
            "update helper target and backup must be different".into(),
        ));
    }
    let backup_parent = Path::new(&journal.backup_path).parent();
    if backup_parent.is_none()
        || Path::new(&journal.success_marker).parent() != backup_parent
        || Path::new(&journal.failure_marker).parent() != backup_parent
    {
        return Err(AppError::InvalidArgument(
            "update helper artifacts must share a directory".into(),
        ));
    }
    for (label, path, suffix) in [
        ("success marker", &journal.success_marker, "-success"),
        ("failure marker", &journal.failure_marker, "-failure"),
    ] {
        let Some(name) = Path::new(path).file_name().and_then(|value| value.to_str()) else {
            return Err(AppError::InvalidArgument(format!(
                "update helper {label} path must have a valid file name"
            )));
        };
        if !name.starts_with("tabby-rs-update-") || !name.ends_with(suffix) {
            return Err(AppError::InvalidArgument(format!(
                "update helper {label} has an unmanaged file name"
            )));
        }
    }
    Ok(())
}

fn installation_target(executable: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(bundle) = executable
            .ancestors()
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
        {
            return bundle.to_path_buf();
        }
    }
    #[cfg(windows)]
    if let Some(parent) = executable.parent() {
        return parent.to_path_buf();
    }
    executable.to_path_buf()
}

fn rollback_staging_dir(paths: &StoragePaths, target: &Path) -> PathBuf {
    if target.starts_with(paths.data_dir()) {
        std::env::temp_dir().join("tabby-rs-update-staging")
    } else {
        paths.update_staging_dir().to_path_buf()
    }
}

fn helper_executable(
    executable: &Path,
    staging_dir: &Path,
    backup_id: &str,
) -> Result<PathBuf, AppError> {
    #[cfg(windows)]
    {
        let helper = staging_dir.join(format!("tabby-rs-update-{backup_id}-helper.exe"));
        return match fs::copy(executable, &helper) {
            Ok(_) => Ok(helper),
            Err(error) => {
                let _ = remove_path(&helper);
                Err(error.into())
            }
        };
    }
    #[cfg(not(windows))]
    {
        let _ = staging_dir;
        let _ = backup_id;
        Ok(executable.to_path_buf())
    }
}

fn copy_path(source: &Path, target: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::PermissionDenied(
            "refusing to copy a symbolic link into update rollback storage".into(),
        ));
    }
    if metadata.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::set_permissions(target, metadata.permissions())?;
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
        fs::set_permissions(target, metadata.permissions())?;
    } else {
        return Err(AppError::InvalidData(
            "update installation target contains an unsupported file type".into(),
        ));
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), AppError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn restore_binary_backup(journal: &BinaryRollbackJournal) -> Result<(), AppError> {
    let target = Path::new(&journal.target_path);
    let backup = Path::new(&journal.backup_path);
    let displaced = target.with_extension(format!("tabby-rs-update-failed-{}", std::process::id()));
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AppError::PermissionDenied(
                "refusing to restore through a symbolic-link installation target".into(),
            ));
        }
        Ok(_) => fs::rename(target, &displaced)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match copy_path(backup, target) {
        Ok(()) => {
            let _ = remove_path(&displaced);
            Ok(())
        }
        Err(error) => {
            let _ = remove_path(target);
            let _ = fs::rename(&displaced, target);
            Err(error)
        }
    }
}

fn cleanup_rollback_artifacts(journal: &BinaryRollbackJournal) {
    let _ = remove_path(Path::new(&journal.backup_path));
    let _ = remove_path(Path::new(&journal.success_marker));
    let _ = remove_path(Path::new(&journal.failure_marker));
}

fn marker_matches(path: &Path, accepted_contents: &[&[u8]]) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    if metadata.len() > 64 {
        return false;
    }
    let Ok(contents) = fs::read(path) else {
        return false;
    };
    accepted_contents
        .iter()
        .any(|accepted| contents.as_slice() == *accepted)
}

fn stop_helper(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn process_exists(pid: u32) -> bool {
    let mut system = System::new();
    let process = Pid::from_u32(pid);
    system.refresh_process(process);
    system.process(process).is_some()
}

fn is_managed_marker(paths: &StoragePaths, marker: &Path, suffix: &str) -> bool {
    let Some(parent) = marker.parent() else {
        return false;
    };
    let Some(name) = marker.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    (parent.starts_with(paths.data_dir()) || parent.starts_with(std::env::temp_dir()))
        && name.starts_with("tabby-rs-update-")
        && name.ends_with(suffix)
}

/// Keep the last known Stable snapshot available while a Nightly-to-Stable
/// transition is being validated. The current update backup may contain
/// Nightly data even after the channel has been switched to Stable.
pub fn remember_stable_backup(state: &mut TabbyRsState, backup_id: &str) {
    if state.last_stable_backup.is_none() {
        state.last_stable_backup = Some(backup_id.to_owned());
    }
}

pub fn clear_pending_update_journal(paths: &StoragePaths) -> Result<(), AppError> {
    match std::fs::symlink_metadata(paths.pending_update_file()) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::PermissionDenied(
            "refusing to remove a symbolic link from managed storage".into(),
        )),
        Ok(metadata) if !metadata.is_file() => Err(AppError::InvalidData(
            "pending update journal is not a regular file".into(),
        )),
        Ok(_) => {
            std::fs::remove_file(paths.pending_update_file())?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn recover_pending_update_from_disk(
    paths: &StoragePaths,
    current_version: &str,
) -> Result<TabbyRsState, AppError> {
    let journal = read_optional_regular_file(paths.pending_update_file())?
        .map(|bytes| serde_json::from_slice::<PendingUpdateJournal>(&bytes))
        .transpose()?;
    let state_result = load_state(paths.state_file());

    let Some(journal) = journal else {
        return recover_pending_update(paths, state_result?, current_version);
    };

    let state_is_corrupt = state_result.is_err();
    let state = match state_result {
        Ok(state) => state,
        Err(_error) if journal.target_version == current_version => TabbyRsState::default(),
        Err(error) => return Err(error),
    };
    let startup_incompatible = journal.target_version == current_version
        && (state_is_corrupt || !config_is_readable(paths)?);
    let recovered = recover_journal(paths, state, &journal, current_version, state_is_corrupt)?;
    if journal.target_version == current_version {
        if startup_incompatible {
            if let Some(binary) = journal.binary_rollback.as_ref() {
                validate_helper_paths(binary)?;
                if !is_managed_marker(paths, Path::new(&binary.failure_marker), "-failure") {
                    return Err(AppError::PermissionDenied(
                        "refusing to write an unmanaged update failure marker".into(),
                    ));
                }
                atomic_write(Path::new(&binary.failure_marker), b"startup-incompatible\n")?;
                return Err(AppError::Io(
                    "update startup was incompatible and binary rollback is pending application exit".into(),
                ));
            }
        }
        mark_update_startup_success(paths, current_version)?;
    }
    clear_pending_update_journal(paths)?;
    Ok(recovered)
}

fn recover_journal(
    paths: &StoragePaths,
    mut state: TabbyRsState,
    journal: &PendingUpdateJournal,
    current_version: &str,
    state_is_corrupt: bool,
) -> Result<TabbyRsState, AppError> {
    if journal.target_version == current_version
        && (state_is_corrupt || !config_is_readable(paths)?)
    {
        restore_backup(paths, &journal.backup_id)?;
        state = load_state(paths.state_file())?;
        state.extra.insert(
            "lastUpdateRecovery".into(),
            serde_json::json!({
                "reason": if state_is_corrupt { "state-incompatible" } else { "config-incompatible" },
                "backupId": journal.backup_id,
                "targetVersion": journal.target_version,
            }),
        );
    }
    state.pending_update = None;
    save_state(paths.state_file(), &state)?;
    Ok(state)
}

pub fn recover_pending_update(
    paths: &StoragePaths,
    mut state: TabbyRsState,
    current_version: &str,
) -> Result<TabbyRsState, AppError> {
    let Some(pending) = state.pending_update.clone() else {
        return Ok(state);
    };

    if pending.target_version == current_version && !config_is_readable(paths)? {
        let backup_id = if matches!(
            pending.channel,
            crate::storage::state_file::UpdateChannel::Stable
        ) {
            state
                .last_stable_backup
                .clone()
                .unwrap_or_else(|| pending.backup_id.clone())
        } else {
            pending.backup_id.clone()
        };
        restore_backup(paths, &backup_id)?;
        state = load_state(paths.state_file())?;
        state.extra.insert(
            "lastUpdateRecovery".into(),
            serde_json::json!({
                "reason": "config-incompatible",
                "backupId": backup_id,
                "targetVersion": pending.target_version,
            }),
        );
        save_state(paths.state_file(), &state)?;
        return Ok(state);
    }

    state.pending_update = None;
    save_state(paths.state_file(), &state)?;
    Ok(state)
}

fn config_is_readable(paths: &StoragePaths) -> Result<bool, AppError> {
    let Some(bytes) = read_optional_regular_file(paths.config_file())? else {
        return Ok(true);
    };
    if bytes.is_empty() {
        return Ok(true);
    }
    let value: BTreeMap<String, serde_json::Value> = match serde_yaml::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(!value.is_empty() || bytes.iter().all(u8::is_ascii_whitespace))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, ffi::OsString};

    use tempfile::tempdir;

    use super::{
        clear_pending_update_journal, mark_update_startup_success, recover_pending_update,
        recover_pending_update_from_disk, remember_stable_backup, restore_binary_backup,
        run_update_rollback_helper, write_pending_update_journal,
        write_pending_update_journal_with_binary, BinaryRollbackJournal, BACKUP_ARG, DEADLINE_ARG,
        FAILURE_ARG, HELPER_ARG, PARENT_ARG, SUCCESS_ARG, TARGET_ARG,
    };
    use crate::storage::{
        atomic_file::atomic_write,
        backup::{create_backup, BackupRequest},
        paths::StoragePaths,
        state_file::{save_state, PendingUpdateState, TabbyRsState, UpdateChannel},
    };

    #[test]
    fn restores_before_update_backup_when_new_config_is_incompatible() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        let mut state = TabbyRsState::default();
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.2".into(),
            backup_id: backup.backup_id.clone(),
            channel: UpdateChannel::Stable,
        });
        state.extra = BTreeMap::new();
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
        assert_eq!(
            recovered.extra["lastUpdateRecovery"]["backupId"],
            backup.backup_id
        );
    }

    #[test]
    fn clears_stale_marker_when_old_version_remains_available() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        let mut state = TabbyRsState::default();
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.2".into(),
            backup_id: "backup".into(),
            channel: UpdateChannel::Stable,
        });
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.1").unwrap();
        assert!(recovered.pending_update.is_none());
    }

    #[test]
    fn restores_when_config_is_yaml_but_not_a_bootstrap_mapping() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"- valid-yaml-but-not-a-config-map\n").unwrap();

        let mut state = TabbyRsState::default();
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.2".into(),
            backup_id: backup.backup_id.clone(),
            channel: UpdateChannel::Stable,
        });
        state.extra = BTreeMap::new();
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
    }

    #[test]
    fn stable_recovery_prefers_last_stable_backup_over_nightly_update_backup() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"channel: stable\n").unwrap();
        let stable_backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-nightly".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();

        atomic_write(paths.config_file(), b"channel: nightly\n").unwrap();
        let nightly_backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-stable".into(),
                source_version: Some("1.0.231-tabbyrs.2.nightly.20260812.1".into()),
                channel: Some(UpdateChannel::Nightly),
            },
            "1.0.231-tabbyrs.2.nightly.20260812.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();

        let mut state = TabbyRsState::default();
        state.last_stable_backup = Some(stable_backup.backup_id.clone());
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.3".into(),
            backup_id: nightly_backup.backup_id,
            channel: UpdateChannel::Stable,
        });
        state.extra = BTreeMap::new();
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.3").unwrap();
        assert_eq!(
            std::fs::read(paths.config_file()).unwrap(),
            b"channel: stable\n"
        );
        assert_eq!(
            recovered.extra["lastUpdateRecovery"]["backupId"],
            stable_backup.backup_id
        );
    }

    #[test]
    fn stable_update_does_not_replace_last_stable_backup_with_nightly_snapshot() {
        let mut state = TabbyRsState::default();
        state.last_stable_backup = Some("stable-before-nightly".into());

        remember_stable_backup(&mut state, "nightly-before-stable");

        assert_eq!(
            state.last_stable_backup.as_deref(),
            Some("stable-before-nightly")
        );
    }

    #[test]
    fn journal_recovers_when_state_file_is_corrupt() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        atomic_write(paths.state_file(), b"{broken").unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: backup.backup_id.clone(),
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
        assert!(!paths.pending_update_file().exists());
        assert_eq!(
            recovered.extra["lastUpdateRecovery"]["reason"],
            "state-incompatible"
        );
    }

    #[test]
    fn journal_recovers_when_state_schema_is_newer() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        atomic_write(paths.state_file(), br#"{"schemaVersion":99}"#).unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: backup.backup_id,
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert!(!paths.pending_update_file().exists());
    }

    #[test]
    fn journal_is_cleared_after_successful_restart() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 2\n").unwrap();
        save_state(paths.state_file(), &TabbyRsState::default()).unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: "unused".into(),
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert!(!paths.pending_update_file().exists());
    }

    #[test]
    fn old_version_clears_install_failure_marker_without_restoring() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: old\n").unwrap();
        save_state(paths.state_file(), &TabbyRsState::default()).unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: "unused".into(),
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.1").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(
            std::fs::read(paths.config_file()).unwrap(),
            b"version: old\n"
        );
        assert!(!paths.pending_update_file().exists());
    }

    #[test]
    fn malformed_journal_is_preserved_for_diagnosis() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.pending_update_file(), b"{broken").unwrap();

        assert!(recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").is_err());
        assert!(paths.pending_update_file().exists());
    }

    #[test]
    fn startup_marks_binary_update_success_before_clearing_journal() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        let marker = paths
            .update_staging_dir()
            .join("tabby-rs-update-test-success");
        let journal = BinaryRollbackJournal {
            target_path: temp.path().join("target").display().to_string(),
            backup_path: paths
                .update_staging_dir()
                .join("tabby-rs-update-test-binary")
                .display()
                .to_string(),
            success_marker: marker.display().to_string(),
            failure_marker: paths
                .update_staging_dir()
                .join("tabby-rs-update-test-failure")
                .display()
                .to_string(),
            parent_pid: std::process::id(),
            deadline_ms: 120_000,
        };
        write_pending_update_journal_with_binary(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: "backup".into(),
                channel: UpdateChannel::Stable,
            },
            Some(journal),
        )
        .unwrap();

        mark_update_startup_success(&paths, "1.0.231-tabbyrs.2").unwrap();

        assert_eq!(std::fs::read(marker).unwrap(), b"ready\n");
    }

    #[test]
    fn incompatible_startup_keeps_journal_and_requests_binary_rollback() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        save_state(paths.state_file(), &TabbyRsState::default()).unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        let target = temp.path().join("target");
        let binary_backup = paths
            .update_staging_dir()
            .join("tabby-rs-update-test-binary");
        let failure_marker = paths
            .update_staging_dir()
            .join("tabby-rs-update-test-failure");
        let success_marker = paths
            .update_staging_dir()
            .join("tabby-rs-update-test-success");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&binary_backup, b"old").unwrap();
        write_pending_update_journal_with_binary(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: backup.backup_id,
                channel: UpdateChannel::Stable,
            },
            Some(BinaryRollbackJournal {
                target_path: target.display().to_string(),
                backup_path: binary_backup.display().to_string(),
                success_marker: success_marker.display().to_string(),
                failure_marker: failure_marker.display().to_string(),
                parent_pid: std::process::id(),
                deadline_ms: 120_000,
            }),
        )
        .unwrap();

        assert!(recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").is_err());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
        assert!(failure_marker.exists());
        assert!(paths.pending_update_file().exists());
        assert!(!success_marker.exists());
    }

    #[test]
    fn binary_restore_replaces_a_file_and_cleans_the_displaced_version() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        let backup = temp.path().join("backup");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&backup, b"old").unwrap();
        let journal = BinaryRollbackJournal {
            target_path: target.display().to_string(),
            backup_path: backup.display().to_string(),
            success_marker: temp
                .path()
                .join("tabby-rs-update-test-success")
                .display()
                .to_string(),
            failure_marker: temp
                .path()
                .join("tabby-rs-update-test-failure")
                .display()
                .to_string(),
            parent_pid: std::process::id(),
            deadline_ms: 0,
        };

        restore_binary_backup(&journal).unwrap();

        assert_eq!(std::fs::read(target).unwrap(), b"old");
        assert!(!temp
            .path()
            .join(format!(
                "target.tabby-rs-update-failed-{}",
                std::process::id()
            ))
            .exists());
    }

    #[test]
    fn rollback_helper_restores_after_parent_exit_and_cleans_artifacts() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        let backup = temp.path().join("tabby-rs-update-test-binary");
        let success = temp.path().join("tabby-rs-update-test-success");
        let failure = temp.path().join("tabby-rs-update-test-failure");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&backup, b"old").unwrap();
        std::fs::write(&failure, b"failed\n").unwrap();
        let args = vec![
            OsString::from(HELPER_ARG),
            OsString::from(TARGET_ARG),
            target.as_os_str().to_owned(),
            OsString::from(BACKUP_ARG),
            backup.as_os_str().to_owned(),
            OsString::from(SUCCESS_ARG),
            success.as_os_str().to_owned(),
            OsString::from(FAILURE_ARG),
            failure.as_os_str().to_owned(),
            OsString::from(PARENT_ARG),
            OsString::from(u32::MAX.to_string()),
            OsString::from(DEADLINE_ARG),
            OsString::from("0"),
        ];

        run_update_rollback_helper(&args).unwrap();

        assert_eq!(std::fs::read(target).unwrap(), b"old");
        assert!(!backup.exists());
        assert!(!success.exists());
        assert!(!failure.exists());
    }

    #[test]
    fn rollback_helper_keeps_new_version_after_success_marker() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        let backup = temp.path().join("tabby-rs-update-test-binary");
        let success = temp.path().join("tabby-rs-update-test-success");
        let failure = temp.path().join("tabby-rs-update-test-failure");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&backup, b"old").unwrap();
        std::fs::write(&success, b"ready\n").unwrap();
        let args = vec![
            OsString::from(HELPER_ARG),
            OsString::from(TARGET_ARG),
            target.as_os_str().to_owned(),
            OsString::from(BACKUP_ARG),
            backup.as_os_str().to_owned(),
            OsString::from(SUCCESS_ARG),
            success.as_os_str().to_owned(),
            OsString::from(FAILURE_ARG),
            failure.as_os_str().to_owned(),
            OsString::from(PARENT_ARG),
            OsString::from(u32::MAX.to_string()),
            OsString::from(DEADLINE_ARG),
            OsString::from("0"),
        ];

        run_update_rollback_helper(&args).unwrap();

        assert_eq!(std::fs::read(target).unwrap(), b"new");
        assert!(!backup.exists());
        assert!(!success.exists());
        assert!(!failure.exists());
    }

    #[test]
    fn helper_rejects_unmanaged_marker_names() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        let backup = temp.path().join("tabby-rs-update-test-binary");
        let journal = BinaryRollbackJournal {
            target_path: target.display().to_string(),
            backup_path: backup.display().to_string(),
            success_marker: temp.path().join("success").display().to_string(),
            failure_marker: temp
                .path()
                .join("tabby-rs-update-test-failure")
                .display()
                .to_string(),
            parent_pid: std::process::id(),
            deadline_ms: 0,
        };
        let args = vec![
            OsString::from(HELPER_ARG),
            OsString::from(TARGET_ARG),
            OsString::from(&journal.target_path),
            OsString::from(BACKUP_ARG),
            OsString::from(&journal.backup_path),
            OsString::from(SUCCESS_ARG),
            OsString::from(&journal.success_marker),
            OsString::from(FAILURE_ARG),
            OsString::from(&journal.failure_marker),
            OsString::from(PARENT_ARG),
            OsString::from(journal.parent_pid.to_string()),
            OsString::from(DEADLINE_ARG),
            OsString::from(journal.deadline_ms.to_string()),
        ];

        assert!(run_update_rollback_helper(&args).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_remove_a_journal_symlink() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, paths.pending_update_file()).unwrap();

        assert!(clear_pending_update_journal(&paths).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    }
}
