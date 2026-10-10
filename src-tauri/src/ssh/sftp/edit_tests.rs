use super::{SftpManager, SftpOverwritePolicy};
use crate::transfer::{file_edit::FileEditManager, manager::TransferManager};
use russh_sftp::{
    client::SftpSession,
    protocol::{Attrs, Data, FileAttributes, FileMode, Handle, OpenFlags, Status, StatusCode},
    server::{Handler, StatusReply},
};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};

#[derive(Default)]
struct Memory {
    files: HashMap<String, (Vec<u8>, u32)>,
    handles: HashMap<String, String>,
    seq: u64,
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
impl Handler for Peer {
    type Error = StatusReply;
    fn unimplemented(&self) -> Self::Error {
        StatusReply::new(StatusCode::OpUnsupported)
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let state = self.0.lock().unwrap();
        let (bytes, mode) = state
            .files
            .get(&path)
            .ok_or_else(|| StatusReply::new(StatusCode::NoSuchFile))?;
        let mut attrs = FileAttributes {
            size: Some(bytes.len() as u64),
            permissions: Some(*mode),
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
            state
                .files
                .entry(filename.clone())
                .or_insert((vec![], 0o644));
        }
        if !state.files.contains_key(&filename) {
            return Err(StatusReply::new(StatusCode::NoSuchFile));
        }
        if flags.contains(OpenFlags::TRUNCATE) {
            state.files.get_mut(&filename).unwrap().0.clear();
        }
        state.seq += 1;
        let handle = format!("h{}", state.seq);
        state.handles.insert(handle.clone(), filename);
        Ok(Handle { id, handle })
    }
    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let state = self.0.lock().unwrap();
        let path = &state.handles[&handle];
        let bytes = &state.files[path].0;
        if offset as usize >= bytes.len() {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        Ok(Data {
            id,
            data: bytes[offset as usize..bytes.len().min(offset as usize + len as usize)].to_vec(),
        })
    }
    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        let path = state.handles[&handle].clone();
        let target = &mut state.files.get_mut(&path).unwrap().0;
        target.resize(target.len().max(offset as usize + bytes.len()), 0);
        target[offset as usize..offset as usize + bytes.len()].copy_from_slice(&bytes);
        Ok(ok(id))
    }
    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.0.lock().unwrap().handles.remove(&handle);
        Ok(ok(id))
    }
    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        let file = state
            .files
            .remove(&from)
            .ok_or_else(|| StatusReply::new(StatusCode::NoSuchFile))?;
        state.files.insert(to, file);
        Ok(ok(id))
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.0.lock().unwrap().files.remove(&path);
        Ok(ok(id))
    }
    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        let file = state
            .files
            .get_mut(&path)
            .ok_or_else(|| StatusReply::new(StatusCode::PermissionDenied))?;
        assert!(
            attrs.size.is_none(),
            "chmod must not truncate the remote file"
        );
        file.1 = attrs.permissions.unwrap();
        Ok(ok(id))
    }
}
async fn connect(state: Arc<Mutex<Memory>>) -> SftpManager {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        russh_sftp::server::run(stream, Peer(state)).await;
    });
    SftpManager::new(
        SftpSession::new(TcpStream::connect(address).await.unwrap())
            .await
            .unwrap(),
    )
}
#[tokio::test]
async fn actual_editor_file_watch_streams_binary_save_back_and_restores_remote_mode() {
    let state = Arc::new(Mutex::new(Memory::default()));
    state
        .lock()
        .unwrap()
        .files
        .insert("/note.bin".into(), (vec![0, 255, 128, 2], 0o640));
    let mut remote = connect(state.clone()).await;
    let local = TransferManager::default();
    let edits = FileEditManager::default();
    let edit = edits.prepare("note.bin", 0o100640, 4, &local).unwrap();
    let download = remote.open_download("/note.bin").await.unwrap();
    loop {
        let (bytes, _) = remote.read(&download.id, 2).await.unwrap();
        if bytes.is_empty() {
            break;
        }
        local.write(&edit.transfer.id, &bytes).unwrap();
    }
    remote.close(&download.id).await.unwrap();
    local.close(&edit.transfer.id).unwrap();
    edits.ready(&edit.id).unwrap();
    let (send, receive) = mpsc::channel();
    edits
        .watch(&edit.id, move |event| {
            let _ = send.send(event);
        })
        .unwrap();
    let saved = [255, 0, 11, 128, 8, 9, 0];
    std::fs::write(&edit.path, saved).unwrap();
    assert_eq!(
        receive.recv_timeout(Duration::from_secs(8)).unwrap().event,
        "change"
    );
    let upload = local.open_upload(&[edit.path.clone()]).unwrap().remove(0);
    let sending = remote
        .open_upload(
            "/note.bin",
            Some(saved.len() as u64),
            SftpOverwritePolicy::Overwrite,
        )
        .await
        .unwrap();
    loop {
        let (bytes, _) = local.read(&upload.id, 2).unwrap();
        if bytes.is_empty() {
            break;
        }
        remote.write(&sending.id, &bytes).await.unwrap();
    }
    remote.close(&sending.id).await.unwrap();
    local.close(&upload.id).unwrap();
    remote.chmod("/note.bin", 0o100640).await.unwrap();
    assert_eq!(
        state.lock().unwrap().files["/note.bin"],
        (saved.to_vec(), 0o640)
    );
    assert!(remote.chmod("/denied.bin", 0o600).await.is_err());
    edits.stop(&edit.id).unwrap();
    assert_eq!(std::fs::read(&edit.path).unwrap(), saved);
    remote.shutdown().await;
    std::fs::remove_dir_all(std::path::Path::new(&edit.path).parent().unwrap()).unwrap();
}
