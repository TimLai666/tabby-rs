use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::oneshot,
    time::{error::Elapsed, timeout},
};

use crate::error::AppError;

const MAX_NPM_OUTPUT: usize = 64 * 1024;
const NPM_OPERATION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PLUGIN_PACKAGE_MANIFEST: &str = r#"{
  "name": "tabby-rs-plugins",
  "private": true,
  "version": "0.0.0"
}
"#;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOperation {
    pub id: String,
    pub package_name: String,
    pub action: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Clone, Default)]
pub struct OperationManager {
    cancellations: Arc<Mutex<HashMap<String, OperationCancellation>>>,
}

struct OperationCancellation {
    cancelled: Arc<AtomicBool>,
    registered: bool,
    sender: Option<oneshot::Sender<()>>,
}

impl OperationManager {
    pub fn reserve(&self, id: &str) -> Result<(), AppError> {
        validate_operation_id(id)?;
        let mut cancellations = self
            .cancellations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if cancellations.contains_key(id) {
            return Err(AppError::Conflict(
                "plugin operation ID is already active".into(),
            ));
        }
        cancellations.insert(
            id.into(),
            OperationCancellation {
                cancelled: Arc::new(AtomicBool::new(false)),
                registered: false,
                sender: None,
            },
        );
        Ok(())
    }

    pub fn register(&self, id: &str) -> Result<oneshot::Receiver<()>, AppError> {
        validate_operation_id(id)?;
        let mut cancellations = self
            .cancellations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(operation) = cancellations.get_mut(id) {
            if operation.registered {
                return Err(AppError::Conflict(
                    "plugin operation ID is already active".into(),
                ));
            }
            operation.registered = true;
            let (sender, receiver) = oneshot::channel();
            if operation.cancelled.load(Ordering::Acquire) {
                drop(sender);
            } else {
                operation.sender = Some(sender);
            }
            return Ok(receiver);
        }
        let (sender, receiver) = oneshot::channel();
        cancellations.insert(
            id.into(),
            OperationCancellation {
                cancelled: Arc::new(AtomicBool::new(false)),
                registered: true,
                sender: Some(sender),
            },
        );
        Ok(receiver)
    }

    pub fn cancel(&self, id: &str) -> Result<(), AppError> {
        let mut cancellations = self
            .cancellations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let operation = cancellations
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound(format!("plugin operation {id} not found")))?;
        if operation.cancelled.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if let Some(sender) = operation.sender.take() {
            let _ = sender.send(());
        }
        Ok(())
    }

    pub fn finish(&self, id: &str) {
        self.cancellations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(id);
    }
}

pub fn validate_operation_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        return Err(AppError::InvalidArgument(
            "plugin operation ID is invalid".into(),
        ));
    }
    Ok(())
}

pub async fn install(
    root: PathBuf,
    node_path: PathBuf,
    npm_path: PathBuf,
    operation_id: &str,
    package_name: &str,
    version: &str,
    cancel: oneshot::Receiver<()>,
    progress: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<PluginOperation, AppError> {
    validate_operation_id(operation_id)?;
    let package_spec = validate_package_spec(package_name, version)?;
    prepare_plugin_root(&root)?;
    ensure_plugin_manifest(&root)?;
    reject_package_symlink_escape(&root, package_name)?;
    run_npm(
        &root,
        &node_path,
        &npm_path,
        "install",
        &[
            "--no-audit".into(),
            "--no-fund".into(),
            "--".into(),
            package_spec,
        ],
        cancel,
        progress,
    )
    .await?;
    Ok(PluginOperation {
        id: operation_id.into(),
        package_name: package_name.into(),
        action: "install".into(),
        status: "succeeded".into(),
        message: None,
    })
}

pub async fn uninstall(
    root: PathBuf,
    node_path: PathBuf,
    npm_path: PathBuf,
    operation_id: &str,
    package_name: &str,
    cancel: oneshot::Receiver<()>,
    progress: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<PluginOperation, AppError> {
    validate_operation_id(operation_id)?;
    validate_package_name(package_name)?;
    prepare_plugin_root(&root)?;
    ensure_plugin_manifest(&root)?;
    reject_package_symlink_escape(&root, package_name)?;
    run_npm(
        &root,
        &node_path,
        &npm_path,
        "uninstall",
        &[
            "--no-audit".into(),
            "--no-fund".into(),
            "--".into(),
            package_name.into(),
        ],
        cancel,
        progress,
    )
    .await?;
    Ok(PluginOperation {
        id: operation_id.into(),
        package_name: package_name.into(),
        action: "uninstall".into(),
        status: "succeeded".into(),
        message: None,
    })
}

pub async fn update(
    root: PathBuf,
    node_path: PathBuf,
    npm_path: PathBuf,
    operation_id: &str,
    package_name: &str,
    cancel: oneshot::Receiver<()>,
    progress: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<PluginOperation, AppError> {
    validate_operation_id(operation_id)?;
    validate_package_name(package_name)?;
    prepare_plugin_root(&root)?;
    ensure_plugin_manifest(&root)?;
    reject_package_symlink_escape(&root, package_name)?;
    run_npm(
        &root,
        &node_path,
        &npm_path,
        "update",
        &[
            "--no-audit".into(),
            "--no-fund".into(),
            "--".into(),
            package_name.into(),
        ],
        cancel,
        progress,
    )
    .await?;
    Ok(PluginOperation {
        id: operation_id.into(),
        package_name: package_name.into(),
        action: "update".into(),
        status: "succeeded".into(),
        message: None,
    })
}

fn validate_package_spec(package_name: &str, version: &str) -> Result<String, AppError> {
    validate_package_name(package_name)?;
    validate_version(version)?;
    Ok(format!("{package_name}@{version}"))
}

pub(super) fn validate_package_name(package_name: &str) -> Result<(), AppError> {
    if package_name.is_empty()
        || package_name.len() > 214
        || package_name.starts_with('-')
        || package_name.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\\'
        })
    {
        return Err(AppError::InvalidArgument(
            "plugin package name is invalid".into(),
        ));
    }

    let parts = package_name.split('/').collect::<Vec<_>>();
    let valid = match parts.as_slice() {
        [name] => !name.starts_with('@') && valid_package_part(name),
        [scope, name] => {
            scope.starts_with('@') && valid_package_part(&scope[1..]) && valid_package_part(name)
        }
        _ => false,
    };
    if !valid {
        return Err(AppError::InvalidArgument(
            "plugin package name is invalid".into(),
        ));
    }
    Ok(())
}

fn valid_package_part(part: &str) -> bool {
    let mut characters = part.chars();
    matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || "-_.".contains(character)
        })
}

pub(super) fn validate_version(version: &str) -> Result<(), AppError> {
    if version.is_empty()
        || version.starts_with('-')
        || !version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".+-".contains(character))
    {
        return Err(AppError::InvalidArgument(
            "plugin version is invalid".into(),
        ));
    }

    let normalized = version.strip_prefix('v').unwrap_or(version);
    let separator = normalized.find(['-', '+']);
    let (core, suffix) = separator
        .map(|index| normalized.split_at(index))
        .unwrap_or((normalized, ""));
    if !suffix.is_empty() && !suffix[1..].chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::InvalidArgument(
            "plugin version is invalid".into(),
        ));
    }
    let components = core.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components
            .iter()
            .any(|component| component.is_empty() || !component.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(AppError::InvalidArgument(
            "plugin version is invalid".into(),
        ));
    }
    Ok(())
}

fn prepare_plugin_root(root: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AppError::PermissionDenied(
                "plugin root must not be a symbolic link".into(),
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(AppError::InvalidData(
                "plugin root is not a directory".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(root)?;
        }
        Err(error) => return Err(error.into()),
    }

    let node_modules = root.join("node_modules");
    match fs::symlink_metadata(&node_modules) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::PermissionDenied(
            "plugin node_modules must not be a symbolic link".into(),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(AppError::InvalidData(
            "plugin node_modules is not a directory".into(),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_plugin_manifest(root: &Path) -> Result<(), AppError> {
    let path = root.join("package.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AppError::PermissionDenied(
                "plugin package.json must not be a symbolic link".into(),
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(AppError::InvalidData(
                "plugin package.json is not a regular file".into(),
            ));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut file = match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    Err(AppError::PermissionDenied(
                        "plugin package.json must not be a symbolic link".into(),
                    ))
                }
                Ok(metadata) if metadata.is_file() => Ok(()),
                Ok(_) => Err(AppError::InvalidData(
                    "plugin package.json is not a regular file".into(),
                )),
                Err(error) => Err(error.into()),
            };
        }
        Err(error) => return Err(error.into()),
    };
    file.write_all(PLUGIN_PACKAGE_MANIFEST.as_bytes())?;
    file.flush()?;
    Ok(())
}

fn reject_package_symlink_escape(root: &Path, package_name: &str) -> Result<(), AppError> {
    let package_path = package_path(root, package_name)?;
    let mut current = root.join("node_modules");
    for component in package_path.strip_prefix(&current).unwrap().components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(AppError::PermissionDenied(
                    "refusing to operate on a symlinked plugin package".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn package_path(root: &Path, package_name: &str) -> Result<PathBuf, AppError> {
    validate_package_name(package_name)?;
    let mut path = root.join("node_modules");
    for part in package_name.split('/') {
        path.push(part);
    }
    Ok(path)
}

async fn run_npm(
    root: &Path,
    node_path: &Path,
    npm_path: &Path,
    action: &str,
    args: &[String],
    mut cancel: oneshot::Receiver<()>,
    progress: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<(), AppError> {
    let mut command = npm_command(node_path, npm_path, action, args)?;
    command
        .current_dir(root)
        .env("PATH", tool_search_path(node_path, npm_path))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| AppError::Io(format!("could not start npm: {error}")))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let secret_values = sensitive_environment_values();
    let process = async {
        timeout(NPM_OPERATION_TIMEOUT, async {
            tokio::join!(
                read_limited(stdout, "stdout", progress.clone(), &secret_values),
                read_limited(stderr, "stderr", progress, &secret_values),
                child.wait()
            )
        })
        .await
    };
    let result = tokio::select! {
        result = process => result,
        _ = &mut cancel => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(AppError::Conflict("plugin operation cancelled".into()));
        }
    };
    let (stdout, stderr, status) = match result {
        Ok(result) => result,
        Err(Elapsed { .. }) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(AppError::Io(
                "npm operation timed out after 15 minutes".into(),
            ));
        }
    };
    let _ = stdout?;
    let _ = stderr?;
    let status = status?;
    if !status.success() {
        return Err(AppError::Io(format!("npm {action} failed")));
    }
    Ok(())
}

fn npm_command(
    _node_path: &Path,
    npm_path: &Path,
    action: &str,
    args: &[String],
) -> Result<Command, AppError> {
    #[cfg(windows)]
    if npm_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
    {
        let cli = npm_cli_path(_node_path, npm_path).ok_or_else(|| {
            AppError::Unsupported("npm CLI JavaScript entry was not found".into())
        })?;
        let mut command = Command::new(_node_path);
        command.arg(cli).arg(action).args(args);
        return Ok(command);
    }

    let mut command = Command::new(npm_path);
    command.arg(action).args(args);
    Ok(command)
}

#[cfg(windows)]
fn npm_cli_path(node_path: &Path, npm_path: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for parent in [node_path.parent(), npm_path.parent()]
        .into_iter()
        .flatten()
    {
        candidates.push(parent.join("node_modules/npm/bin/npm-cli.js"));
        candidates.push(parent.join("../lib/node_modules/npm/bin/npm-cli.js"));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn tool_search_path(node_path: &Path, npm_path: &Path) -> OsString {
    let mut paths = Vec::new();
    for path in [node_path.parent(), npm_path.parent()]
        .into_iter()
        .flatten()
    {
        if !paths.iter().any(|existing: &PathBuf| existing == path) {
            paths.push(path.to_path_buf());
        }
    }
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).unwrap_or_default()
}

async fn read_limited<R>(
    reader: Option<R>,
    stream: &str,
    progress: Arc<dyn Fn(String) + Send + Sync>,
    secret_values: &[String],
) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    let mut pending_progress = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        if output.len() < MAX_NPM_OUTPUT {
            let keep = (MAX_NPM_OUTPUT - output.len()).min(count);
            output.extend_from_slice(&buffer[..keep]);
            pending_progress.extend_from_slice(&buffer[..keep]);
            while let Some(index) = pending_progress.iter().position(|byte| *byte == b'\n') {
                let line = pending_progress.drain(..=index).collect::<Vec<_>>();
                let message = redact_output(&line, secret_values);
                if !message.is_empty() {
                    progress(format!("{stream}: {message}"));
                }
            }
            if pending_progress.len() > MAX_MESSAGE_CHARS * 4 {
                pending_progress.clear();
                progress(format!("{stream}: [output truncated]"));
            }
        }
    }
    if !pending_progress.is_empty() {
        let message = redact_output(&pending_progress, secret_values);
        if !message.is_empty() {
            progress(format!("{stream}: {message}"));
        }
    }
    Ok(output)
}

const MAX_MESSAGE_CHARS: usize = 4096;

fn sensitive_environment_values() -> Vec<String> {
    env::vars()
        .filter_map(|(key, value)| {
            let key = key.to_ascii_lowercase();
            let sensitive = [
                "token",
                "secret",
                "password",
                "passwd",
                "authorization",
                "credential",
                "private_key",
                "access_key",
                "api_key",
            ]
            .iter()
            .any(|part| key.contains(part));
            sensitive.then_some(value).filter(|value| !value.is_empty())
        })
        .collect()
}

fn redact_output(bytes: &[u8], secret_values: &[String]) -> String {
    let mut redacted = String::from_utf8_lossy(bytes).into_owned();
    for secret in secret_values.iter().filter(|value| !value.is_empty()) {
        redacted = redacted.replace(secret, "[redacted]");
    }
    redacted = redacted
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("token")
                || lower.contains("password")
                || lower.contains("_auth")
                || lower.contains("authorization")
            {
                "[redacted]".to_owned()
            } else {
                line.chars()
                    .filter(|character| !character.is_control())
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if redacted.chars().count() > MAX_MESSAGE_CHARS {
        redacted = redacted.chars().take(MAX_MESSAGE_CHARS).collect();
        redacted.push_str("…");
    }
    redacted
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::{fs, path::PathBuf};

    use tokio::io::AsyncWriteExt;

    use super::{
        ensure_plugin_manifest, package_path, prepare_plugin_root, read_limited, redact_output,
        tool_search_path, validate_operation_id, validate_package_name, validate_package_spec,
        OperationManager,
    };

    #[test]
    fn accepts_scoped_plugin_package_specs() {
        assert_eq!(
            validate_package_spec("@tabby-rs/plugin", "1.2.3-beta.1").unwrap(),
            "@tabby-rs/plugin@1.2.3-beta.1"
        );
    }

    #[test]
    fn rejects_option_injection_and_shell_metacharacters() {
        for package_name in [
            "--global",
            "tabby/plugin/extra",
            "tabby plugin",
            "tabby;rm",
            ".",
            "../tabby-plugin",
            "_tabby-plugin",
        ] {
            assert!(
                validate_package_name(package_name).is_err(),
                "{package_name}"
            );
        }
        for version in ["--legacy-peer-deps", "latest", "1.2", "1.2.3;rm", "1.2.3-"] {
            assert!(
                validate_package_spec("tabby-plugin", version).is_err(),
                "{version}"
            );
        }
    }

    #[test]
    fn creates_an_isolated_plugin_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        prepare_plugin_root(&root).unwrap();
        ensure_plugin_manifest(&root).unwrap();
        assert!(root.is_dir());
        assert!(root.join("package.json").is_file());
    }

    #[test]
    fn preserves_an_existing_plugin_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        prepare_plugin_root(&root).unwrap();
        fs::write(root.join("package.json"), b"{}\n").unwrap();
        assert!(ensure_plugin_manifest(&root).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_plugin_root_and_package_path() {
        use std::{fs, os::unix::fs::symlink};

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        symlink(outside.path(), &root).unwrap();
        assert!(prepare_plugin_root(&root).is_err());

        let safe_root = temp.path().join("safe");
        fs::create_dir_all(safe_root.join("node_modules")).unwrap();
        symlink(outside.path(), safe_root.join("node_modules/tabby-plugin")).unwrap();
        assert!(super::reject_package_symlink_escape(&safe_root, "tabby-plugin").is_err());
    }

    #[test]
    fn builds_package_path_without_interpreting_user_path() {
        let path = package_path(std::path::Path::new("/tmp/plugins"), "@scope/plugin").unwrap();
        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/plugins/node_modules/@scope/plugin")
        );
    }

    #[test]
    fn prepends_node_and_npm_directories_to_child_path() {
        let path = tool_search_path(
            std::path::Path::new("/custom/node/bin/node"),
            std::path::Path::new("/custom/npm/bin/npm"),
        );
        let entries = std::env::split_paths(&path).collect::<Vec<_>>();
        assert_eq!(entries[0], std::path::PathBuf::from("/custom/node/bin"));
        assert_eq!(entries[1], std::path::PathBuf::from("/custom/npm/bin"));
    }

    #[cfg(windows)]
    #[test]
    fn resolves_npm_cli_without_a_cmd_shell() {
        let temp = tempfile::tempdir().unwrap();
        let node_path = temp.path().join("node.exe");
        let npm_path = temp.path().join("npm.cmd");
        let cli = temp.path().join("node_modules/npm/bin/npm-cli.js");
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&node_path, b"").unwrap();
        fs::write(&npm_path, b"").unwrap();
        fs::write(&cli, b"").unwrap();

        assert_eq!(super::npm_cli_path(&node_path, &npm_path), Some(cli));
    }

    #[test]
    fn operation_manager_rejects_duplicate_and_invalid_ids() {
        let manager = OperationManager::default();
        let _cancel = manager.register("plugin-op-1").unwrap();
        assert!(manager.register("plugin-op-1").is_err());
        assert!(manager.cancel("plugin-op-1").is_ok());
        assert!(manager.cancel("plugin-op-1").is_ok());
        assert!(validate_operation_id("bad id").is_err());
    }

    #[tokio::test]
    async fn reserved_operation_can_be_cancelled_before_install_registers_it() {
        let manager = OperationManager::default();
        manager.reserve("plugin-op-2").unwrap();
        manager.cancel("plugin-op-2").unwrap();

        let cancel = manager.register("plugin-op-2").unwrap();
        assert!(cancel.await.is_err());
        manager.finish("plugin-op-2");
    }

    #[test]
    fn cancellation_is_idempotent_until_operation_finishes() {
        let manager = OperationManager::default();
        let _cancel = manager.register("plugin-op-3").unwrap();

        assert!(manager.cancel("plugin-op-3").is_ok());
        assert!(manager.cancel("plugin-op-3").is_ok());
        assert!(manager.register("plugin-op-3").is_err());
        manager.finish("plugin-op-3");
        assert!(manager.cancel("plugin-op-3").is_err());
    }

    #[test]
    fn redacts_sensitive_and_control_output_with_a_bound() {
        let output = redact_output(
            b"token=secret\nhello\x1b[2K\nvery long output that should remain bounded",
            &["secret".into()],
        );
        assert!(output.contains("[redacted]"));
        assert!(!output.contains('\x1b'));
        assert!(output.len() <= 4097);
    }

    #[test]
    fn redacts_exact_secret_values_without_sensitive_labels() {
        let output = redact_output(b"registry output: s3cr3t", &["s3cr3t".into()]);
        assert!(!output.contains("s3cr3t"));
        assert!(output.contains("[redacted]"));
    }

    #[tokio::test]
    async fn redacts_secret_split_across_read_chunks() {
        let (mut writer, reader) = tokio::io::duplex(16 * 1024);
        let mut input = vec![b'x'; 8190];
        input.extend_from_slice(b"s3cr3t\n");
        writer.write_all(&input).await.unwrap();
        drop(writer);

        let messages = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&messages);
        read_limited(
            Some(reader),
            "stdout",
            Arc::new(move |message| captured.lock().unwrap().push(message)),
            &["s3cr3t".into()],
        )
        .await
        .unwrap();

        let messages = messages.lock().unwrap().join("\n");
        assert!(!messages.contains("s3cr3t"));
    }

    #[tokio::test]
    #[ignore = "runs only from the system npm lifecycle harness"]
    async fn system_npm_plugin_lifecycle() {
        let node_path = PathBuf::from(
            std::env::var("TABBY_RS_NPM_E2E_NODE")
                .expect("TABBY_RS_NPM_E2E_NODE must point to the system Node.js executable"),
        );
        let npm_path = PathBuf::from(
            std::env::var("TABBY_RS_NPM_E2E_NPM")
                .expect("TABBY_RS_NPM_E2E_NPM must point to the system npm executable"),
        );
        let root = PathBuf::from(
            std::env::var("TABBY_RS_NPM_E2E_ROOT")
                .expect("TABBY_RS_NPM_E2E_ROOT must point to an isolated plugin root"),
        );
        let package_name = std::env::var("TABBY_RS_NPM_E2E_PACKAGE")
            .expect("TABBY_RS_NPM_E2E_PACKAGE must name the fixture package");
        let versions = ["1.0.0", "1.0.1"];
        let progress: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|message: String| {
            if std::env::var_os("TABBY_RS_NPM_E2E_DEBUG").is_some() {
                eprintln!("{message}");
            }
        });

        let (_sender, cancel) = tokio::sync::oneshot::channel();
        let operation = super::install(
            root.clone(),
            node_path.clone(),
            npm_path.clone(),
            "system-npm-install",
            &package_name,
            versions[0],
            cancel,
            Arc::clone(&progress),
        )
        .await
        .expect("system npm install should succeed");
        assert_eq!(operation.action, "install");

        let installed_manifest = root
            .join("node_modules")
            .join(&package_name)
            .join("package.json");
        let installed: serde_json::Value =
            serde_json::from_slice(&fs::read(&installed_manifest).unwrap()).unwrap();
        assert_eq!(installed["version"], versions[0]);

        {
            let (_sender, cancel) = tokio::sync::oneshot::channel();
            let operation = super::update(
                root.clone(),
                node_path.clone(),
                npm_path.clone(),
                "system-npm-update",
                &package_name,
                cancel,
                Arc::clone(&progress),
            )
            .await
            .expect("system npm update should succeed");
            assert_eq!(operation.action, "update");
        }

        let updated: serde_json::Value =
            serde_json::from_slice(&fs::read(&installed_manifest).unwrap()).unwrap();
        assert_eq!(updated["version"], versions[1]);

        let (_sender, cancel) = tokio::sync::oneshot::channel();
        let operation = super::uninstall(
            root.clone(),
            node_path,
            npm_path,
            "system-npm-uninstall",
            &package_name,
            cancel,
            progress,
        )
        .await
        .expect("system npm uninstall should succeed");
        assert_eq!(operation.status, "succeeded");
        assert!(!root.join("node_modules").join(&package_name).exists());
    }
}
