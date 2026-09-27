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

/// Converts the keys, builds the arguments and starts WinSCP once.
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

    // The jump host is converted first, so its key exists while the target key
    // is being converted.
    let jump_key = match request.jump.as_ref() {
        Some(jump) => super::key::convert(executable, &jump.private_keys, run)?,
        None => None,
    };
    let target_key = super::key::convert(executable, &request.target.private_keys, run)?;

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
fn run_process(executable: &Path, args: &[String]) -> Result<(), AppError> {
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
    use crate::winscp::key::KeyInput;
    use crate::winscp::uri::ConnectionOptions;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// A `WinSCP.exe` and the `WinSCP.com` sibling every key is converted with.
    const EXECUTABLE: &str = "/winscp/WinSCP.exe";
    const KEYGEN: &str = "/winscp/WinSCP.com";
    const JUMP_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\n跳板🔑\n-----END-----\n";
    const TARGET_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\n目標🔑\n-----END-----\n";

    fn options(host: &str, username: &str, keys: &[(&str, Option<&str>)]) -> ConnectionOptions {
        ConnectionOptions {
            host: host.into(),
            port: 22,
            username: username.into(),
            password: None,
            private_keys: keys
                .iter()
                .map(|(content, passphrase)| KeyInput {
                    content: (*content).into(),
                    passphrase: passphrase.map(str::to_owned),
                })
                .collect(),
        }
    }

    fn request(target: ConnectionOptions, jump: Option<ConnectionOptions>) -> LaunchRequest {
        LaunchRequest {
            executable: EXECUTABLE.into(),
            target,
            jump,
        }
    }

    /// Every process call a launch made, the key files that were staged on the
    /// way, and for each of them whether it was still there when WinSCP started.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<(PathBuf, Vec<String>)>,
        staged: Vec<PathBuf>,
        alive_at_launch: Vec<bool>,
    }

    /// Records every call. `accept_keys` decides whether a key conversion
    /// succeeds; the WinSCP launch itself always does.
    fn record(
        calls: &RefCell<Recorder>,
        accept_keys: bool,
    ) -> impl FnMut(&Path, &[String]) -> Result<(), AppError> + '_ {
        move |executable, args| {
            let keygen = executable == Path::new(KEYGEN);
            let mut record = calls.borrow_mut();
            if keygen {
                record.staged.push(PathBuf::from(&args[1]));
            } else {
                record.alive_at_launch = record.staged.iter().map(|key| key.exists()).collect();
            }
            record.calls.push((executable.to_path_buf(), args.to_vec()));
            if keygen && !accept_keys {
                return Err(AppError::Io("mock keygen failure".into()));
            }
            Ok(())
        }
    }

    /// The exact argument list a key conversion has to use.
    fn keygen_args(staged: &str, passphrase: &str) -> Vec<String> {
        ["/keygen", staged, "-o", staged, "--old-passphrase", passphrase]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// The tunnel key file a session URL points at, decoded.
    ///
    /// The value is percent-encoded the way `encodeURIComponent` does, which is
    /// what `form_urlencoded` decodes, so the staged path is compared exactly as
    /// it was recorded. Re-encoding the path here would only test this module
    /// against itself, and decoding `%2F` alone would miss a Windows path with
    /// `%3A` or `%5C` in it.
    fn tunnel_key_file(uri: &str) -> String {
        let (encoded, _) = uri
            .split_once(";x-tunnelpublickeyfile=")
            .and_then(|(_, rest)| rest.split_once(';'))
            .expect("a tunnel key file is followed by the tunnel passphrase");
        let pair = format!("p={encoded}");
        let (_, value) = url::form_urlencoded::parse(pair.as_bytes())
            .next()
            .expect("the tunnel key file is percent-encoded");
        value.into_owned()
    }

    /// No staged key file may outlive the launch.
    fn assert_no_key_files_left(calls: &RefCell<Recorder>) {
        for key in &calls.borrow().staged {
            assert!(!key.exists(), "{} must be removed", key.display());
        }
    }

    #[test]
    fn reads_a_camel_case_request_from_the_front_end() {
        let request: LaunchRequest = serde_json::from_str(
            r#"{"executable":"C:\\WinSCP\\WinSCP.exe","target":{"host":"h","port":22,"username":"u"}}"#,
        )
        .unwrap();
        assert_eq!(request.executable, r"C:\WinSCP\WinSCP.exe");
        assert_eq!(request.target.host, "h");
        assert!(request.jump.is_none());

        let request: LaunchRequest = serde_json::from_str(
            r#"{"executable":"e","target":{"host":"h","port":22,"username":"u"},
                "jump":{"host":"j","port":2222,"username":"v"}}"#,
        )
        .unwrap();
        assert_eq!(request.jump.as_ref().unwrap().host, "j");
    }

    #[test]
    fn opens_a_direct_session_with_only_the_uri() {
        let calls = RefCell::new(Recorder::default());
        let request = request(options("example.com", "alice", &[]), None);

        launch_with(&request, &mut record(&calls, true)).unwrap();

        assert_eq!(
            calls.borrow().calls,
            vec![(
                PathBuf::from(EXECUTABLE),
                vec!["scp://alice@example.com:22/".to_owned()]
            )]
        );
    }

    #[test]
    fn starts_the_configured_executable_verbatim() {
        // A configured path may carry surrounding whitespace; only an entirely
        // blank one is rejected, and the rest has to be used as configured.
        let configured = " \tC:\\Program Files\\WinSCP\\WinSCP.exe \n";
        let calls = RefCell::new(Recorder::default());
        let request = LaunchRequest {
            executable: configured.into(),
            target: options("example.com", "alice", &[]),
            jump: None,
        };

        launch_with(&request, &mut record(&calls, true)).unwrap();

        let (executable, args) = calls.borrow().calls.last().cloned().unwrap();
        assert_eq!(executable, Path::new(configured));
        assert_eq!(args, vec!["scp://alice@example.com:22/".to_owned()]);
    }

    #[test]
    fn converts_the_jump_key_first_and_keeps_both_files_until_winscp_is_done() {
        let calls = RefCell::new(Recorder::default());
        let target = options("example.com", "alice", &[(TARGET_KEY, Some("hunter2"))]);
        let jump = options("jump.example.net", "bob", &[(JUMP_KEY, None)]);
        let request = request(target, Some(jump));

        launch_with(&request, &mut record(&calls, true)).unwrap();

        let record = calls.borrow();
        assert_eq!(record.calls.len(), 3, "jump key, target key, WinSCP");
        let (jump_file, target_file) = (&record.calls[0].1[1], &record.calls[1].1[1]);
        assert_eq!(record.calls[0].0, PathBuf::from(KEYGEN));
        assert_eq!(record.calls[1].0, PathBuf::from(KEYGEN));
        assert_eq!(record.calls[0].1, keygen_args(jump_file, "tabby"));
        assert_eq!(record.calls[1].1, keygen_args(target_file, "hunter2"));
        assert_ne!(jump_file, target_file, "each key gets its own file");
        assert_eq!(
            record.alive_at_launch,
            vec![true, true],
            "both keys must exist while WinSCP reads them"
        );

        let (executable, args) = record.calls.last().unwrap();
        assert_eq!(executable, Path::new(EXECUTABLE));
        assert_eq!(args.len(), 3, "the URI and both key arguments stay separate");
        let (uri, private_key, passphrase) = (&args[0], &args[1], &args[2]);
        assert!(!uri.contains(jump_file), "the key path must be encoded");
        assert_eq!(tunnel_key_file(uri), *jump_file);
        assert!(uri.contains(";x-tunnelpassphraseplain=tabby"), "{uri}");
        assert_eq!(private_key, &format!("/privatekey={target_file}"));
        assert_eq!(passphrase, "/passphrase=hunter2");
        drop(record);

        assert_no_key_files_left(&calls);
    }

    #[test]
    fn a_failing_launch_is_reported_without_its_cause_and_removes_every_key() {
        let calls = RefCell::new(Recorder::default());
        let target = options("example.com", "alice", &[(TARGET_KEY, None)]);
        let jump = options("jump.example.net", "bob", &[(JUMP_KEY, None)]);
        let request = request(target, Some(jump));
        let mut run = |executable: &Path, args: &[String]| {
            if executable != Path::new(KEYGEN) {
                calls
                    .borrow_mut()
                    .calls
                    .push((executable.to_path_buf(), args.to_vec()));
                return Err(AppError::Io(r"spawn failed at C:\Temp\key 🔑.ppk".into()));
            }
            record(&calls, true)(executable, args)
        };

        let error = launch_with(&request, &mut run).unwrap_err();

        assert!(
            matches!(&error, AppError::Io(details) if details == "WinSCP launch failed"),
            "{error:?}"
        );
        assert_eq!(calls.borrow().staged.len(), 2);
        assert_no_key_files_left(&calls);
    }

    #[test]
    fn opens_the_session_unchanged_when_no_key_can_be_converted() {
        let calls = RefCell::new(Recorder::default());
        let target = options("example.com", "alice", &[("first", None), ("second", None)]);
        let jump = options("jump.example.net", "bob", &[("third", None)]);
        let request = request(target, Some(jump));

        launch_with(&request, &mut record(&calls, false)).unwrap();

        let (executable, args) = calls.borrow().calls.last().cloned().unwrap();
        assert_eq!(executable, Path::new(EXECUTABLE));
        assert_eq!(
            args,
            vec!["scp://alice;x-tunnel=1;x-tunnelhostname=jump.example.net\
                 ;x-tunnelportnumber=22;x-tunnelusername=bob@example.com:22/"
                .to_owned()],
            "a keyless session is opened exactly like the keyless URI"
        );
        assert_eq!(calls.borrow().staged.len(), 3, "every key was attempted");
        assert_eq!(
            calls.borrow().alive_at_launch,
            vec![false; 3],
            "no rejected key file survives to the launch"
        );
        assert_no_key_files_left(&calls);
    }

    #[test]
    fn invalid_requests_never_start_a_process() {
        let blank = LaunchRequest {
            executable: " \t\n ".into(),
            target: options("example.com", "alice", &[(TARGET_KEY, None)]),
            jump: Some(options("jump.example.net", "bob", &[(JUMP_KEY, None)])),
        };
        let cases = [
            (blank, "WinSCP executable must not be empty"),
            (
                request(options("", "alice", &[(TARGET_KEY, None)]), None),
                "target host must not be empty",
            ),
            (
                request(
                    options("example.com", "alice", &[(TARGET_KEY, None)]),
                    Some(options("", "bob", &[(JUMP_KEY, None)])),
                ),
                "jump host must not be empty",
            ),
        ];

        for (request, expected) in cases {
            let calls = RefCell::new(Recorder::default());
            let error = launch_with(&request, &mut record(&calls, true)).unwrap_err();

            assert!(
                matches!(&error, AppError::InvalidArgument(details) if details == expected),
                "{error:?}"
            );
            assert!(
                calls.borrow().calls.is_empty(),
                "no key may be converted and nothing may be started"
            );
        }
    }

    #[test]
    fn process_failures_are_reported_without_their_cause() {
        let error = run_process(
            Path::new("/nonexistent-dir/WinSCP.exe"),
            &["scp://u@example.com:22/".to_owned()],
        )
        .unwrap_err();

        assert!(
            matches!(&error, AppError::Io(details) if details == "WinSCP launch failed"),
            "{error:?}"
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn the_command_reports_unsupported_off_windows() {
        let request = request(options("example.com", "alice", &[(TARGET_KEY, None)]), None);

        let error = super::winscp_launch(request).await.unwrap_err();

        assert!(
            matches!(&error, AppError::Unsupported(details) if details == "WinSCP is available on Windows only"),
            "{error:?}"
        );
    }
}
