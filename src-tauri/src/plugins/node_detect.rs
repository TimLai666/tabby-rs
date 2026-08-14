use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::{error::Elapsed, timeout},
};

use crate::error::AppError;

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_VERSION_OUTPUT: usize = 4096;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeToolchainStatus {
    pub node_path: Option<PathBuf>,
    pub node_version: Option<String>,
    pub npm_path: Option<PathBuf>,
    pub npm_version: Option<String>,
    pub supported: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub version: Option<String>,
    pub error: Option<String>,
}

impl ToolResult {
    #[cfg(test)]
    fn success(version: &str) -> Self {
        Self {
            version: Some(version.into()),
            error: None,
        }
    }

    #[cfg(test)]
    fn failure(error: &str) -> Self {
        Self {
            version: None,
            error: Some(error.into()),
        }
    }
}

pub async fn detect(custom_node_path: Option<String>) -> Result<NodeToolchainStatus, AppError> {
    let custom_path_requested = custom_node_path.is_some();
    let node_path = resolve_node_path(custom_node_path)?;
    let npm_path = resolve_npm_path(node_path.as_deref());

    let node_result = match node_path.as_ref() {
        Some(path) => Some(run_version(path, None).await),
        None => None,
    };
    let npm_result = match npm_path.as_ref() {
        Some(path) => Some(run_version(path, node_path.as_deref()).await),
        None => None,
    };

    let mut result = evaluate_toolchain(node_result, npm_result);
    if custom_path_requested && node_path.is_none() {
        result.reason = Some("the custom Node.js path was not found".into());
    }
    result.node_path = node_path;
    result.npm_path = npm_path;
    Ok(result)
}

fn resolve_node_path(custom_node_path: Option<String>) -> Result<Option<PathBuf>, AppError> {
    if let Some(path) = custom_node_path {
        if path.chars().any(char::is_control) {
            return Err(AppError::InvalidArgument(
                "custom Node.js path contains control characters".into(),
            ));
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(AppError::InvalidArgument(
                "custom Node.js path must be absolute".into(),
            ));
        }
        if !is_executable_file(&path) {
            return Ok(None);
        }
        return Ok(Some(path));
    }

    Ok(find_on_path(&node_command_names()).or_else(find_common_node_path))
}

fn resolve_npm_path(node_path: Option<&Path>) -> Option<PathBuf> {
    let names = npm_command_names();
    if let Some(node_path) = node_path {
        if let Some(parent) = node_path.parent() {
            if let Some(path) = first_executable(parent, &names) {
                return Some(path);
            }
        }
    }
    find_on_path(&names).or_else(find_common_npm_path)
}

fn evaluate_toolchain(node: Option<ToolResult>, npm: Option<ToolResult>) -> NodeToolchainStatus {
    let node_version = node.as_ref().and_then(|result| result.version.clone());
    let npm_version = npm.as_ref().and_then(|result| result.version.clone());
    let reason = match node.as_ref() {
        None => Some("Node.js was not found".into()),
        Some(result) if result.error.is_some() => Some(format!(
            "node --version failed: {}",
            result.error.as_deref().unwrap_or("unknown error")
        )),
        Some(_) if npm.is_none() => Some("npm was not found".into()),
        Some(_)
            if npm
                .as_ref()
                .and_then(|result| result.error.as_ref())
                .is_some() =>
        {
            Some(format!(
                "npm --version failed: {}",
                npm.as_ref()
                    .and_then(|result| result.error.as_deref())
                    .unwrap_or("unknown error")
            ))
        }
        Some(_) if node_version.is_none() => Some("node --version returned no version".into()),
        Some(_) if npm_version.is_none() => Some("npm --version returned no version".into()),
        Some(_) => None,
    };

    NodeToolchainStatus {
        node_path: None,
        node_version,
        npm_path: None,
        npm_version,
        supported: reason.is_none(),
        reason,
    }
}

async fn run_version(path: &Path, additional_path: Option<&Path>) -> ToolResult {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut search_paths = vec![parent.to_path_buf()];
    if let Some(additional_path) = additional_path {
        search_paths.push(additional_path.to_path_buf());
    }
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let search_path =
        env::join_paths(search_paths).unwrap_or_else(|_| parent.as_os_str().to_os_string());
    let mut command = Command::new(path);
    command
        .arg("--version")
        .current_dir(parent)
        .env_clear()
        .env("PATH", search_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ToolResult {
                version: None,
                error: Some(error.to_string()),
            }
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let result = timeout(VERSION_TIMEOUT, async move {
        let (stdout, stderr) = tokio::join!(read_limited(stdout), read_limited(stderr));
        let status = child.wait().await;
        (stdout, stderr, status)
    })
    .await;

    match result {
        Ok((Ok(stdout), Ok(stderr), Ok(status))) if status.success() => ToolResult {
            version: sanitize_version_output(&stdout),
            error: if sanitize_version_output(&stdout).is_none() {
                Some(if stderr.is_empty() {
                    "version output was invalid".into()
                } else {
                    String::from_utf8_lossy(&stderr).trim().to_owned()
                })
            } else {
                None
            },
        },
        Ok((Ok(_), Ok(stderr), Ok(status))) => ToolResult {
            version: None,
            error: Some(if stderr.is_empty() {
                format!("process exited with {status}")
            } else {
                String::from_utf8_lossy(&stderr).trim().to_owned()
            }),
        },
        Ok((Err(error), _, _)) | Ok((_, Err(error), _)) => ToolResult {
            version: None,
            error: Some(error.to_string()),
        },
        Ok((_, _, Err(error))) => ToolResult {
            version: None,
            error: Some(error.to_string()),
        },
        Err(Elapsed { .. }) => ToolResult {
            version: None,
            error: Some("timed out after 3 seconds".into()),
        },
    }
}

async fn read_limited<R>(reader: Option<R>) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    reader
        .take((MAX_VERSION_OUTPUT + 1) as u64)
        .read_to_end(&mut output)
        .await?;
    if output.len() > MAX_VERSION_OUTPUT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "version output exceeded 4096 bytes",
        ));
    }
    Ok(output)
}

fn sanitize_version_output(output: &[u8]) -> Option<String> {
    if output.len() > MAX_VERSION_OUTPUT
        || output
            .iter()
            .any(|byte| byte.is_ascii_control() && !byte.is_ascii_whitespace())
    {
        return None;
    }
    let version = String::from_utf8(output.to_vec()).ok()?.trim().to_owned();
    if version.is_empty() || version.lines().count() != 1 || !is_version_like(&version) {
        return None;
    }
    Some(version)
}

fn is_version_like(version: &str) -> bool {
    let version = version.strip_prefix('v').unwrap_or(version);
    let core = version
        .split(|character| character == '-' || character == '+')
        .next();
    let Some(core) = core else {
        return false;
    };
    let components = core.split('.').collect::<Vec<_>>();
    components.len() == 3
        && components
            .iter()
            .all(|component| !component.is_empty() && component.chars().all(|c| c.is_ascii_digit()))
}

fn node_command_names() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["node.exe", "node"]
    } else {
        vec!["node"]
    }
}

fn npm_command_names() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["npm.cmd", "npm.exe", "npm"]
    } else {
        vec!["npm"]
    }
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|directory| first_executable(&directory, names))
}

fn find_common_node_path() -> Option<PathBuf> {
    common_path_directories()
        .iter()
        .find_map(|directory| first_executable(directory, &node_command_names()))
}

fn find_common_npm_path() -> Option<PathBuf> {
    common_path_directories()
        .iter()
        .find_map(|directory| first_executable(directory, &npm_command_names()))
}

fn common_path_directories() -> Vec<PathBuf> {
    let mut directories = vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")];
    #[cfg(target_os = "macos")]
    directories.push(PathBuf::from("/opt/homebrew/bin"));
    #[cfg(windows)]
    {
        if let Some(program_files) = env::var_os("ProgramFiles") {
            directories.push(PathBuf::from(program_files).join("nodejs"));
        }
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            directories.push(
                PathBuf::from(local_app_data)
                    .join("Programs")
                    .join("nodejs"),
            );
        }
    }
    directories
}

fn first_executable(directory: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| directory.join(name))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        return path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate_toolchain, resolve_node_path, sanitize_version_output, ToolResult};

    #[cfg(unix)]
    fn write_executable(
        directory: &std::path::Path,
        name: &str,
        contents: &str,
    ) -> std::path::PathBuf {
        use std::{fs, os::unix::fs::PermissionsExt};

        let path = directory.join(name);
        fs::write(&path, contents).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn supports_node_and_npm_with_valid_versions() {
        let result = evaluate_toolchain(
            Some(ToolResult::success("v22.1.0")),
            Some(ToolResult::success("10.8.2")),
        );

        assert!(result.supported);
        assert!(result.reason.is_none());
    }

    #[test]
    fn requires_both_node_and_npm() {
        let result = evaluate_toolchain(Some(ToolResult::success("v22.1.0")), None);

        assert!(!result.supported);
        assert_eq!(result.node_version.as_deref(), Some("v22.1.0"));
        assert!(result.npm_version.is_none());
        assert_eq!(result.reason.as_deref(), Some("npm was not found"));
    }

    #[test]
    fn rejects_failed_version_commands() {
        let result = evaluate_toolchain(
            Some(ToolResult::failure("permission denied")),
            Some(ToolResult::success("10.8.2")),
        );

        assert!(!result.supported);
        assert_eq!(
            result.reason.as_deref(),
            Some("node --version failed: permission denied")
        );
    }

    #[test]
    fn reports_version_command_timeout() {
        let result = evaluate_toolchain(
            Some(ToolResult::success("v22.1.0")),
            Some(ToolResult::failure("timed out after 3 seconds")),
        );

        assert!(!result.supported);
        assert_eq!(
            result.reason.as_deref(),
            Some("npm --version failed: timed out after 3 seconds")
        );
    }

    #[test]
    fn rejects_empty_version_output() {
        let node_result = evaluate_toolchain(
            Some(ToolResult {
                version: None,
                error: None,
            }),
            Some(ToolResult::success("10.8.2")),
        );
        assert!(!node_result.supported);
        assert_eq!(
            node_result.reason.as_deref(),
            Some("node --version returned no version")
        );

        let npm_result = evaluate_toolchain(
            Some(ToolResult::success("v22.1.0")),
            Some(ToolResult {
                version: None,
                error: None,
            }),
        );
        assert!(!npm_result.supported);
        assert_eq!(
            npm_result.reason.as_deref(),
            Some("npm --version returned no version")
        );
    }

    #[test]
    fn trims_version_output_and_rejects_control_data() {
        assert_eq!(
            sanitize_version_output(b" v22.1.0\r\n"),
            Some("v22.1.0".into())
        );
        assert_eq!(sanitize_version_output(b"v22.1.0\nextra"), None);
        assert_eq!(sanitize_version_output(b"ok"), None);
        assert_eq!(sanitize_version_output(&vec![b'x'; 4097]), None);
    }

    #[cfg(unix)]
    #[test]
    fn skips_non_executable_path_entries() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let candidate = directory.path().join("node");
        fs::write(&candidate, "not executable").unwrap();
        let mut permissions = fs::metadata(&candidate).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&candidate, permissions).unwrap();

        assert!(super::first_executable(directory.path(), &["node"]).is_none());
    }

    #[test]
    fn custom_node_path_must_be_absolute_and_clean() {
        assert!(resolve_node_path(Some("node".into())).is_err());
        assert!(resolve_node_path(Some("/tmp/node\n".into())).is_err());
    }

    #[tokio::test]
    async fn reports_missing_custom_node_path() {
        let directory = tempfile::tempdir().unwrap();
        let node_path = directory.path().join("missing-node");

        let result = super::detect(Some(node_path.to_string_lossy().into_owned()))
            .await
            .unwrap();

        assert!(!result.supported);
        assert!(result.node_path.is_none());
        assert_eq!(
            result.reason.as_deref(),
            Some("the custom Node.js path was not found")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uses_custom_node_and_adjacent_npm() {
        let directory = tempfile::tempdir().unwrap();
        let node_path =
            write_executable(directory.path(), "node", "#!/bin/sh\nprintf 'v22.1.0\\n'\n");
        let npm_path = write_executable(directory.path(), "npm", "#!/bin/sh\nprintf '10.8.2\\n'\n");

        let result = super::detect(Some(node_path.to_string_lossy().into_owned()))
            .await
            .unwrap();

        assert!(result.supported);
        assert_eq!(result.node_path.as_deref(), Some(node_path.as_path()));
        assert_eq!(result.npm_path.as_deref(), Some(npm_path.as_path()));
        assert_eq!(result.node_version.as_deref(), Some("v22.1.0"));
        assert_eq!(result.npm_version.as_deref(), Some("10.8.2"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_invalid_custom_node_version_output() {
        let directory = tempfile::tempdir().unwrap();
        let node_path = write_executable(
            directory.path(),
            "node",
            "#!/bin/sh\nprintf 'not-a-version\\n'\n",
        );
        write_executable(directory.path(), "npm", "#!/bin/sh\nprintf '10.8.2\\n'\n");

        let result = super::detect(Some(node_path.to_string_lossy().into_owned()))
            .await
            .unwrap();

        assert!(!result.supported);
        assert_eq!(
            result.reason.as_deref(),
            Some("node --version failed: version output was invalid")
        );
    }
}
