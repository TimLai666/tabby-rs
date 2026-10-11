// Protocol regressions for concurrent SFTP upload ownership.
use super::{SftpManager, SftpOverwritePolicy};
use russh_sftp::{
    client::SftpSession,
    protocol::{Attrs, Data, FileAttributes, FileMode, Handle, OpenFlags, Status, StatusCode},
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

#[derive(Default)]
struct Memory {
    files: HashMap<String, Vec<u8>>,
    handles: HashMap<String, String>,
    opened: Vec<String>,
    sequence: u64,
    collide_next_open: bool,
    collided_path: Option<String>,
    fail_rename: bool,
    max_component: Option<usize>,
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
        if state
            .max_component
            .is_some_and(|limit| filename.rsplit('/').next().unwrap_or(&filename).len() > limit)
        {
            return Err(StatusReply::new(StatusCode::Failure));
        }
        // Model a competing writer creating the exact path after the client chose
        // it, but before the server applies OPEN's CREATE/EXCLUDE/TRUNCATE flags.
        if state.collide_next_open && flags.contains(OpenFlags::CREATE) {
            state.collide_next_open = false;
            state.collided_path = Some(filename.clone());
            state
                .files
                .insert(filename.clone(), b"collision sentinel".to_vec());
        }
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
        state.opened.push(filename.clone());
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
        let path = state
            .handles
            .get(&handle)
            .ok_or_else(|| StatusReply::new(StatusCode::Failure))?;
        let data = state.files.get(path).unwrap();
        let start = offset as usize;
        if start >= data.len() {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        Ok(Data {
            id,
            data: data[start..data.len().min(start + len as usize)].to_vec(),
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
        self.0.lock().unwrap().handles.remove(&handle);
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        if state.fail_rename {
            state.fail_rename = false;
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
        self.0.lock().unwrap().files.remove(&path);
        Ok(ok(id))
    }
}

struct Fixture {
    state: Arc<Mutex<Memory>>,
}

impl Fixture {
    async fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(Memory::default())),
        }
    }

    async fn manager(&mut self) -> SftpManager {
        // Keep the real serialized SFTP protocol on independent byte streams.
        // The library's server loop only ends on EOF; bare TCP resets on shutdown
        // can retain its handler on Windows even after all upload checks pass.
        let (client, stream) = duplex(256 * 1024);
        russh_sftp::server::run(stream, Peer(self.state.clone())).await;
        SftpManager::new(SftpSession::new(client).await.unwrap())
    }
}

async fn finish(fixture: Fixture, managers: Vec<SftpManager>) {
    for manager in managers {
        timeout(Duration::from_secs(2), manager.shutdown())
            .await
            .expect("SFTP manager shutdown exceeded its bound");
    }
    timeout(Duration::from_secs(5), async {
        loop {
            if Arc::strong_count(&fixture.state) == 1 {
                return;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("synthetic SFTP peer handler did not release shared storage after shutdown");
}

async fn barrier(manager: &SftpManager, path: &str) {
    manager
        .stat(path, false)
        .await
        .expect("same-session STAT barrier should succeed");
}

#[tokio::test]
async fn separate_sessions_interleave_and_cancel_or_shutdown_only_their_upload() {
    let mut fixture = Fixture::new().await;
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/shared.bin".into(), b"original".to_vec());
    let mut first = fixture.manager().await;
    let mut second = fixture.manager().await;
    let a = first
        .open_upload("/shared.bin", Some(5), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let b = second
        .open_upload("/shared.bin", Some(6), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    // Transfer IDs are manager-local; equal values still have separate sessions.
    first.write(&a.id, &[0, 255]).await.unwrap();
    second.write(&b.id, &[9, 0, 128]).await.unwrap();
    first.write(&a.id, &[1, 0, 2]).await.unwrap();
    second.write(&b.id, &[254, 3, 4]).await.unwrap();
    let paths = fixture.state.lock().unwrap().opened.clone();
    barrier(&first, &paths[0]).await;
    barrier(&second, &paths[1]).await;
    assert_eq!(
        fixture.state.lock().unwrap().files["/shared.bin"],
        b"original"
    );
    assert_eq!(paths.len(), 2);
    assert_ne!(paths[0], paths[1]);
    assert_eq!(first.close(&a.id).await.unwrap().state, "completed");
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["/shared.bin"], [0, 255, 1, 0, 2]);
        assert_eq!(state.files[&paths[1]], [9, 0, 128, 254, 3, 4]);
    }
    assert_eq!(second.close(&b.id).await.unwrap().state, "completed");
    assert_eq!(
        fixture.state.lock().unwrap().files["/shared.bin"],
        [9, 0, 128, 254, 3, 4]
    );

    // Cancel before the other session commits: only the cancelled stage goes away.
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/shared.bin".into(), b"preserve me".to_vec());
    let c = first
        .open_upload("/shared.bin", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let d = second
        .open_upload("/shared.bin", Some(2), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    first.write(&c.id, &[7]).await.unwrap();
    second.write(&d.id, &[8, 9]).await.unwrap();
    let paths = fixture.state.lock().unwrap().opened.clone();
    let c_path = paths[2].clone();
    let d_path = paths[3].clone();
    barrier(&first, &c_path).await;
    barrier(&second, &d_path).await;
    first.cancel(&c.id).await.unwrap();
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["/shared.bin"], b"preserve me");
        assert!(!state.files.contains_key(&c_path));
        assert_eq!(state.files[&d_path], [8, 9]);
    }
    second.close(&d.id).await.unwrap();
    assert_eq!(fixture.state.lock().unwrap().files["/shared.bin"], [8, 9]);

    // Cancelling A after B has committed preserves B's result and only removes A's stage.
    let e = first
        .open_upload("/shared.bin", Some(3), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let e_path = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    let f = second
        .open_upload("/shared.bin", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let f_path = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    first.write(&e.id, &[1, 2, 3]).await.unwrap();
    second.write(&f.id, &[42]).await.unwrap();
    barrier(&first, &e_path).await;
    barrier(&second, &f_path).await;
    second.close(&f.id).await.unwrap();
    first.cancel(&e.id).await.unwrap();
    assert_eq!(fixture.state.lock().unwrap().files["/shared.bin"], [42]);
    assert!(!fixture.state.lock().unwrap().files.contains_key(&e_path));

    // Shutting down A while B remains active preserves B's stage and lets B commit.
    let g = first
        .open_upload("/shared.bin", Some(2), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let g_path = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    let h = second
        .open_upload("/shared.bin", Some(2), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let h_path = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    first.write(&g.id, &[10, 11]).await.unwrap();
    second.write(&h.id, &[61, 62]).await.unwrap();
    barrier(&first, &g_path).await;
    barrier(&second, &h_path).await;
    timeout(Duration::from_secs(2), first.shutdown())
        .await
        .expect("first session shutdown exceeded its bound");
    assert_eq!(fixture.state.lock().unwrap().files["/shared.bin"], [42]);
    assert!(!fixture.state.lock().unwrap().files.contains_key(&g_path));
    assert_eq!(fixture.state.lock().unwrap().files[&h_path], [61, 62]);
    second.close(&h.id).await.unwrap();
    assert_eq!(fixture.state.lock().unwrap().files["/shared.bin"], [61, 62]);
    finish(fixture, vec![second]).await;
}

#[tokio::test]
async fn exclusive_temp_collision_preserves_competing_file_and_destination() {
    let mut fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state
            .files
            .insert("/target.bin".into(), b"destination".to_vec());
        state.collide_next_open = true;
    }
    let mut manager = fixture.manager().await;
    let result = manager
        .open_upload("/target.bin", Some(1), SftpOverwritePolicy::Overwrite)
        .await;
    assert!(result.is_err(), "existing staging path must reject OPEN");
    {
        let state = fixture.state.lock().unwrap();
        let collision_path = state
            .collided_path
            .as_ref()
            .expect("peer injected a collision");
        assert_eq!(state.files[collision_path], b"collision sentinel");
        assert_eq!(state.files["/target.bin"], b"destination");
        assert!(
            state.handles.is_empty(),
            "collision must not yield a handle"
        );
    }
    finish(fixture, vec![manager]).await;
}

#[tokio::test]
async fn rename_and_incomplete_size_failures_clean_only_their_own_stages() {
    let mut fixture = Fixture::new().await;
    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/shared".into(), b"old destination".to_vec());
    let mut first = fixture.manager().await;
    let mut second = fixture.manager().await;
    let a = first
        .open_upload("/shared", Some(2), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let b = second
        .open_upload("/shared", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    first.write(&a.id, &[1, 2]).await.unwrap();
    second.write(&b.id, &[3]).await.unwrap();
    let paths = fixture.state.lock().unwrap().opened.clone();
    barrier(&first, &paths[0]).await;
    barrier(&second, &paths[1]).await;
    fixture.state.lock().unwrap().fail_rename = true;
    assert!(first.close(&a.id).await.is_err());
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["/shared"], b"old destination");
        assert!(!state.files.contains_key(&paths[0]));
        assert_eq!(state.files[&paths[1]], [3]);
    }
    second.close(&b.id).await.unwrap();
    assert_eq!(fixture.state.lock().unwrap().files["/shared"], [3]);

    fixture
        .state
        .lock()
        .unwrap()
        .files
        .insert("/shared".into(), b"reset destination".to_vec());
    let short = first
        .open_upload("/shared", Some(4), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    let keep = second
        .open_upload("/shared", Some(1), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    first.write(&short.id, &[5]).await.unwrap();
    second.write(&keep.id, &[6]).await.unwrap();
    let paths = fixture.state.lock().unwrap().opened.clone();
    barrier(&first, &paths[2]).await;
    barrier(&second, &paths[3]).await;
    assert!(first.close(&short.id).await.is_err());
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["/shared"], b"reset destination");
        assert!(!state.files.contains_key(&paths[2]));
        assert_eq!(state.files[&paths[3]], [6]);
    }
    second.close(&keep.id).await.unwrap();
    assert_eq!(fixture.state.lock().unwrap().files["/shared"], [6]);
    finish(fixture, vec![first, second]).await;
}

#[tokio::test]
async fn overwrite_policy_controls_zero_unknown_sizes_and_bounded_remote_names() {
    let mut fixture = Fixture::new().await;
    {
        let mut state = fixture.state.lock().unwrap();
        state.files.insert("/skip".into(), b"keep".to_vec());
        state.files.insert("/rename".into(), b"old".to_vec());
        // Synthetic filesystem policy: this peer rejects components over 255 bytes.
        state.max_component = Some(255);
    }
    let mut manager = fixture.manager().await;
    assert!(manager
        .open_upload("/skip", Some(1), SftpOverwritePolicy::Skip)
        .await
        .is_err());
    assert_eq!(fixture.state.lock().unwrap().files["/skip"], b"keep");
    let renamed = manager
        .open_upload("/rename", Some(1), SftpOverwritePolicy::Rename)
        .await
        .unwrap();
    manager.write(&renamed.id, &[77]).await.unwrap();
    let renamed_stage = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    barrier(&manager, &renamed_stage).await;
    let renamed_name = renamed.name.clone();
    manager.close(&renamed.id).await.unwrap();
    assert_eq!(
        fixture.state.lock().unwrap().files[&format!("/rename ({})", 1)],
        [77]
    );
    assert_eq!(renamed_name, "rename (1)");

    let long_path = format!("/{}", "x".repeat(250));
    let zero = manager
        .open_upload(&long_path, Some(0), SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    manager.close(&zero.id).await.unwrap();
    assert!(fixture.state.lock().unwrap().files[&long_path].is_empty());

    let relative = manager
        .open_upload("relative.bin", None, SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    manager.write(&relative.id, &[0, 255, 128]).await.unwrap();
    let relative_stage = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    barrier(&manager, &relative_stage).await;
    manager.close(&relative.id).await.unwrap();
    let root_child = manager
        .open_upload("/root-child.bin", None, SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    manager.write(&root_child.id, &[1, 0]).await.unwrap();
    let root_stage = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    barrier(&manager, &root_stage).await;
    manager.close(&root_child.id).await.unwrap();
    let nested = manager
        .open_upload("/parent/child.bin", None, SftpOverwritePolicy::Overwrite)
        .await
        .unwrap();
    manager.write(&nested.id, &[4, 0, 5]).await.unwrap();
    let nested_stage = fixture.state.lock().unwrap().opened.last().unwrap().clone();
    barrier(&manager, &nested_stage).await;
    manager.close(&nested.id).await.unwrap();

    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.files["relative.bin"], [0, 255, 128]);
        assert_eq!(state.files["/root-child.bin"], [1, 0]);
        assert_eq!(state.files["/parent/child.bin"], [4, 0, 5]);
        for (opened, final_path) in state.opened[1..5].iter().zip([
            long_path.as_str(),
            "relative.bin",
            "/root-child.bin",
            "/parent/child.bin",
        ]) {
            assert_eq!(
                opened.rsplit_once('/').map(|(p, _)| p).unwrap_or(""),
                final_path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
            );
            assert!(opened.rsplit('/').next().unwrap().len() <= 255);
        }
    }
    finish(fixture, vec![manager]).await;
}
