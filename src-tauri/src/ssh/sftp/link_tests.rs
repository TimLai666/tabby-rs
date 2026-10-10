use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use russh_sftp::{
    client::SftpSession,
    protocol::{
        Attrs, Data, File, FileAttributes, FileMode, Handle, Name, OpenFlags, Status, StatusCode,
    },
    server::{Handler, StatusReply},
};
use tokio::{
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};

use super::{backend::SftpBackend, manager::SftpManager, model::SftpPathRequest};
use crate::ssh::model::SshError;

const BINARY_PAYLOAD: &[u8] = &[0x00, 0xff, 0x01, 0x80, b'h', 0x00];

#[derive(Clone, Copy)]
enum Kind {
    Dir,
    File(&'static [u8]),
    Link(&'static str),
}

fn lookup(path: &str) -> Option<Kind> {
    match path {
        "/data" | "/data/dir" => Some(Kind::Dir),
        "/data/file.txt" | "/data/unsized.txt" => Some(Kind::File(BINARY_PAYLOAD)),
        "/data/link-rel.txt" => Some(Kind::Link("file.txt")),
        "/data/link-abs.txt" => Some(Kind::Link("/data/file.txt")),
        "/data/link-dir-rel" => Some(Kind::Link("dir")),
        "/data/link-dir-abs" => Some(Kind::Link("/data/dir")),
        "/data/dangling" => Some(Kind::Link("missing.txt")),
        _ => None,
    }
}

fn resolve(path: &str) -> Option<Kind> {
    let mut current = path.to_owned();
    for _ in 0..16 {
        match lookup(&current)? {
            Kind::Link(target) => {
                current = if target.starts_with('/') {
                    target.to_owned()
                } else {
                    let parent = current
                        .rsplit_once('/')
                        .map(|(parent, _)| parent)
                        .unwrap_or("");
                    format!("{parent}/{target}")
                };
            }
            kind => return Some(kind),
        }
    }
    None
}

fn regular_attrs(size: u64) -> FileAttributes {
    let mut attrs = FileAttributes {
        size: Some(size),
        permissions: Some(0o644),
        ..Default::default()
    };
    attrs.set_type(FileMode::REG);
    attrs
}

fn dir_attrs() -> FileAttributes {
    let mut attrs = FileAttributes {
        size: Some(0),
        permissions: Some(0o755),
        ..Default::default()
    };
    attrs.set_type(FileMode::DIR);
    attrs
}

fn link_attrs(size: u64) -> FileAttributes {
    let mut attrs = FileAttributes {
        size: Some(size),
        permissions: Some(0o777),
        ..Default::default()
    };
    attrs.set_type(FileMode::LNK);
    attrs
}

fn lstat_attrs(path: &str) -> Option<FileAttributes> {
    match lookup(path)? {
        Kind::Dir => Some(dir_attrs()),
        Kind::File(bytes) => {
            let mut attrs = regular_attrs(bytes.len() as u64);
            if path == "/data/unsized.txt" {
                attrs.size = None;
            }
            Some(attrs)
        }
        Kind::Link(target) => Some(link_attrs(target.len() as u64)),
    }
}

fn stat_attrs(path: &str) -> Option<FileAttributes> {
    match resolve(path)? {
        Kind::Dir => Some(dir_attrs()),
        Kind::File(bytes) => {
            let mut attrs = regular_attrs(bytes.len() as u64);
            if path == "/data/unsized.txt" {
                attrs.size = None;
            }
            Some(attrs)
        }
        Kind::Link(_) => None,
    }
}

#[derive(Default)]
struct PeerState {
    requests: Vec<String>,
    handles: HashMap<String, String>,
    next_handle: u64,
}

struct LinkPeer {
    state: Arc<Mutex<PeerState>>,
    _marker: Arc<()>,
}

impl LinkPeer {
    fn record(&self, entry: String) {
        self.state.lock().unwrap().requests.push(entry);
    }

    fn allocate_handle(&self, path: &str) -> String {
        let mut state = self.state.lock().unwrap();
        state.next_handle += 1;
        let handle = format!("handle-{}", state.next_handle);
        state.handles.insert(handle.clone(), path.to_owned());
        handle
    }
}

impl Handler for LinkPeer {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusReply::new(StatusCode::OpUnsupported)
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        _pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        self.record(format!("open:{filename}"));
        match resolve(&filename) {
            Some(Kind::File(_)) => Ok(Handle {
                id,
                handle: self.allocate_handle(&filename),
            }),
            _ => Err(StatusReply::new(StatusCode::NoSuchFile)),
        }
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        self.record(format!("read:{handle}@{offset}"));
        let path = {
            let state = self.state.lock().unwrap();
            state.handles.get(&handle).cloned()
        };
        let Some(Kind::File(bytes)) = path.as_deref().and_then(resolve) else {
            return Err(StatusReply::new(StatusCode::Failure));
        };
        let start = offset as usize;
        if start >= bytes.len() {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        let end = start.saturating_add(len as usize).min(bytes.len());
        Ok(Data {
            id,
            data: bytes[start..end].to_vec(),
        })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.record(format!("close:{handle}"));
        self.state.lock().unwrap().handles.remove(&handle);
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".into(),
            language_tag: "en-US".into(),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.record(format!("lstat:{path}"));
        match lstat_attrs(&path) {
            Some(attrs) => Ok(Attrs { id, attrs }),
            None => Err(StatusReply::new(StatusCode::NoSuchFile)),
        }
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.record(format!("stat:{path}"));
        match stat_attrs(&path) {
            Some(attrs) => Ok(Attrs { id, attrs }),
            None => Err(StatusReply::new(StatusCode::NoSuchFile)),
        }
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        self.record(format!("readlink:{path}"));
        match lookup(&path) {
            Some(Kind::Link(target)) => Ok(Name {
                id,
                files: vec![File::dummy(target)],
            }),
            _ => Err(StatusReply::new(StatusCode::NoSuchFile)),
        }
    }
}

struct PeerFixture {
    manager: SftpManager,
    state: Arc<Mutex<PeerState>>,
    marker: Arc<()>,
}

impl PeerFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind loopback SFTP peer");
        let address = listener.local_addr().expect("peer address");
        let state = Arc::new(Mutex::new(PeerState::default()));
        let marker = Arc::new(());
        let handler = LinkPeer {
            state: Arc::clone(&state),
            _marker: Arc::clone(&marker),
        };
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept SFTP client");
            drop(listener);
            russh_sftp::server::run(stream, handler).await;
        });
        let stream = TcpStream::connect(address)
            .await
            .expect("connect to loopback SFTP peer");
        let session = SftpSession::new(stream)
            .await
            .expect("start client SFTP session");
        Self {
            manager: SftpManager::new(session),
            state,
            marker,
        }
    }

    fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    fn open_handles(&self) -> usize {
        self.state.lock().unwrap().handles.len()
    }

    async fn wait_for_no_handles(&self) {
        timeout(Duration::from_secs(5), async {
            loop {
                if self.open_handles() == 0 {
                    return;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("peer file handle was not released");
    }

    async fn shutdown(self) {
        assert_eq!(
            Arc::strong_count(&self.marker),
            2,
            "the peer SFTP task must still be running before shutdown"
        );
        let Self {
            manager, marker, ..
        } = self;
        manager.shutdown().await;
        timeout(Duration::from_secs(5), async move {
            loop {
                if Arc::strong_count(&marker) == 1 {
                    return;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("peer SFTP task did not stop after the client session closed");
    }
}

#[tokio::test]
async fn backend_readlink_returns_raw_relative_target() {
    let fixture = PeerFixture::start().await;
    let target = fixture
        .manager
        .backend
        .readlink("/data/link-rel.txt")
        .await
        .expect("backend.readlink must return the raw relative link target");
    assert_eq!(target, "file.txt");
    assert!(fixture
        .requests()
        .iter()
        .any(|entry| entry == "readlink:/data/link-rel.txt"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn backend_readlink_returns_raw_absolute_target() {
    let fixture = PeerFixture::start().await;
    let target = fixture
        .manager
        .backend
        .readlink("/data/link-abs.txt")
        .await
        .expect("backend.readlink must return the raw absolute link target");
    assert_eq!(target, "/data/file.txt");
    assert!(fixture
        .requests()
        .iter()
        .any(|entry| entry == "readlink:/data/link-abs.txt"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn backend_readlink_propagates_peer_status_unchanged() {
    let fixture = PeerFixture::start().await;
    let error = fixture
        .manager
        .backend
        .readlink("/data/file.txt")
        .await
        .expect_err("readlink on a regular file must surface the peer status");
    assert!(
        matches!(error, SshError::Sftp(ref message) if message.contains("No such file")),
        "unexpected error: {error:?}"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn backend_readlink_rejects_malformed_paths_without_remote_operations() {
    let fixture = PeerFixture::start().await;
    let error = fixture
        .manager
        .backend
        .readlink("/data/link\u{0}.txt")
        .await
        .expect_err("readlink must reject a NUL byte path");
    assert!(
        matches!(error, SshError::InvalidRequest(_)),
        "unexpected error: {error:?}"
    );
    assert!(
        fixture.requests().is_empty(),
        "no packet may be sent for a malformed path: {:?}",
        fixture.requests()
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn open_download_follows_relative_file_link_binary_payload() {
    let mut fixture = PeerFixture::start().await;
    let descriptor = fixture
        .manager
        .open_download("/data/link-rel.txt")
        .await
        .expect("open_download must follow a relative file link");
    assert_eq!(descriptor.name, "link-rel.txt");
    let (data, state) = fixture
        .manager
        .read(&descriptor.id, 4096)
        .await
        .expect("read the followed relative file link");
    assert_eq!(data.as_slice(), BINARY_PAYLOAD);
    assert_eq!(state.state, "running");
    let (tail, state) = fixture
        .manager
        .read(&descriptor.id, 4096)
        .await
        .expect("read the followed relative file link to EOF");
    assert!(tail.is_empty());
    assert_eq!(state.state, "completed");
    let closed = fixture
        .manager
        .close(&descriptor.id)
        .await
        .expect("close the followed relative file link");
    assert_eq!(closed.state, "completed");
    fixture.wait_for_no_handles().await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn open_download_follows_absolute_file_link_binary_payload() {
    let mut fixture = PeerFixture::start().await;
    let descriptor = fixture
        .manager
        .open_download("/data/link-abs.txt")
        .await
        .expect("open_download must follow an absolute file link");
    assert_eq!(descriptor.name, "link-abs.txt");
    let (data, _) = fixture
        .manager
        .read(&descriptor.id, 4096)
        .await
        .expect("read the followed absolute file link");
    assert_eq!(data.as_slice(), BINARY_PAYLOAD);
    let closed = fixture
        .manager
        .close(&descriptor.id)
        .await
        .expect("close the followed absolute file link");
    assert_eq!(closed.state, "completed");
    fixture.wait_for_no_handles().await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn open_download_rejects_directory_link_before_opening() {
    let mut fixture = PeerFixture::start().await;
    for alias in ["/data/link-dir-rel", "/data/link-dir-abs"] {
        let result = fixture.manager.open_download(alias).await;
        assert!(
            result.is_err(),
            "a directory link must be rejected: {result:?}"
        );
    }
    let requests = fixture.requests();
    assert!(
        !requests.iter().any(|entry| entry.starts_with("open:")),
        "a directory link must be rejected before a file open: {requests:?}"
    );
    assert_eq!(fixture.open_handles(), 0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn open_download_rejects_dangling_link_before_opening() {
    let mut fixture = PeerFixture::start().await;
    let result = fixture.manager.open_download("/data/dangling").await;
    assert!(
        result.is_err(),
        "a dangling link must be rejected: {result:?}"
    );
    let requests = fixture.requests();
    assert!(
        !requests.iter().any(|entry| entry.starts_with("open:")),
        "a dangling link must be rejected before a file open: {requests:?}"
    );
    assert_eq!(fixture.open_handles(), 0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn open_download_reads_plain_file_exact_bytes_and_releases_handle() {
    let mut fixture = PeerFixture::start().await;
    let descriptor = fixture
        .manager
        .open_download("/data/file.txt")
        .await
        .expect("open_download must still open a plain file");
    assert_eq!(descriptor.name, "file.txt");
    assert_eq!(descriptor.size, Some(BINARY_PAYLOAD.len() as u64));
    let (data, state) = fixture
        .manager
        .read(&descriptor.id, 4096)
        .await
        .expect("read the plain file");
    assert_eq!(data.as_slice(), BINARY_PAYLOAD);
    assert_eq!(state.state, "running");
    let (tail, state) = fixture
        .manager
        .read(&descriptor.id, 4096)
        .await
        .expect("read the plain file to EOF");
    assert!(tail.is_empty());
    assert_eq!(state.state, "completed");
    let closed = fixture
        .manager
        .close(&descriptor.id)
        .await
        .expect("close the plain file");
    assert_eq!(closed.state, "completed");
    fixture.wait_for_no_handles().await;
    assert!(fixture
        .requests()
        .iter()
        .any(|entry| entry == "open:/data/file.txt"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn cancel_releases_the_peer_file_handle() {
    let mut fixture = PeerFixture::start().await;
    let descriptor = fixture
        .manager
        .open_download("/data/file.txt")
        .await
        .expect("open_download must still open a plain file");
    let _ = fixture
        .manager
        .read(&descriptor.id, 4096)
        .await
        .expect("read before cancelling");
    let cancelled = fixture
        .manager
        .cancel(&descriptor.id)
        .await
        .expect("cancel the download");
    assert_eq!(cancelled.state, "cancelled");
    fixture.wait_for_no_handles().await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn cancel_file_links_releases_the_peer_file_handle() {
    for alias in ["/data/link-rel.txt", "/data/link-abs.txt"] {
        let mut fixture = PeerFixture::start().await;
        let descriptor = fixture
            .manager
            .open_download(alias)
            .await
            .expect("open the file link before cancelling");
        let (data, _) = fixture
            .manager
            .read(&descriptor.id, 2)
            .await
            .expect("read part of the file link before cancelling");
        assert_eq!(data.as_slice(), &BINARY_PAYLOAD[..2]);
        let cancelled = fixture
            .manager
            .cancel(&descriptor.id)
            .await
            .expect("cancel the file-link download");
        assert_eq!(cancelled.state, "cancelled");
        fixture.wait_for_no_handles().await;
        assert!(fixture
            .requests()
            .iter()
            .any(|request| request.starts_with("close:")));
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn stat_rejects_malformed_paths_without_remote_operations() {
    let fixture = PeerFixture::start().await;
    let error = fixture
        .manager
        .stat("/data/link\u{0}.txt", false)
        .await
        .expect_err("stat must reject a NUL byte path");
    assert!(
        matches!(error, SshError::InvalidRequest(_)),
        "unexpected error: {error:?}"
    );
    assert!(
        fixture.requests().is_empty(),
        "no packet may be sent for a malformed path: {:?}",
        fixture.requests()
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn readlink_missing_ssh_session_errors_without_remote_operations() {
    let manager = crate::ssh::SshManager::new(std::path::PathBuf::from("blind-known-hosts"));
    let error = manager
        .sftp_readlink(SftpPathRequest {
            id: "missing-session".into(),
            path: "/data/link-rel.txt".into(),
        })
        .await
        .expect_err("an unknown SSH session must be rejected");
    assert!(
        matches!(error, SshError::InvalidRequest(_)),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn open_download_preserves_missing_size_without_claiming_zero_bytes() {
    let mut fixture = PeerFixture::start().await;
    let descriptor = fixture
        .manager
        .open_download("/data/unsized.txt")
        .await
        .expect("open a regular file whose peer omits the size attribute");
    assert_eq!(descriptor.name, "unsized.txt");
    assert_eq!(descriptor.size, None, "unknown size must remain unknown");
    let (data, _) = fixture.manager.read(&descriptor.id, 4096).await.unwrap();
    assert_eq!(data.as_slice(), BINARY_PAYLOAD);
    fixture.manager.close(&descriptor.id).await.unwrap();
    fixture.wait_for_no_handles().await;
    fixture.shutdown().await;
}
