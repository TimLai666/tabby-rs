use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::transfer::manager::TransferManager;
use russh_sftp::{
    client::SftpSession,
    protocol::{
        Attrs, Data, File, FileAttributes, FileMode, Handle, Name, OpenFlags, Status, StatusCode,
    },
    server::{Handler, StatusReply},
};
use tokio::{io::duplex, time::timeout};

use super::SftpManager;

const BINARY: &[u8] = &[0x00, 0xff, 0x01, 0x80, 0x00];

#[derive(Clone, Copy)]
enum Entry {
    Dir,
    File(&'static [u8]),
    Link(&'static str),
}

fn lookup(path: &str) -> Option<Entry> {
    match path {
        "/data" => Some(Entry::Dir),
        "/data/unknown.bin" => Some(Entry::File(BINARY)),
        "/data/empty.bin" => Some(Entry::File(&[])),
        "/data/unknown-link.bin" => Some(Entry::Link("unknown.bin")),
        _ => None,
    }
}

fn resolve(path: &str) -> Option<Entry> {
    match lookup(path)? {
        Entry::Link(target) => resolve(&format!("/data/{target}")),
        entry => Some(entry),
    }
}

fn attrs(entry: Entry, follow: bool) -> FileAttributes {
    match if follow {
        resolve_entry(entry)
    } else {
        Some(entry)
    }
    .unwrap()
    {
        Entry::Dir => {
            let mut attrs = FileAttributes {
                size: Some(0),
                permissions: Some(0o755),
                ..Default::default()
            };
            attrs.set_type(FileMode::DIR);
            attrs
        }
        Entry::File(bytes) => {
            let mut attrs = FileAttributes {
                size: if bytes.is_empty() { Some(0) } else { None },
                permissions: Some(0o644),
                ..Default::default()
            };
            attrs.set_type(FileMode::REG);
            attrs
        }
        Entry::Link(target) => {
            let mut attrs = FileAttributes {
                size: Some(target.len() as u64),
                permissions: Some(0o777),
                ..Default::default()
            };
            attrs.set_type(FileMode::LNK);
            attrs
        }
    }
}

fn resolve_entry(entry: Entry) -> Option<Entry> {
    match entry {
        Entry::Link(target) => resolve(&format!("/data/{target}")),
        other => Some(other),
    }
}

#[derive(Default)]
struct State {
    handles: HashMap<String, String>,
    next: u64,
    directory_read: bool,
}

struct Peer {
    state: Arc<Mutex<State>>,
    _lifetime: Arc<()>,
}

impl Peer {
    fn handle(&self, path: &str) -> String {
        let mut state = self.state.lock().unwrap();
        state.next += 1;
        let handle = format!("h{}", state.next);
        state.handles.insert(handle.clone(), path.to_owned());
        handle
    }
}

impl Handler for Peer {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusReply::new(StatusCode::OpUnsupported)
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        if path != "/data" {
            return Err(StatusReply::new(StatusCode::NoSuchFile));
        }
        self.state.lock().unwrap().directory_read = false;
        Ok(Handle { id, handle: path })
    }

    async fn readdir(&mut self, id: u32, _handle: String) -> Result<Name, Self::Error> {
        let mut state = self.state.lock().unwrap();
        if std::mem::replace(&mut state.directory_read, true) {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        Ok(Name {
            id,
            files: vec![
                File::new("unknown.bin", attrs(Entry::File(BINARY), false)),
                File::new("empty.bin", attrs(Entry::File(&[]), false)),
                File::new("unknown-link.bin", attrs(Entry::Link("unknown.bin"), false)),
            ],
        })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        _flags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if !matches!(resolve(&filename), Some(Entry::File(_))) {
            return Err(StatusReply::new(StatusCode::NoSuchFile));
        }
        Ok(Handle {
            id,
            handle: self.handle(&filename),
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let path = self.state.lock().unwrap().handles.get(&handle).cloned();
        let Some(Entry::File(bytes)) = path.as_deref().and_then(resolve) else {
            return Err(StatusReply::new(StatusCode::Failure));
        };
        let start = offset as usize;
        if start >= bytes.len() {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        Ok(Data {
            id,
            data: bytes[start..bytes.len().min(start + len as usize)].to_vec(),
        })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.state.lock().unwrap().handles.remove(&handle);
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".into(),
            language_tag: "en-US".into(),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let Some(entry) = lookup(&path) else {
            return Err(StatusReply::new(StatusCode::NoSuchFile));
        };
        Ok(Attrs {
            id,
            attrs: attrs(entry, false),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let Some(entry) = lookup(&path).and_then(resolve_entry) else {
            return Err(StatusReply::new(StatusCode::NoSuchFile));
        };
        Ok(Attrs {
            id,
            attrs: attrs(entry, true),
        })
    }
}

struct Fixture {
    manager: SftpManager,
    peer_lifetime: Arc<()>,
    peer_task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let peer_lifetime = Arc::new(());
        let (client, server) = duplex(256 * 1024);
        let peer = Peer {
            state: Arc::clone(&state),
            _lifetime: Arc::clone(&peer_lifetime),
        };
        let peer_task = tokio::spawn(async move {
            russh_sftp::server::run(server, peer).await;
        });
        let session = SftpSession::new(client)
            .await
            .expect("create SFTP client session");
        Self {
            manager: SftpManager::new(session),
            peer_lifetime,
            peer_task,
        }
    }

    async fn shutdown(self) {
        let Self {
            manager,
            peer_lifetime,
            peer_task,
        } = self;
        timeout(Duration::from_secs(2), manager.shutdown())
            .await
            .expect("manager shutdown timed out");
        timeout(Duration::from_secs(2), peer_task)
            .await
            .expect("peer shutdown timed out")
            .expect("peer task panicked");
        timeout(Duration::from_secs(2), async move {
            while Arc::strong_count(&peer_lifetime) != 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("SFTP peer handler did not stop after session shutdown");
    }
}

fn serialized_size<T: serde::Serialize>(value: &T) -> Option<u64> {
    serde_json::to_value(value).expect("serialize SFTP result")["size"].as_u64()
}

#[tokio::test]
async fn missing_binary_size_is_json_null_and_known_empty_stays_zero() {
    timeout(Duration::from_secs(8), async {
        let fixture = Fixture::start().await;
        let entries = fixture
            .manager
            .list("/data")
            .await
            .expect("list fixture directory");
        let unknown = entries
            .iter()
            .find(|entry| entry.name == "unknown.bin")
            .unwrap();
        let empty = entries
            .iter()
            .find(|entry| entry.name == "empty.bin")
            .unwrap();
        let listing_unknown_is_null = serde_json::to_value(unknown).unwrap()["size"].is_null();
        let listing_empty_is_zero = serialized_size(empty) == Some(0);

        let unknown_lstat = fixture
            .manager
            .stat("/data/unknown.bin", false)
            .await
            .unwrap();
        let lstat_unknown_is_null = serde_json::to_value(&unknown_lstat).unwrap()["size"].is_null();
        let unknown_followed = fixture
            .manager
            .stat("/data/unknown-link.bin", true)
            .await
            .unwrap();
        let followed_unknown_is_null =
            serde_json::to_value(&unknown_followed).unwrap()["size"].is_null();
        let known_empty = fixture.manager.stat("/data/empty.bin", true).await.unwrap();
        let stat_empty_is_zero = serialized_size(&known_empty) == Some(0);
        fixture.shutdown().await;
        let checks = [
            ("listing unknown size is null", listing_unknown_is_null),
            ("listing known empty size is zero", listing_empty_is_zero),
            ("unfollowed unknown size is null", lstat_unknown_is_null),
            ("followed unknown size is null", followed_unknown_is_null),
            ("stat known empty size is zero", stat_empty_is_zero),
        ];
        assert!(
            checks.iter().all(|(_, passed)| *passed),
            "metadata checks failed: {checks:?}"
        );
    })
    .await
    .expect("metadata regression exceeded test bound");
}

#[tokio::test]
async fn unknown_binary_direct_and_link_downloads_reach_local_filesystem_exactly() {
    timeout(Duration::from_secs(10), async {
        let mut fixture = Fixture::start().await;
        let local = TransferManager::default();
        let root = tempfile::tempdir().unwrap();
        let mut failures = Vec::new();
        for (remote_path, output) in [
            ("/data/unknown.bin", "direct.bin"),
            ("/data/unknown-link.bin", "link.bin"),
        ] {
            let remote = fixture
                .manager
                .open_download(remote_path)
                .await
                .expect("open unknown-sized remote file");
            let advertised = fixture
                .manager
                .stat(remote_path, true)
                .await
                .expect("stat the advertised direct file or followed link target");
            let wire_size = serialized_size(&advertised);
            let destination = root.path().join(output);
            let local_transfer = local
                .open_download(
                    &advertised.name,
                    0o644,
                    wire_size,
                    destination.to_str().unwrap(),
                    None,
                    None,
                )
                .unwrap();
            let mut received = Vec::new();
            let mut read_error = None;
            let mut write_error = None;
            loop {
                let (bytes, _) = match fixture.manager.read(&remote.id, 2).await {
                    Ok(result) => result,
                    Err(error) => {
                        read_error = Some(error.to_string());
                        break;
                    }
                };
                if bytes.is_empty() {
                    break;
                }
                received.extend_from_slice(&bytes);
                if write_error.is_none() {
                    if let Err(error) = local.write(&local_transfer.id, &bytes) {
                        write_error = Some(error.to_string());
                    }
                }
            }
            let remote_cleanup_ok = if read_error.is_some() {
                fixture.manager.cancel(&remote.id).await.is_ok()
            } else {
                fixture.manager.close(&remote.id).await.is_ok()
            };
            let local_cancelled = read_error.is_some() || write_error.is_some();
            let local_close_ok = if local_cancelled {
                local.cancel(&local_transfer.id).is_ok()
            } else {
                local.close(&local_transfer.id).is_ok()
            };
            let persisted_bytes = std::fs::read(&destination).ok();
            let passed = received == BINARY
                && read_error.is_none()
                && write_error.is_none()
                && remote_cleanup_ok
                && local_close_ok
                && persisted_bytes.as_deref() == Some(BINARY);
            if !passed {
                failures.push(format!(
                    "{remote_path}: advertised_size={wire_size:?}, bytes={received:?}, read_error={read_error:?}, write_error={write_error:?}, remote_cleanup_ok={remote_cleanup_ok}, local_cleanup_ok={local_close_ok}, persisted={persisted_bytes:?}"
                ));
            }
        }
        fixture.shutdown().await;
        assert!(failures.is_empty(), "unknown-size downloads failed: {failures:?}");
    })
    .await
    .expect("binary download regression exceeded test bound");
}

#[tokio::test]
async fn cancelling_unknown_download_cleans_stage_and_preserves_destination() {
    timeout(Duration::from_secs(8), async {
        let mut fixture = Fixture::start().await;
        let local = TransferManager::default();
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("existing.bin");
        std::fs::write(&destination, b"original destination").unwrap();
        let remote = fixture
            .manager
            .open_download("/data/unknown.bin")
            .await
            .unwrap();
        let local_transfer = local
            .open_download(
                &remote.name,
                0o644,
                serialized_size(&remote),
                destination.to_str().unwrap(),
                None,
                None,
            )
            .unwrap();
        let (first, _) = fixture.manager.read(&remote.id, 2).await.unwrap();
        assert_eq!(first, &BINARY[..2]);
        local.write(&local_transfer.id, &first).unwrap();
        local.cancel(&local_transfer.id).unwrap();
        fixture.manager.cancel(&remote.id).await.unwrap();
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"original destination"
        );
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            1,
            "cancelled local staging file must be removed"
        );
        fixture.shutdown().await;
    })
    .await
    .expect("cancellation cleanup exceeded test bound");
}

#[tokio::test]
async fn known_empty_download_control_commits_empty_file() {
    timeout(Duration::from_secs(8), async {
        let mut fixture = Fixture::start().await;
        let local = TransferManager::default();
        let root = tempfile::tempdir().unwrap();
        let remote = fixture
            .manager
            .open_download("/data/empty.bin")
            .await
            .unwrap();
        assert_eq!(serialized_size(&remote), Some(0));
        let destination = root.path().join("empty.bin");
        let local_transfer = local
            .open_download(
                &remote.name,
                0o644,
                serialized_size(&remote),
                destination.to_str().unwrap(),
                None,
                None,
            )
            .unwrap();
        let (bytes, _) = fixture.manager.read(&remote.id, 2).await.unwrap();
        assert!(bytes.is_empty());
        fixture.manager.close(&remote.id).await.unwrap();
        local.close(&local_transfer.id).unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"");
        fixture.shutdown().await;
    })
    .await
    .expect("known-empty control exceeded test bound");
}

#[test]
fn file_edit_prepare_request_accepts_optional_size_and_rejects_invalid_numbers() {
    use serde_json::json;

    let cases = [
        (
            "explicit null size",
            json!({ "name": "unknown.bin", "mode": 0o644, "size": null }),
            true,
        ),
        (
            "missing size",
            json!({ "name": "unknown.bin", "mode": 0o644 }),
            true,
        ),
        (
            "known empty size",
            json!({ "name": "empty.bin", "mode": 0o644, "size": 0 }),
            true,
        ),
        (
            "known positive size",
            json!({ "name": "binary.bin", "mode": 0o644, "size": 5 }),
            true,
        ),
        (
            "negative size",
            json!({ "name": "invalid.bin", "mode": 0o644, "size": -1 }),
            false,
        ),
        (
            "fractional size",
            json!({ "name": "invalid.bin", "mode": 0o644, "size": 1.5 }),
            false,
        ),
    ];
    let checks: Vec<_> = cases
        .iter()
        .map(|(label, value, expected_ok)| {
            let actual_ok =
                serde_json::from_value::<crate::commands::file_edit::PrepareRequest>(value.clone())
                    .is_ok();
            (*label, actual_ok, *expected_ok)
        })
        .collect();
    assert!(
        checks
            .iter()
            .all(|(_, actual, expected)| actual == expected),
        "PrepareRequest JSON deserialization results: {checks:?}"
    );
}
