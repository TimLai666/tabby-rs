#[derive(Debug, serde::Serialize, thiserror::Error)]
#[serde(tag = "code", content = "details", rename_all = "camelCase")]
pub enum AppError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("unsupported on this platform: {0}")]
    Unsupported(String),
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidData(error.to_string())
    }
}

impl From<serde_yaml::Error> for AppError {
    fn from(error: serde_yaml::Error) -> Self {
        Self::InvalidData(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn serializes_public_error_shape() {
        let value = serde_json::to_value(AppError::NotFound("profile".into())).unwrap();
        assert_eq!(value["code"], "notFound");
        assert_eq!(value["details"], "profile");
    }

    #[test]
    fn exposes_revision_conflicts_without_internal_paths() {
        let value =
            serde_json::to_value(AppError::Conflict("config revision changed".into())).unwrap();
        assert_eq!(value["code"], "conflict");
        assert_eq!(value["details"], "config revision changed");
    }
}
