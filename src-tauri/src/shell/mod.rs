mod environment;
mod model;
mod unix;
#[cfg(windows)]
mod windows;

pub use environment::prepare_spawn;
pub use model::{
    DetectShellsRequest, DetectedShell, PrepareSpawnRequest, PreparedSpawnRequest,
    ShellDetectionResult, ShellType,
};

pub fn detect_shells(request: DetectShellsRequest) -> ShellDetectionResult {
    let mut warnings = Vec::new();

    #[cfg(windows)]
    let shells = windows::detect(request.identification.as_deref(), &mut warnings);

    #[cfg(not(windows))]
    let shells = {
        let _ = request;
        unix::detect(&mut warnings)
    };

    ShellDetectionResult { shells, warnings }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{detect_shells, DetectShellsRequest};

    #[test]
    fn detected_shell_ids_are_unique() {
        let result = detect_shells(DetectShellsRequest::default());
        let mut ids = BTreeSet::new();
        for shell in result.shells {
            assert!(ids.insert(shell.id));
        }
    }
}
