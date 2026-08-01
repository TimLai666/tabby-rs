use std::{env, path::PathBuf};

use tauri::Manager;

use crate::error::AppError;

pub const PRODUCT_NAME: &str = "Tabby RS";
pub const APP_IDENTIFIER: &str = "io.tabbyrs.app";
pub const CLI_NAME: &str = "tabby-rs";
pub const URL_SCHEME: &str = "tabby-rs";
pub const DATA_DIR_NAME: &str = "tabby-rs";
pub const CREDENTIAL_SERVICE: &str = "tabby-rs";
pub const PORTABLE_MARKER: &str = ".tabby-rs-portable";

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
}
