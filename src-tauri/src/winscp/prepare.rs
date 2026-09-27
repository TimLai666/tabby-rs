//! Converts a single private key on request and hands the converted key back to
//! the front end.
//!
//! `launch` keeps every converted key in a temporary file until WinSCP has
//! exited, so the front end asks for one key at a time here and only moves on to
//! the next file once WinSCP has rejected the previous one. The converted
//! content is returned instead of a file handle: the front end already holds
//! the source key and its passphrase, so nothing native has to stay alive
//! between the two calls. Files are removed when conversion returns normally;
//! abrupt application exit still needs the cleanup tracked in `AGENTS.md`.

use std::path::Path;

use super::key::KeyInput;
use crate::error::AppError;

/// A request to convert one private key.
///
/// This type carries key material, so it deliberately has no `Debug`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConvertKeyRequest {
    /// The `WinSCP.exe` whose `WinSCP.com` sibling converts the key.
    pub executable: String,
    pub key: KeyInput,
}

/// Converts one private key and returns what WinSCP accepted.
///
/// `Ok(None)` means WinSCP rejected the key, which is how the front end learns
/// to try the next file. WinSCP is a Windows program, so every other platform
/// reports that instead of starting anything.
#[tauri::command]
pub(crate) async fn winscp_convert_key(
    request: ConvertKeyRequest,
) -> Result<Option<KeyInput>, AppError> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            convert_one(&request, &mut super::launch::run_process)
        })
        .await
        .map_err(|_| preparation_failed())?
    }

    #[cfg(not(windows))]
    {
        let _ = request;
        Err(AppError::Unsupported(
            "WinSCP is available on Windows only".into(),
        ))
    }
}

/// Converts the one key the request carries and returns its content.
///
/// `run` is the only way a process is started. `Ok(None)` means WinSCP rejected
/// the key, so the front end can ask for the next file.
fn convert_one(
    request: &ConvertKeyRequest,
    run: &mut impl FnMut(&Path, &[String]) -> Result<(), AppError>,
) -> Result<Option<KeyInput>, AppError> {
    // Only an entirely blank executable is rejected. Whatever else was
    // configured is used verbatim, because a path the user stored is the path
    // the key has to be converted with.
    if request.executable.trim().is_empty() {
        return Err(AppError::InvalidArgument(
            "WinSCP executable must not be empty".into(),
        ));
    }
    let executable = Path::new(&request.executable);

    // Reuse the native key conversion and temporary-file handling. `None` is
    // the front end's signal to offer the next file.
    let Some(converted) = super::key::convert(executable, std::slice::from_ref(&request.key), run)?
    else {
        return Ok(None);
    };

    // The key is read while it is still there, and `converted` is dropped as
    // this function returns, on this path as well as on the read error below,
    // so no key file or handle outlives the command.
    let content = std::fs::read_to_string(&converted.path).map_err(|_| preparation_failed())?;

    Ok(Some(KeyInput {
        content,
        passphrase: Some(converted.passphrase.clone()),
    }))
}

/// Builds the one error this module is allowed to report for a key it could not
/// convert or read back. The `std::io::Error` is dropped on purpose: its message
/// can name the temporary key file and therefore the key material.
fn preparation_failed() -> AppError {
    AppError::Io("WinSCP private key preparation failed".into())
}

#[cfg(test)]
mod tests {
    use super::{convert_one, ConvertKeyRequest};
    use crate::error::AppError;
    use crate::winscp::key::KeyInput;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// A `WinSCP.exe` and the `WinSCP.com` sibling the key is converted with.
    const EXECUTABLE: &str = "/winscp/WinSCP.exe";
    const KEYGEN: &str = "/winscp/WinSCP.com";

    fn request(content: &str, passphrase: Option<&str>) -> ConvertKeyRequest {
        ConvertKeyRequest {
            executable: EXECUTABLE.into(),
            key: KeyInput {
                content: content.into(),
                passphrase: passphrase.map(str::to_owned),
            },
        }
    }

    /// Takes the error of a failed conversion. `KeyInput` deliberately has no
    /// `Debug`, so the error cannot be unwrapped from the `Ok` side and is
    /// matched by hand.
    fn expect_error(result: Result<Option<KeyInput>, AppError>) -> AppError {
        match result {
            Err(error) => error,
            Ok(_) => panic!("the request must be rejected"),
        }
    }

    #[test]
    fn reads_a_camel_case_request_and_writes_a_key_back() {
        let request: ConvertKeyRequest = serde_json::from_str(
            r#"{"executable":"C:\\WinSCP\\WinSCP.exe",
                "key":{"content":"-----BEGIN-----","passphrase":"p"}}"#,
        )
        .unwrap();

        assert_eq!(request.executable, r"C:\WinSCP\WinSCP.exe");
        assert_eq!(request.key.content, "-----BEGIN-----");
        assert_eq!(request.key.passphrase.as_deref(), Some("p"));

        let key = KeyInput {
            content: "converted".into(),
            passphrase: Some("tabby".into()),
        };
        assert_eq!(
            serde_json::to_value(&key).unwrap(),
            serde_json::json!({"content": "converted", "passphrase": "tabby"})
        );
    }

    #[test]
    fn returns_the_converted_content_and_removes_the_staged_file() {
        // What a real `WinSCP.com /keygen` leaves in the staged file.
        const CONVERTED: &str = "PuTTY-User-Key-File-2: ssh-rsa\n轉換後🔑\n";
        let staged = RefCell::new(PathBuf::new());
        let mut run = |executable: &Path, args: &[String]| {
            assert_eq!(executable, Path::new(KEYGEN));
            let path = PathBuf::from(&args[1]);
            std::fs::write(&path, CONVERTED).unwrap();
            *staged.borrow_mut() = path;
            Ok(())
        };

        let converted = convert_one(&request("-----BEGIN-----", Some("hunter2")), &mut run)
            .unwrap()
            .expect("the key converts");

        assert_eq!(converted.content, CONVERTED);
        assert_eq!(converted.passphrase.as_deref(), Some("hunter2"));
        let staged = staged.into_inner();
        assert!(
            !staged.exists(),
            "{} must be gone before the command returns",
            staged.display()
        );
    }

    #[test]
    fn a_rejected_key_is_reported_as_none_with_no_file_left() {
        let staged = RefCell::new(PathBuf::new());
        let mut run = |_: &Path, args: &[String]| {
            *staged.borrow_mut() = PathBuf::from(&args[1]);
            Err(AppError::Io("mock keygen failure".into()))
        };

        let converted = convert_one(&request("-----BEGIN-----", None), &mut run).unwrap();

        assert!(converted.is_none(), "the front end has to try the next key");
        let staged = staged.into_inner();
        assert!(!staged.exists(), "{} must be removed", staged.display());
    }

    #[test]
    fn an_unreadable_converted_key_is_reported_without_its_cause() {
        let mut run = |_: &Path, args: &[String]| {
            std::fs::remove_file(&args[1]).unwrap();
            Ok(())
        };

        let error = expect_error(convert_one(&request("-----BEGIN-----", None), &mut run));

        assert!(
            matches!(&error, AppError::Io(details) if details == "WinSCP private key preparation failed"),
            "{error:?}"
        );
    }

    #[test]
    fn a_blank_executable_never_starts_a_process() {
        let mut run = |_: &Path, _: &[String]| -> Result<(), AppError> {
            panic!("no key may be staged and nothing may be started");
        };
        let mut request = request("-----BEGIN-----", None);
        request.executable = " \t\n ".into();

        let error = expect_error(convert_one(&request, &mut run));

        assert!(
            matches!(&error, AppError::InvalidArgument(details) if details == "WinSCP executable must not be empty"),
            "{error:?}"
        );
    }

    #[test]
    fn converts_a_configured_executable_with_its_whitespace_kept() {
        // Only an entirely blank executable is rejected; whatever else was
        // configured is what the key has to be converted with, so a trimmed
        // path would be reported here instead of the configured one.
        let configured = " \tC:\\Program Files\\WinSCP\\WinSCP.exe";
        let mut run = |executable: &Path, _: &[String]| {
            assert_eq!(
                executable,
                Path::new(" \tC:\\Program Files\\WinSCP\\WinSCP.com")
            );
            Err(AppError::Io("mock keygen failure".into()))
        };
        let mut request = request("-----BEGIN-----", None);
        request.executable = configured.into();

        let converted = convert_one(&request, &mut run).unwrap();

        assert!(converted.is_none(), "the conversion was still attempted");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn the_command_reports_unsupported_off_windows() {
        let error = expect_error(super::winscp_convert_key(request("-----BEGIN-----", None)).await);

        assert!(
            matches!(&error, AppError::Unsupported(details) if details == "WinSCP is available on Windows only"),
            "{error:?}"
        );
    }
}
