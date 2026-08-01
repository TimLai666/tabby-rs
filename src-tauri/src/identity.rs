use std::{env, fs, path::{Path, PathBuf}};

use tauri::Manager;

use crate::error::AppError;

pub const PRODUCT_NAME: &str = "Tabby RS";
pub const APP_IDENTIFIER: &str = "io.tabbyrs.app";
pub const CLI_NAME: &str = "tabby-rs";
pub const URL_SCHEME: &str = "tabby-rs";
pub const DATA_DIR_NAME: &str = "tabby-rs";
pub const CREDENTIAL_SERVICE: &str = "tabby-rs";
pub const PORTABLE_MARKER: &str = ".tabby-rs-portable";

#[cfg(windows)]
const MANAGED_WINDOWS_ALIAS: &str =
    "@echo off\r\nREM Tabby RS managed alias\r\n\"%~dp0tabby-rs.exe\" %*\r\n";

#[derive(Debug, Clone)]
pub struct AppPaths {
    executable: PathBuf,
    data_dir: PathBuf,
    plugins_dir: PathBuf,
    logs_dir: PathBuf,
    portable_root: Option<PathBuf>,
}

impl AppPaths {
    pub fn detect(app: &tauri::AppHandle) -> Result<Self, AppError> {
        let executable = env::current_exe()?;
        let executable_dir = executable.parent().ok_or_else(|| {
            AppError::InvalidArgument("the executable path has no parent directory".into())
        })?;
        let marker = executable_dir.join(PORTABLE_MARKER);
        let legacy_data_dir = executable_dir.join("data");
        let portable_root = if marker.is_file() || legacy_data_dir.is_dir() {
            Some(executable_dir.to_path_buf())
        } else {
            None
        };
        let data_dir = match portable_root.as_ref() {
            Some(root) => root.join("data"),
            None => app
                .path()
                .app_data_dir()
                .map_err(|error| AppError::Io(error.to_string()))?,
        };

        Ok(Self {
            executable,
            plugins_dir: data_dir.join("plugins"),
            logs_dir: data_dir.join("logs"),
            data_dir,
            portable_root,
        })
    }

    pub fn executable(&self) -> &PathBuf {
        &self.executable
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn plugins_dir(&self) -> &PathBuf {
        &self.plugins_dir
    }

    pub fn logs_dir(&self) -> &PathBuf {
        &self.logs_dir
    }

    pub fn portable_root(&self) -> Option<&PathBuf> {
        self.portable_root.as_ref()
    }

    pub fn identity(&self) -> AppIdentity {
        AppIdentity {
            product_name: PRODUCT_NAME.into(),
            app_identifier: APP_IDENTIFIER.into(),
            cli_name: CLI_NAME.into(),
            url_scheme: URL_SCHEME.into(),
            data_dir_name: DATA_DIR_NAME.into(),
            credential_service: CREDENTIAL_SERVICE.into(),
            executable: self.executable.to_string_lossy().into_owned(),
            data_dir: self.data_dir.to_string_lossy().into_owned(),
            plugins_dir: self.plugins_dir.to_string_lossy().into_owned(),
            logs_dir: self.logs_dir.to_string_lossy().into_owned(),
            portable: self.portable_root.is_some(),
            portable_root: self
                .portable_root
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }

    pub fn alias_status(&self) -> CliAliasStatus {
        let Some(alias_path) = self.alias_path_in_current_path_directory() else {
            return CliAliasStatus {
                supported: false,
                enabled: false,
                alias_path: None,
                conflict: find_path_alias_conflict(None)
                    .map(|path| path.to_string_lossy().into_owned()),
                message: Some(
                    "The Tabby RS executable directory is not on PATH; an alias would not be discoverable."
                        .into(),
                ),
            };
        };

        let owned = is_managed_alias(&alias_path, &self.executable);
        if alias_path.exists() && !owned {
            return CliAliasStatus {
                supported: true,
                enabled: false,
                alias_path: Some(alias_path.to_string_lossy().into_owned()),
                conflict: Some(alias_path.to_string_lossy().into_owned()),
                message: Some("An existing tabby command is not managed by Tabby RS.".into()),
            };
        }

        if let Some(conflict) = find_path_alias_conflict(Some(&alias_path)) {
            return CliAliasStatus {
                supported: true,
                enabled: owned,
                alias_path: Some(alias_path.to_string_lossy().into_owned()),
                conflict: Some(conflict.to_string_lossy().into_owned()),
                message: Some("Another tabby command already exists on PATH.".into()),
            };
        }

        CliAliasStatus {
            supported: true,
            enabled: owned,
            alias_path: Some(alias_path.to_string_lossy().into_owned()),
            conflict: None,
            message: None,
        }
    }

    pub fn set_alias_enabled(&self, enabled: bool) -> Result<CliAliasStatus, AppError> {
        let status = self.alias_status();
        if !status.supported {
            return Err(AppError::Unsupported(
                status
                    .message
                    .unwrap_or_else(|| "the tabby alias is unavailable".into()),
            ));
        }
        if let Some(conflict) = status.conflict {
            return Err(AppError::InvalidArgument(format!(
                "the tabby alias conflicts with {conflict}"
            )));
        }

        let alias_path = status
            .alias_path
            .map(PathBuf::from)
            .ok_or_else(|| AppError::Unsupported("the alias path is unavailable".into()))?;

        if enabled {
            if !status.enabled {
                create_managed_alias(&alias_path, &self.executable)?;
            }
        } else if alias_path.exists() {
            if !is_managed_alias(&alias_path, &self.executable) {
                return Err(AppError::InvalidArgument(
                    "refusing to remove an alias not managed by Tabby RS".into(),
                ));
            }
            fs::remove_file(&alias_path)?;
        }

        Ok(self.alias_status())
    }

    fn alias_path_in_current_path_directory(&self) -> Option<PathBuf> {
        let executable_dir = self.executable.parent()?;
        let path = env::var_os("PATH")?;
        let matching_dir = env::split_paths(&path).find(|directory| same_directory(directory, executable_dir))?;

        #[cfg(windows)]
        return Some(matching_dir.join("tabby.cmd"));

        #[cfg(not(windows))]
        return Some(matching_dir.join("tabby"));
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIdentity {
    pub product_name: String,
    pub app_identifier: String,
    pub cli_name: String,
    pub url_scheme: String,
    pub data_dir_name: String,
    pub credential_service: String,
    pub executable: String,
    pub data_dir: String,
    pub plugins_dir: String,
    pub logs_dir: String,
    pub portable: bool,
    pub portable_root: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliAliasStatus {
    pub supported: bool,
    pub enabled: bool,
    pub alias_path: Option<String>,
    pub conflict: Option<String>,
    pub message: Option<String>,
}

fn same_directory(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());

    #[cfg(windows)]
    return left
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy());

    #[cfg(not(windows))]
    return left == right;
}

fn find_path_alias_conflict(owned_alias: Option<&Path>) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        for candidate in alias_candidates(&directory) {
            if !candidate.exists() {
                continue;
            }
            if owned_alias.is_some_and(|owned| same_path(&candidate, owned)) {
                continue;
            }
            return Some(candidate);
        }
    }
    None
}

fn alias_candidates(directory: &Path) -> Vec<PathBuf> {
    #[cfg(windows)]
    return ["tabby.com", "tabby.exe", "tabby.bat", "tabby.cmd"]
        .into_iter()
        .map(|name| directory.join(name))
        .collect();

    #[cfg(not(windows))]
    return vec![directory.join("tabby")];
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());

    #[cfg(windows)]
    return left
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy());

    #[cfg(not(windows))]
    return left == right;
}

#[cfg(unix)]
fn create_managed_alias(alias: &Path, executable: &Path) -> Result<(), AppError> {
    std::os::unix::fs::symlink(executable, alias)?;
    Ok(())
}

#[cfg(windows)]
fn create_managed_alias(alias: &Path, _executable: &Path) -> Result<(), AppError> {
    fs::write(alias, MANAGED_WINDOWS_ALIAS)?;
    Ok(())
}

#[cfg(unix)]
fn is_managed_alias(alias: &Path, executable: &Path) -> bool {
    let Ok(target) = fs::read_link(alias) else {
        return false;
    };
    let target = if target.is_absolute() {
        target
    } else {
        alias.parent().unwrap_or_else(|| Path::new(".")).join(target)
    };
    same_path(&target, executable)
}

#[cfg(windows)]
fn is_managed_alias(alias: &Path, _executable: &Path) -> bool {
    fs::read_to_string(alias)
        .map(|content| content == MANAGED_WINDOWS_ALIAS)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{APP_IDENTIFIER, CLI_NAME, CREDENTIAL_SERVICE, DATA_DIR_NAME, URL_SCHEME};

    #[test]
    fn identity_is_separate_from_upstream_tabby() {
        assert_eq!(APP_IDENTIFIER, "io.tabbyrs.app");
        assert_eq!(CLI_NAME, "tabby-rs");
        assert_eq!(URL_SCHEME, "tabby-rs");
        assert_eq!(DATA_DIR_NAME, "tabby-rs");
        assert_eq!(CREDENTIAL_SERVICE, "tabby-rs");
    }

    #[cfg(windows)]
    #[test]
    fn windows_alias_does_not_reparse_arguments() {
        assert!(super::MANAGED_WINDOWS_ALIAS.contains("%*"));
        assert!(!super::MANAGED_WINDOWS_ALIAS.contains("cmd /c"));
    }
}
