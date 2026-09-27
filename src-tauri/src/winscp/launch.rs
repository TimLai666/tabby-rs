//! Starts WinSCP with a session URL and keeps the converted key files in place
//! until the spawned process has exited.
//!
//! The launch follows `tabby-ssh/src/services/ssh.service.ts`: the session URL
//! is the first argument, a converted target key is added as two separate
//! `/privatekey=` and `/passphrase=` arguments, and no instance, shell or
//! working directory option is forced, so a WinSCP that is already running may
//! take the session over. That hand-off has not been checked on Windows, so the
//! only process lifetime relied on here is the one this module waits for.
//! Key material, passphrases, arguments and subprocess errors never leave this
//! module; a process that will not start or will not succeed is reported as one
//! fixed message.

use std::path::Path;
#[cfg(any(windows, test))]
use std::process::Stdio;

use super::uri::ConnectionOptions;
use crate::error::AppError;

/// A request to open one WinSCP session.
///
/// This type carries passwords and key material, so it deliberately has no
/// `Debug`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaunchRequest {
    /// The `WinSCP.exe` the session is opened with.
    pub executable: String,
    pub target: ConnectionOptions,
    /// An optional jump host, turned into an `x-tunnel` in the session URL.
    pub jump: Option<ConnectionOptions>,
}

/// Opens the WinSCP session described by `request`.
///
/// WinSCP is a Windows program, so every other platform reports that instead
/// of starting anything.
#[tauri::command]
pub(crate) async fn winscp_launch(request: LaunchRequest) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || launch_with(&request, &mut run_process))
            .await
            .map_err(|_| launch_failed())?
    }

    #[cfg(not(windows))]
    {
        let _ = request;
        Err(AppError::Unsupported(
            "WinSCP is available on Windows only".into(),
        ))
    }
}

/// Stages the already converted keys, builds the arguments and starts WinSCP once.
///
/// `run` is the only way a process is started, so a caller can inspect the
/// exact argument list. Both converted keys are held in this function, which
/// keeps their temporary files in place until `run` has returned; dropping them
/// removes the files again, whether the launch succeeded or not.
fn launch_with(
    request: &LaunchRequest,
    run: &mut impl FnMut(&Path, &[String]) -> Result<(), AppError>,
) -> Result<(), AppError> {
    // Only an entirely blank executable is rejected. Whatever else was
    // configured is used verbatim, because a path the user stored is the path
    // WinSCP has to be started with.
    if request.executable.trim().is_empty() {
        return Err(AppError::InvalidArgument(
            "WinSCP executable must not be empty".into(),
        ));
    }
    let executable = Path::new(&request.executable);

    // The request is validated before a key is staged or a process is started.
    super::uri::connection_uri(&request.target, request.jump.as_ref(), None)?;

    let jump_key = request
        .jump
        .as_ref()
        .and_then(|jump| jump.private_key.as_ref())
        .map(super::key::stage_prepared)
        .transpose()?;
    let target_key = request
        .target
        .private_key
        .as_ref()
        .map(super::key::stage_prepared)
        .transpose()?;

    let uri = super::uri::connection_uri(
        &request.target,
        request.jump.as_ref(),
        jump_key
            .as_ref()
            .map(|key| (&*key.path, key.passphrase.as_str())),
    )?;

    let mut args = vec![uri];
    if let Some(key) = target_key.as_ref() {
        args.push(format!("/privatekey={}", key.path.to_string_lossy()));
        args.push(format!("/passphrase={}", key.passphrase));
    }

    run(executable, &args).map_err(|_| launch_failed())
}

/// Starts a process with every stream detached, waits for it to finish and
/// requires a successful exit.
#[cfg(any(windows, test))]
pub(super) fn run_process(executable: &Path, args: &[String]) -> Result<(), AppError> {
    let status = std::process::Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| launch_failed())?;
    status.success().then_some(()).ok_or_else(launch_failed)
}

/// The only error this module reports for a process that would not start or
/// would not succeed. The `std::io::Error` is dropped on purpose: it can name
/// the executable, the arguments and therefore the passphrases.
fn launch_failed() -> AppError {
    AppError::Io("WinSCP launch failed".into())
}

#[cfg(test)]
mod tests {
    use super::{launch_with, run_process, LaunchRequest};
    use crate::error::AppError;
    use std::path::{Path, PathBuf};

    const EXECUTABLE: &str = "/winscp/WinSCP.exe";

    fn request(with_keys: bool) -> LaunchRequest {
        let mut value = serde_json::json!({
            "executable": EXECUTABLE,
            "target": {"host":"target", "port":22, "username":"alice"},
            "jump": {"host":"jump", "port":2222, "username":"bob"}
        });
        if with_keys {
            value["target"]["privateKey"] =
                serde_json::json!({"content":"converted target", "passphrase":""});
            value["jump"]["privateKey"] =
                serde_json::json!({"content":"converted jump", "passphrase":"phrase"});
        }
        serde_json::from_value(value).unwrap()
    }

    fn tunnel_key_file(uri: &str) -> PathBuf {
        let encoded = uri
            .split_once(";x-tunnelpublickeyfile=")
            .unwrap()
            .1
            .split(';')
            .next()
            .unwrap();
        let pair = format!("p={encoded}");
        PathBuf::from(
            url::form_urlencoded::parse(pair.as_bytes())
                .next()
                .unwrap()
                .1
                .into_owned(),
        )
    }

    #[test]
    fn launches_frontend_prepared_keys_without_converting_again() {
        let request = request(true);
        let mut paths = Vec::new();
        let mut count = 0;
        launch_with(&request, &mut |executable, args| {
            count += 1;
            assert_eq!(executable, Path::new(EXECUTABLE));
            assert_eq!(args.len(), 3, "prepared target key must reach WinSCP");
            let target = PathBuf::from(args[1].strip_prefix("/privatekey=").unwrap());
            let jump = tunnel_key_file(&args[0]);
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "converted target"
            );
            assert_eq!(std::fs::read_to_string(&jump).unwrap(), "converted jump");
            assert_eq!(args[2], "/passphrase=");
            assert!(args[0].contains(";x-tunnelpassphraseplain=phrase"));
            paths.extend([target, jump]);
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1);
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn reads_camel_case_and_optional_request_fields() {
        let value: LaunchRequest = serde_json::from_str(
            r#"{"executable":"C:\\WinSCP\\WinSCP.exe","target":{"host":"h","port":22,"username":"u"}}"#,
        ).unwrap();
        assert_eq!(value.executable, r"C:\WinSCP\WinSCP.exe");
        assert!(value.jump.is_none());
        assert!(value.target.private_key.is_none());
        assert!(request(true).target.private_key.is_some());
    }

    #[test]
    fn keyless_sessions_and_configured_executable_are_preserved() {
        for configured in [EXECUTABLE, " \tC:\\Program Files\\WinSCP\\WinSCP.exe \n"] {
            let mut request = request(false);
            request.executable = configured.into();
            request.jump = None;
            let mut count = 0;
            launch_with(&request, &mut |exe, args| {
                count += 1;
                assert_eq!(exe, Path::new(configured));
                assert_eq!(args, ["scp://alice@target:22/"]);
                Ok(())
            })
            .unwrap();
            assert_eq!(count, 1);
        }
        launch_with(&request(false), &mut |_, args| {
            assert_eq!(args, ["scp://alice;x-tunnel=1;x-tunnelhostname=jump;x-tunnelportnumber=2222;x-tunnelusername=bob@target:22/"]);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn launch_failure_is_redacted_and_removes_both_keys() {
        let mut paths = Vec::new();
        let error = launch_with(&request(true), &mut |_, args| {
            paths.push(PathBuf::from(args[1].strip_prefix("/privatekey=").unwrap()));
            paths.push(tunnel_key_file(&args[0]));
            assert!(paths.iter().all(|path| path.exists()));
            Err(AppError::Io("secret path and password".into()))
        })
        .unwrap_err();
        assert!(matches!(error, AppError::Io(message) if message == "WinSCP launch failed"));
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn invalid_requests_never_start_a_process() {
        for variant in 0..3 {
            let mut request = request(true);
            match variant {
                0 => request.executable = " \t\n".into(),
                1 => request.target.host.clear(),
                _ => request.jump.as_mut().unwrap().port = 0,
            }
            let error = launch_with(&request, &mut |_, _| {
                panic!("invalid request reached process")
            });
            assert!(matches!(error, Err(AppError::InvalidArgument(_))));
        }
    }

    #[test]
    fn process_failures_are_reported_without_their_cause() {
        let error = run_process(
            Path::new("/nonexistent-dir/WinSCP.exe"),
            &["scp://u@host:22/".into()],
        )
        .unwrap_err();
        assert!(matches!(error, AppError::Io(message) if message == "WinSCP launch failed"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn the_command_reports_unsupported_off_windows() {
        let error = super::winscp_launch(request(true)).await.unwrap_err();
        assert!(
            matches!(error, AppError::Unsupported(message) if message == "WinSCP is available on Windows only")
        );
    }
}
