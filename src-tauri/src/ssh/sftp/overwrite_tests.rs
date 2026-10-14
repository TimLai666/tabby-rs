// OpenSSH POSIX rename overwrite regressions over the serialized SFTP protocol.
use super::{SftpManager, SftpOverwritePolicy};
use russh_sftp::{
    client::{error::Error as SftpClientError, SftpSession},
    protocol::{
        Attrs, Data, FileAttributes, FileMode, Handle, OpenFlags, Packet, Status, StatusCode,
        Version,
    },
    server::{Handler, StatusReply},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::duplex,
    time::{sleep, timeout},
};

const POSIX_RENAME: &str = "posix-rename@openssh.com";

#[derive(Default)]
struct Memory {
    files: HashMap<String, Vec<u8>>,
    handles: HashMap<String, String>,
    opened: Vec<String>,
    events: Vec<String>,
    sequence: u64,
    fail_posix_rename: bool,
    extension_version: Option<String>,
    extended_reply_injection: Option<ExtendedReplyInjection>,
}

#[derive(Clone, Copy)]
enum ExtendedReplyInjection {
    Status(StatusCode),
    UnexpectedPacket,
}

struct Peer(Arc<Mutex<Memory>>);

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}

fn take_string(data: &[u8], offset: &mut usize) -> String {
    let len = u32::from_be_bytes(data[*offset..*offset + 4].try_into().unwrap()) as usize;
    *offset += 4;
    let value = String::from_utf8(data[*offset..*offset + len].to_vec()).unwrap();
    *offset += len;
    value
}

impl Handler for Peer {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusReply::new(StatusCode::OpUnsupported)
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        let mut version = Version::new();
        if let Some(extension_version) = self.0.lock().unwrap().extension_version.clone() {
            version
                .extensions
                .insert(POSIX_RENAME.into(), extension_version);
        }
        Ok(version)
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let state = self.0.lock().unwrap();
        let bytes = state
            .files
            .get(&path)
            .ok_or_else(|| StatusReply::new(StatusCode::NoSuchFile))?;
        let mut attrs = FileAttributes {
            size: Some(bytes.len() as u64),
            permissions: Some(0o644),
            ..Default::default()
        };
        attrs.set_type(FileMode::REG);
        Ok(Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let mut state = self.0.lock().unwrap();
        if flags.contains(OpenFlags::EXCLUDE) && state.files.contains_key(&filename) {
            return Err(StatusReply::new(StatusCode::Failure));
        }
        if flags.contains(OpenFlags::CREATE) {
            state.files.entry(filename.clone()).or_default();
        }
        if !state.files.contains_key(&filename) {
            return Err(StatusReply::new(StatusCode::NoSuchFile));
        }
        if flags.contains(OpenFlags::TRUNCATE) {
            state.files.get_mut(&filename).unwrap().clear();
        }
        state.sequence += 1;
        let handle = format!("handle-{}", state.sequence);
        state.events.push(format!("open:{filename}"));
        state.opened.push(filename.clone());
        state.handles.insert(handle.clone(), filename);
        Ok(Handle { id, handle })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        let path = state
            .handles
            .get(&handle)
            .cloned()
            .ok_or_else(|| StatusReply::new(StatusCode::Failure))?;
        let target = state.files.get_mut(&path).unwrap();
        let start = offset as usize;
        target.resize(target.len().max(start + bytes.len()), 0);
        target[start..start + bytes.len()].copy_from_slice(&bytes);
        Ok(ok(id))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        if let Some(path) = state.handles.remove(&handle) {
            state.events.push(format!("close:{path}"));
        }
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        state.events.push(format!("rename:{from}:{to}"));
        // SFTP v3 RENAME has OpenSSH's no-existing-destination behavior.
        if state.files.contains_key(&to) {
            return Err(StatusReply::new(StatusCode::Failure));
        }
        let bytes = state
            .files
            .remove(&from)
            .ok_or_else(|| StatusReply::new(StatusCode::NoSuchFile))?;
        state.files.insert(to, bytes);
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        state.events.push(format!("remove:{path}"));
        state.files.remove(&path);
        Ok(ok(id))
    }

    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, Self::Error> {
        let mut state = self.0.lock().unwrap();
        if request != POSIX_RENAME {
            return Err(StatusReply::new(StatusCode::OpUnsupported));
        }
        let mut offset = 0;
        let from = take_string(&data, &mut offset);
        let to = take_string(&data, &mut offset);
        assert_eq!(
            offset,
            data.len(),
            "POSIX rename payload must contain exactly two SSH strings"
        );
        state.events.push(format!("extended:{from}:{to}"));
        if let Some(injection) = state.extended_reply_injection {
            return match injection {
                ExtendedReplyInjection::Status(status) => Err(StatusReply::new(status)),
                ExtendedReplyInjection::UnexpectedPacket => {
                    Ok(Packet::ExtendedReply(russh_sftp::protocol::ExtendedReply {
                        id,
                        data: Vec::new(),
                    }))
                }
            };
        }
        if state.fail_posix_rename {
            state.fail_posix_rename = false;
            return Err(StatusReply::new(StatusCode::PermissionDenied));
        }
        let bytes = state
            .files
            .remove(&from)
            .ok_or_else(|| StatusReply::new(StatusCode::NoSuchFile))?;
        state.files.insert(to, bytes);
        Ok(Packet::Status(ok(id)))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let state = self.0.lock().unwrap();
        let path = state
            .handles
            .get(&handle)
            .ok_or_else(|| StatusReply::new(StatusCode::Failure))?;
        let data = &state.files[path];
        let start = offset as usize;
        if start >= data.len() {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        Ok(Data {
            id,
            data: data[start..data.len().min(start + len as usize)].to_vec(),
        })
    }
}

struct Fixture {
    state: Arc<Mutex<Memory>>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_extension(Some("1"))
    }

    fn with_extension(extension_version: Option<&str>) -> Self {
        Self {
            state: Arc::new(Mutex::new(Memory {
                extension_version: extension_version.map(str::to_owned),
                ..Memory::default()
            })),
        }
    }

    fn inject_extended_reply(&self, injection: ExtendedReplyInjection) {
        self.state.lock().unwrap().extended_reply_injection = Some(injection);
    }

    fn insert_target(&self) {
        self.state
            .lock()
            .unwrap()
            .files
            .insert("/target".into(), b"original".to_vec());
    }

    async fn manager(&self) -> SftpManager {
        let (client, stream) = duplex(256 * 1024);
        russh_sftp::server::run(stream, Peer(self.state.clone())).await;
        SftpManager::new(SftpSession::new(client).await.unwrap())
    }
}

async fn finish(fixture: Fixture, managers: Vec<SftpManager>) {
    for manager in managers {
        timeout(Duration::from_secs(2), manager.shutdown())
            .await
            .expect("SFTP shutdown exceeded deadline");
    }
    timeout(Duration::from_secs(5), async {
        while Arc::strong_count(&fixture.state) != 1 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("protocol peer did not release shared memory after shutdown");
}

fn stage(state: &Arc<Mutex<Memory>>, index: usize) -> String {
    state.lock().unwrap().opened[index].clone()
}

#[tokio::test]
async fn overwrite_uses_posix_rename_after_closed_stage_and_preserves_binary_bytes() {
    let fixture = Fixture::new();
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/target".into(), b"old".to_vec());
    let mut manager = fixture.manager().await;
    let upload = manager
        .open_upload("/target", Some(5), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let temp = stage(&fixture.state, 0);
    manager
        .write(&upload.id, &[0, 255, 1, 128, 0])
        .await
        .unwrap();
    let result = manager.close(&upload.id).await.unwrap();
    assert_eq!(result.state, "completed");
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.files["/target"], [0, 255, 1, 128, 0]);
    assert!(!state.files.contains_key(&temp));
    assert!(!state.events.iter().any(|event| event == "remove:/target"));
    let close = state
        .events
        .iter()
        .position(|event| event == &format!("close:{temp}"))
        .unwrap();
    let rename = state
        .events
        .iter()
        .position(|event| event.starts_with("extended:"))
        .unwrap();
    assert!(close < rename, "the stage handle must close before commit");
    assert!(!state
        .events
        .iter()
        .any(|event| event.starts_with("rename:") || event.starts_with("remove:")));
    drop(state);
    finish(fixture, vec![manager]).await;
}

#[tokio::test]
async fn overwrite_handles_known_zero_and_unknown_size_existing_destinations() {
    let fixture = Fixture::new();
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/known".into(), b"previous".to_vec());
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/unknown".into(), b"previous".to_vec());
    let mut manager = fixture.manager().await;
    let zero = manager
        .open_upload("/known", Some(0), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    assert_eq!(manager.close(&zero.id).await.unwrap().state, "completed");
    let unknown = manager
        .open_upload("/unknown", None, SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    manager.write(&unknown.id, &[0, 44, 255]).await.unwrap();
    assert_eq!(manager.close(&unknown.id).await.unwrap().state, "completed");
    let state = fixture.state.lock().unwrap();
    assert!(state.files["/known"].is_empty());
    assert_eq!(state.files["/unknown"], [0, 44, 255]);
    assert_eq!(
        state
            .events
            .iter()
            .filter(|event| event.starts_with("extended:"))
            .count(),
        2
    );
    drop(state);
    finish(fixture, vec![manager]).await;
}

#[tokio::test]
async fn posix_permission_failure_keeps_destination_and_only_cleans_own_stage() {
    let fixture = Fixture::new();
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/target".into(), b"original".to_vec());
    let mut first = fixture.manager().await;
    let mut second = fixture.manager().await;
    let a = first
        .open_upload("/target", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let a_stage = stage(&fixture.state, 0);
    let b = second
        .open_upload("/target", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let b_stage = stage(&fixture.state, 1);
    first.write(&a.id, &[7]).await.unwrap();
    second.write(&b.id, &[8]).await.unwrap();
    second.stat(&b_stage, false).await.unwrap();
    fixture.state.lock().unwrap().fail_posix_rename = true;
    assert!(first.close(&a.id).await.is_err());
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["/target"], b"original");
        assert!(!state.files.contains_key(&a_stage));
        assert_eq!(state.files[&b_stage], [8]);
        assert!(!state
            .events
            .iter()
            .any(|event| event == "remove:/target" || event.starts_with("rename:")));
        assert!(state
            .events
            .iter()
            .any(|event| event.starts_with("extended:")));
    }
    second.close(&b.id).await.unwrap();
    finish(fixture, vec![first, second]).await;
}

#[tokio::test]
async fn skip_and_rename_policy_never_replace_a_competing_destination() {
    let fixture = Fixture::new();
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/target".into(), b"original".to_vec());
    let mut manager = fixture.manager().await;
    assert!(manager
        .open_upload("/target", Some(1), SftpOverwritePolicy::Skip)
        .await
        .is_err());
    let renamed = manager
        .open_upload("/target", Some(1), SftpOverwritePolicy::Rename)
        .await
        .unwrap();
    let temp = stage(&fixture.state, 0);
    manager.write(&renamed.id, &[9]).await.unwrap();
    // The available-name STAT and stage OPEN have completed; another session
    // now occupies the selected final path before this upload commits.
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/target (1)".into(), b"racing file".to_vec());
    assert!(manager.close(&renamed.id).await.is_err());
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.files["/target"], b"original");
    assert_eq!(state.files["/target (1)"], b"racing file");
    assert!(state
        .events
        .iter()
        .any(|event| event.starts_with("rename:")));
    assert!(!state
        .events
        .iter()
        .any(|event| event.starts_with("extended:")));
    assert!(!state.events.iter().any(|event| event == "remove:/target"));
    assert!(!state.files.contains_key(&temp));
    drop(state);
    finish(fixture, vec![manager]).await;
}

#[tokio::test]
async fn incomplete_cancel_and_shutdown_keep_destination_until_commit() {
    let fixture = Fixture::new();
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/incomplete".into(), b"old-a".to_vec());
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/cancel".into(), b"old-b".to_vec());
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/shutdown".into(), b"old-c".to_vec());
    let mut manager = fixture.manager().await;
    let short = manager
        .open_upload("/incomplete", Some(2), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let short_stage = stage(&fixture.state, 0);
    manager.write(&short.id, &[1]).await.unwrap();
    assert!(manager.close(&short.id).await.is_err());
    let cancelled = manager
        .open_upload("/cancel", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let cancel_stage = stage(&fixture.state, 1);
    manager.write(&cancelled.id, &[2]).await.unwrap();
    manager.cancel(&cancelled.id).await.unwrap();
    let pending = manager
        .open_upload("/shutdown", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let shutdown_stage = stage(&fixture.state, 2);
    manager.write(&pending.id, &[3]).await.unwrap();
    timeout(Duration::from_secs(2), manager.shutdown())
        .await
        .expect("manager shutdown exceeded deadline");
    let state = fixture.state.lock().unwrap();
    for (path, bytes) in [
        ("/incomplete", &b"old-a"[..]),
        ("/cancel", &b"old-b"[..]),
        ("/shutdown", &b"old-c"[..]),
    ] {
        assert_eq!(state.files[path], bytes);
    }
    for temp in [short_stage, cancel_stage, shutdown_stage] {
        assert!(!state.files.contains_key(&temp));
    }
    assert!(!state
        .events
        .iter()
        .any(|event| event.starts_with("extended:") || event.starts_with("rename:")));
    drop(state);
    finish(fixture, Vec::new()).await;
}

#[tokio::test]
async fn posix_rename_is_not_sent_when_extension_is_absent_or_version_mismatches() {
    for extension_version in [None, Some("2")] {
        let fixture = Fixture::with_extension(extension_version);
        fixture.insert_target();
        let manager = fixture.manager().await;
        assert!(!manager
            .session()
            .posix_rename("/stage", "/target")
            .await
            .unwrap());
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["/target"], b"original");
        assert!(!state
            .events
            .iter()
            .any(|event| event.starts_with("extended:")));
        drop(state);
        finish(fixture, vec![manager]).await;
    }
}

#[tokio::test]
async fn posix_rename_preserves_op_unsupported_status_error() {
    let fixture = Fixture::new();
    fixture.insert_target();
    fixture.inject_extended_reply(ExtendedReplyInjection::Status(StatusCode::OpUnsupported));
    let manager = fixture.manager().await;
    let error = manager
        .session()
        .posix_rename("/stage", "/target")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SftpClientError::Status(status) if status.status_code == StatusCode::OpUnsupported
    ));
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.files["/target"], b"original");
    assert_eq!(
        state
            .events
            .iter()
            .filter(|event| event.starts_with("extended:"))
            .count(),
        1
    );
    drop(state);
    finish(fixture, vec![manager]).await;
}

#[tokio::test]
async fn posix_rename_rejects_unexpected_reply_without_changing_destination() {
    let fixture = Fixture::new();
    fixture.insert_target();
    fixture.inject_extended_reply(ExtendedReplyInjection::UnexpectedPacket);
    let manager = fixture.manager().await;
    let error = manager
        .session()
        .posix_rename("/stage", "/target")
        .await
        .unwrap_err();
    assert!(matches!(error, SftpClientError::UnexpectedPacket));
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.files["/target"], b"original");
    assert_eq!(
        state
            .events
            .iter()
            .filter(|event| event.starts_with("extended:"))
            .count(),
        1
    );
    drop(state);
    finish(fixture, vec![manager]).await;
}
