use std::path::{Component, Path, PathBuf};

use crate::{error::AppError, identity::AppPaths};

#[derive(Debug, Clone)]
pub struct StoragePaths {
    data_dir: PathBuf,
    config_file: PathBuf,
    state_file: PathBuf,
    backups_dir: PathBuf,
    migration_dir: PathBuf,
}

impl StoragePaths {
    pub fn from_app_paths(paths: &AppPaths) -> Self {
        Self::from_data_dir(paths.data_dir().clone())
    }

    pub fn from_data_dir(data_dir: PathBuf) -> Self {
        Self {
            config_file: data_dir.join("config.yaml"),
            state_file: data_dir.join("tabby-rs.json"),
            backups_dir: data_dir.join("backups"),
            migration_dir: data_dir.join("migration"),
            data_dir,
        }
    }

    pub fn ensure_layout(&self) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.backups_dir)?;
        std::fs::create_dir_all(&self.migration_dir)?;
        Ok(())
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    pub fn state_file(&self) -> &Path {
        &self.state_file
    }

    pub fn backups_dir(&self) -> &Path {
        &self.backups_dir
    }

    pub fn migration_dir(&self) -> &Path {
        &self.migration_dir
    }

    pub fn backup_dir(&self, backup_id: &str) -> Result<PathBuf, AppError> {
        validate_single_component(backup_id, "backup id")?;
        Ok(self.backups_dir.join(backup_id))
    }

    pub fn migration_file(&self, name: &str) -> Result<PathBuf, AppError> {
        validate_single_component(name, "migration file name")?;
        Ok(self.migration_dir.join(name))
    }
}

pub fn validate_single_component(value: &str, label: &str) -> Result<(), AppError> {
    if value.is_empty() || value.len() > 160 {
        return Err(AppError::InvalidArgument(format!("invalid {label}")));
    }
    let path = Path::new(value);
    let mut components = path.components();
    let valid = matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && !value.contains(['/', '\\', '\0']);
    if !valid {
        return Err(AppError::InvalidArgument(format!("invalid {label}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_single_component;

    #[test]
    fn rejects_traversal_and_nested_paths() {
        for value in ["", "..", "../x", "x/y", "x\\y", "."] {
            assert!(validate_single_component(value, "id").is_err(), "{value}");
        }
        assert!(validate_single_component("20260801T010203Z-manual", "id").is_ok());
    }
}
