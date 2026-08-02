use std::{collections::BTreeMap, env, path::Path};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::AppError;

use super::model::{PrepareSpawnRequest, PreparedSpawnRequest};

const MAX_ENVIRONMENT_ENTRIES: usize = 4096;
const MAX_ARGUMENTS: usize = 4096;
const MAX_VALUE_BYTES: usize = 1024 * 1024;

pub fn prepare_spawn(request: PrepareSpawnRequest) -> Result<PreparedSpawnRequest, AppError> {
    validate_text("shell command", &request.command, false)?;
    if request.args.len() > MAX_ARGUMENTS {
        return Err(AppError::InvalidArgument(
            "shell argument list is too large".into(),
        ));
    }
    for argument in &request.args {
        validate_text("shell argument", argument, true)?;
    }

    let environment = merge_environment(request.profile_environment, request.runtime_environment)?;
    let executable = resolve_executable(&request.command, &environment)?;
    let (cwd, cwd_fallback) = validate_cwd(request.cwd)?;

    Ok(PreparedSpawnRequest {
        executable,
        arguments: request.args,
        cwd,
        environment,
        shell_type: request.shell_type,
        login_shell: request.login_shell,
        cwd_fallback,
    })
}

pub fn merge_environment(
    profile: BTreeMap<String, String>,
    runtime: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AppError> {
    let mut merged = BTreeMap::new();
    for (key, value) in env::vars_os() {
        let key = key.into_string().map_err(|_| {
            AppError::InvalidData("system environment contains a non-Unicode key".into())
        })?;
        let value = value.into_string().map_err(|_| {
            AppError::InvalidData("system environment contains a non-Unicode value".into())
        })?;
        validate_environment_entry(&key, &value)?;
        insert_environment(&mut merged, key, value);
    }

    for (key, value) in profile.into_iter().chain(runtime) {
        validate_environment_entry(&key, &value)?;
        insert_environment(&mut merged, key, value);
    }
    if merged.len() > MAX_ENVIRONMENT_ENTRIES {
        return Err(AppError::InvalidArgument(
            "shell environment contains too many entries".into(),
        ));
    }
    Ok(merged)
}

fn insert_environment(environment: &mut BTreeMap<String, String>, key: String, value: String) {
    #[cfg(windows)]
    if let Some(existing) = environment
        .keys()
        .find(|candidate| candidate.eq_ignore_ascii_case(&key))
        .cloned()
    {
        environment.insert(existing, value);
        return;
    }

    environment.insert(key, value);
}

fn environment_value<'a>(environment: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    #[cfg(windows)]
    {
        environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[cfg(not(windows))]
    {
        environment.get(name).map(String::as_str)
    }
}

fn validate_environment_entry(key: &str, value: &str) -> Result<(), AppError> {
    if key.is_empty() || key.contains('=') || key.contains('\0') || key.len() > 32 * 1024 {
        return Err(AppError::InvalidArgument(
            "shell environment contains an invalid key".into(),
        ));
    }
    validate_text("shell environment value", value, true)
}

fn validate_text(label: &str, value: &str, allow_empty: bool) -> Result<(), AppError> {
    if (!allow_empty && value.trim().is_empty())
        || value.contains('\0')
        || value.len() > MAX_VALUE_BYTES
    {
        return Err(AppError::InvalidArgument(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_cwd(cwd: Option<String>) -> Result<(Option<String>, bool), AppError> {
    let Some(cwd) = cwd else {
        return Ok((None, false));
    };
    validate_text("shell working directory", &cwd, false)?;
    if Path::new(&cwd).is_dir() {
        return Ok((Some(cwd), false));
    }
    Ok((None, true))
}

fn resolve_executable(
    command: &str,
    environment: &BTreeMap<String, String>,
) -> Result<String, AppError> {
    let path = Path::new(command);
    if path.is_absolute() || command.contains('/') || command.contains('\\') {
        return executable_string(path)
            .ok_or_else(|| AppError::InvalidArgument("shell executable was not found".into()));
    }

    let path_value = environment_value(environment, "PATH").unwrap_or_default();

    #[cfg(windows)]
    let extensions = executable_extensions(command, environment);
    #[cfg(not(windows))]
    let extensions = vec![String::new()];

    for directory in env::split_paths(path_value) {
        for extension in &extensions {
            let candidate = directory.join(format!("{command}{extension}"));
            if let Some(found) = executable_string(&candidate) {
                return Ok(found);
            }
        }
    }

    Err(AppError::InvalidArgument(
        "shell executable was not found".into(),
    ))
}

fn executable_string(path: &Path) -> Option<String> {
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return None;
    }
    path.to_str().map(str::to_owned)
}

#[cfg(windows)]
fn executable_extensions(command: &str, environment: &BTreeMap<String, String>) -> Vec<String> {
    if Path::new(command).extension().is_some() {
        return vec![String::new()];
    }
    environment_value(environment, "PATHEXT")
        .map(|value| {
            value
                .split(';')
                .filter(|part| !part.is_empty())
                .map(|part| part.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| vec![".com".into(), ".exe".into(), ".bat".into(), ".cmd".into()])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt};

    use super::{executable_string, insert_environment, merge_environment, validate_cwd};

    #[test]
    fn runtime_environment_wins_over_profile_environment() {
        let mut profile = BTreeMap::new();
        profile.insert("TABBY_RS_ENV_TEST".into(), "profile".into());
        let mut runtime = BTreeMap::new();
        runtime.insert("TABBY_RS_ENV_TEST".into(), "runtime".into());
        let merged = merge_environment(profile, runtime).unwrap();
        assert_eq!(merged["TABBY_RS_ENV_TEST"], "runtime");
    }

    #[cfg(windows)]
    #[test]
    fn windows_environment_keys_are_case_insensitive() {
        let mut environment = BTreeMap::from([("Path".into(), "base".into())]);
        insert_environment(&mut environment, "PATH".into(), "override".into());
        assert_eq!(environment.len(), 1);
        assert_eq!(environment["Path"], "override");
    }

    #[test]
    fn missing_working_directory_falls_back_without_panicking() {
        let (cwd, fallback) =
            validate_cwd(Some("/definitely/not/a/real/tabby-rs-directory".into())).unwrap();
        assert!(cwd.is_none());
        assert!(fallback);
    }

    #[cfg(unix)]
    #[test]
    fn unix_executables_require_an_execute_bit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("not-executable");
        fs::write(&path, "fixture").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(executable_string(&path).is_none());
    }
}
