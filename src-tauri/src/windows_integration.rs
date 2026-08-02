use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

const EXTRAS_DIRECTORY: &str = "extras";
const UAC_HELPER_NAME: &str = "UAC.exe";

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowsIntegrationStatus {
    pub available: bool,
    pub clink_path: Option<String>,
    pub uac_helper_path: Option<String>,
    pub warnings: Vec<String>,
}

pub fn status(app: &AppHandle) -> WindowsIntegrationStatus {
    #[cfg(not(windows))]
    {
        let _ = app;
        return WindowsIntegrationStatus {
            available: false,
            clink_path: None,
            uac_helper_path: None,
            warnings: Vec::new(),
        };
    }

    #[cfg(windows)]
    {
        let roots = resource_roots(app);
        let clink_path = find_clink(&roots).and_then(path_text);
        let uac_helper_path = find_uac_helper(&roots).and_then(path_text);
        let mut warnings = Vec::new();
        if clink_path.is_none() {
            warnings.push("Bundled Clink helper was not found; stock CMD remains available.".into());
        }
        if uac_helper_path.is_none() {
            warnings.push("Bundled UAC helper was not found; administrator sessions are disabled.".into());
        }
        WindowsIntegrationStatus {
            available: true,
            clink_path,
            uac_helper_path,
            warnings,
        }
    }
}

pub fn resource_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join(EXTRAS_DIRECTORY));
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("resources").join(EXTRAS_DIRECTORY));
            candidates.push(parent.join(EXTRAS_DIRECTORY));
        }
    }

    if cfg!(debug_assertions) {
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(EXTRAS_DIRECTORY),
        );
    }

    canonical_regular_directories(candidates)
}

pub fn find_clink(roots: &[PathBuf]) -> Option<PathBuf> {
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => return None,
    };
    find_regular_resource(
        roots,
        &Path::new("clink").join(format!("clink_{architecture}.exe")),
    )
}

pub fn find_uac_helper(roots: &[PathBuf]) -> Option<PathBuf> {
    find_regular_resource(roots, Path::new(UAC_HELPER_NAME))
}

fn canonical_regular_directories(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let metadata = std::fs::symlink_metadata(&candidate).ok()?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
            let canonical = candidate.canonicalize().ok()?;
            seen.insert(canonical.clone()).then_some(canonical)
        })
        .collect()
}

fn find_regular_resource(roots: &[PathBuf], relative: &Path) -> Option<PathBuf> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return None;
    }

    roots.iter().find_map(|root| {
        let candidate = root.join(relative);
        let metadata = std::fs::symlink_metadata(&candidate).ok()?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return None;
        }
        let canonical = candidate.canonicalize().ok()?;
        canonical.starts_with(root).then_some(canonical)
    })
}

fn path_text(path: PathBuf) -> Option<String> {
    path.into_os_string().into_string().ok()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::{find_regular_resource, UAC_HELPER_NAME};

    #[test]
    fn resource_lookup_rejects_parent_traversal() {
        let temp = tempfile::tempdir().unwrap();
        assert!(find_regular_resource(&[temp.path().to_path_buf()], Path::new("../UAC.exe"))
            .is_none());
    }

    #[test]
    fn resource_lookup_accepts_a_regular_file_inside_root() {
        let temp = tempfile::tempdir().unwrap();
        let helper = temp.path().join(UAC_HELPER_NAME);
        fs::write(&helper, b"fixture").unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();
        assert_eq!(
            find_regular_resource(&[canonical_root], Path::new(UAC_HELPER_NAME)),
            helper.canonicalize().ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn resource_lookup_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.exe");
        fs::write(&target, b"fixture").unwrap();
        symlink(&target, temp.path().join(UAC_HELPER_NAME)).unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();
        assert!(find_regular_resource(&[canonical_root], Path::new(UAC_HELPER_NAME)).is_none());
    }
}
