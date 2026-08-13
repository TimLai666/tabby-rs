mod error {
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
}

#[path = "../src/plugins/npm.rs"]
mod npm;
