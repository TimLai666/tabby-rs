//! Converts a private key into the format WinSCP can use for a session.
//!
//! The key material only ever lives in a temporary file, and this module hands
//! back a handle that deletes that file again. `run` is supplied by the caller
//! so this module never starts a process on its own, and no key content,
//! passphrase, argument or subprocess error is ever logged or passed on.

use std::path::Path;

use crate::error::AppError;

/// A private key as the front end hands it over.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyInput {
    pub content: String,
    pub passphrase: Option<String>,
}

/// A key that WinSCP accepted, ready to be passed to the main program.
pub(crate) struct ConvertedKey {
    /// Temporary file holding the converted key. Dropping it removes the file,
    /// so the caller has to keep it alive until WinSCP has finished.
    pub path: tempfile::TempPath,
    pub passphrase: String,
}

/// Passphrase assumed for keys that arrive without one.
const DEFAULT_PASSPHRASE: &str = "tabby";

/// Builds the one error this module is allowed to report. The `std::io::Error`
/// is dropped on purpose: its message can contain paths and key material.
fn preparation_failed() -> AppError {
    AppError::Io("WinSCP private key preparation failed".into())
}

/// Tries every key in order and returns the first one WinSCP accepts.
///
/// A rejected key is only dropped, so the caller never learns why. `Ok(None)`
/// means that no key was supplied, or that none of them could be converted.
pub(crate) fn convert(
    executable: &Path,
    keys: &[KeyInput],
    run: &mut impl FnMut(&Path, &[String]) -> Result<(), AppError>,
) -> Result<Option<ConvertedKey>, AppError> {
    let keygen = executable.with_extension("com");
    for key in keys {
        let path = stage(&key.content)?;
        let staged = path.to_string_lossy().into_owned();
        let passphrase = key.passphrase.as_deref().unwrap_or(DEFAULT_PASSPHRASE);
        let args = vec![
            "/keygen".to_owned(),
            staged.clone(),
            "-o".to_owned(),
            staged,
            "--old-passphrase".to_owned(),
            passphrase.to_owned(),
        ];
        if run(&keygen, &args).is_err() {
            // `path` goes out of scope here, which deletes the staged key.
            continue;
        }
        return Ok(Some(ConvertedKey {
            path,
            passphrase: passphrase.to_owned(),
        }));
    }
    Ok(None)
}

/// Writes key material to a private temporary file and returns the handle that
/// deletes it again.
pub(super) fn stage_prepared(key: &KeyInput) -> Result<ConvertedKey, AppError> {
    Ok(ConvertedKey {
        path: stage(&key.content)?,
        passphrase: key
            .passphrase
            .clone()
            .unwrap_or_else(|| DEFAULT_PASSPHRASE.into()),
    })
}

fn stage(content: &str) -> Result<tempfile::TempPath, AppError> {
    let file = tempfile::NamedTempFile::new().map_err(|_| preparation_failed())?;
    // The file handle has to be released before the content is written and
    // before WinSCP rewrites the file in place.
    let (handle, path) = file.into_parts();
    drop(handle);
    std::fs::write(&path, content).map_err(|_| preparation_failed())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{convert, preparation_failed, ConvertedKey, KeyInput};
    use crate::error::AppError;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// The `WinSCP.com` sibling every key is converted with.
    const EXECUTABLE: &str = "/winscp/WinSCP.exe";
    const KEYGEN: &str = "/winscp/WinSCP.com";

    type Call = (PathBuf, Vec<String>);

    fn input(content: &str, passphrase: Option<&str>) -> KeyInput {
        KeyInput {
            content: content.into(),
            passphrase: passphrase.map(str::to_owned),
        }
    }

    /// A converter that records every call and accepts every key.
    fn accepting(
        calls: &RefCell<Vec<Call>>,
    ) -> impl FnMut(&Path, &[String]) -> Result<(), AppError> + '_ {
        move |executable, args| {
            calls
                .borrow_mut()
                .push((executable.to_path_buf(), args.to_vec()));
            Ok(())
        }
    }

    /// The exact argument list `convert` has to build for a staged key.
    fn expected_args(staged: &str, passphrase: &str) -> Vec<String> {
        [
            "/keygen",
            staged,
            "-o",
            staged,
            "--old-passphrase",
            passphrase,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn reads_camel_case_input_from_the_front_end() {
        let key: KeyInput =
            serde_json::from_str(r#"{"content":"-----BEGIN-----","passphrase":"p"}"#).unwrap();
        assert_eq!(key.content, "-----BEGIN-----");
        assert_eq!(key.passphrase.as_deref(), Some("p"));

        let key: KeyInput = serde_json::from_str(r#"{"content":"-----BEGIN-----"}"#).unwrap();
        assert_eq!(key.passphrase, None);
    }

    #[test]
    fn empty_keys_never_reach_winscp() {
        let calls = RefCell::new(Vec::new());
        let converted = convert(Path::new(EXECUTABLE), &[], &mut accepting(&calls)).unwrap();

        assert!(converted.is_none());
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn writes_unicode_content_and_calls_keygen_with_exact_args() {
        let calls = RefCell::new(Vec::new());
        let content = "-----BEGIN RSA PRIVATE KEY-----\n秘密🔑\n-----END-----\n".to_owned();
        let keys = [input(&content, Some("hunter2"))];

        let converted = convert(Path::new(EXECUTABLE), &keys, &mut accepting(&calls))
            .unwrap()
            .expect("the only key converts");
        let staged = converted.path.to_path_buf();
        let staged_path = staged.to_string_lossy().into_owned();

        assert_eq!(std::fs::read_to_string(&staged).unwrap(), content);
        assert_eq!(converted.passphrase, "hunter2");
        assert_eq!(calls.borrow()[0].0, PathBuf::from(KEYGEN));
        assert_eq!(calls.borrow()[0].1, expected_args(&staged_path, "hunter2"));
    }

    #[test]
    fn keys_without_passphrase_fall_back_to_tabby() {
        let calls = RefCell::new(Vec::new());
        let keys = [input("-----BEGIN-----", None)];

        let converted = convert(Path::new(EXECUTABLE), &keys, &mut accepting(&calls))
            .unwrap()
            .expect("the only key converts");
        let staged = converted.path.to_path_buf();
        let staged_path = staged.to_string_lossy().into_owned();

        assert_eq!(converted.passphrase, "tabby");
        assert_eq!(calls.borrow()[0].0, PathBuf::from(KEYGEN));
        assert_eq!(calls.borrow()[0].1, expected_args(&staged_path, "tabby"));
    }

    #[test]
    fn converted_key_survives_until_it_is_dropped() {
        let calls = RefCell::new(Vec::new());
        let keys = [input("-----BEGIN-----", None)];

        let converted: ConvertedKey = convert(Path::new(EXECUTABLE), &keys, &mut accepting(&calls))
            .unwrap()
            .expect("the only key converts");
        let staged = converted.path.to_path_buf();
        assert!(staged.exists(), "key must outlive the call itself");

        drop(converted);
        assert!(
            !staged.exists(),
            "key must be gone once the handle is dropped"
        );
    }

    #[test]
    fn a_rejected_key_is_removed_and_the_next_one_is_tried() {
        let staged = RefCell::new(Vec::<PathBuf>::new());
        let mut attempts = 0;
        let mut run = |executable: &Path, args: &[String]| {
            staged.borrow_mut().push(PathBuf::from(&args[1]));
            assert_eq!(executable, Path::new(KEYGEN));
            attempts += 1;
            if attempts == 1 {
                Err(AppError::InvalidData("mock".into()))
            } else {
                Ok(())
            }
        };
        let keys = [input("first", None), input("second", None)];

        let converted = convert(Path::new(EXECUTABLE), &keys, &mut run)
            .unwrap()
            .expect("the second key converts");

        assert_eq!(std::fs::read_to_string(&converted.path).unwrap(), "second");
        let staged = staged.borrow();
        assert_eq!(staged.len(), 2);
        assert!(!staged[0].exists(), "rejected key must be removed");
        assert!(staged[1].exists(), "accepted key must be kept");
    }

    #[test]
    fn no_key_is_reported_when_every_attempt_fails() {
        let staged = RefCell::new(Vec::<PathBuf>::new());
        let mut run = |_: &Path, args: &[String]| {
            staged.borrow_mut().push(PathBuf::from(&args[1]));
            Err(AppError::Io("mock".into()))
        };
        let keys = [input("first", None), input("second", None)];

        let converted = convert(Path::new(EXECUTABLE), &keys, &mut run).unwrap();

        assert!(converted.is_none());
        assert_eq!(staged.borrow().len(), 2);
        for path in staged.borrow().iter() {
            assert!(!path.exists(), "{} must not be left behind", path.display());
        }
    }

    #[test]
    fn staging_failures_never_carry_their_cause() {
        let error = preparation_failed();

        assert!(
            matches!(&error, AppError::Io(details) if details == "WinSCP private key preparation failed"),
            "{error:?}"
        );
    }
}
