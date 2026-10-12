use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

fn downloaded(
    manager: &FileEditManager,
    transfers: &TransferManager,
    bytes: &[u8],
) -> EditableFile {
    let edit = manager
        .prepare("../edit.bin", 0o640, Some(bytes.len() as u64), transfers)
        .unwrap();
    transfers.write(&edit.transfer.id, bytes).unwrap();
    transfers.close(&edit.transfer.id).unwrap();
    manager.ready(&edit.id).unwrap();
    edit
}
#[test]
fn binary_and_empty_editor_copies_are_scoped_and_retained_after_stop() {
    let manager = FileEditManager::default();
    let transfers = TransferManager::default();
    for bytes in [&[0, 255, 128, 3][..], &[][..]] {
        let edit = downloaded(&manager, &transfers, bytes);
        let path = Path::new(&edit.path);
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(
            path.parent().unwrap(),
            path.parent().unwrap().canonicalize().unwrap()
        );
        assert_ne!(path.file_name().unwrap(), "edit.bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        manager.stop(&edit.id).unwrap();
        manager.stop(&edit.id).unwrap();
        assert_eq!(
            fs::read(path).unwrap(),
            bytes,
            "the editor copy survives session end like original"
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
#[test]
fn watch_reports_saved_bytes_and_stops_after_session_end() {
    let manager = FileEditManager::default();
    let transfers = TransferManager::default();
    let edit = downloaded(&manager, &transfers, b"original");
    let (send, receive) = mpsc::channel();
    manager
        .watch(&edit.id, move |event| {
            let _ = send.send(event);
        })
        .unwrap();
    fs::write(&edit.path, [0, 255, 2, 128]).unwrap();
    let event = receive.recv_timeout(Duration::from_secs(8)).unwrap();
    assert_eq!(event.id, edit.id);
    assert_eq!(event.event, "change");
    assert_eq!(fs::read(&edit.path).unwrap(), [0, 255, 2, 128]);
    while receive.try_recv().is_ok() {}
    fs::write(&edit.path, [3, 0, 254, 129, 7]).unwrap();
    let second = receive.recv_timeout(Duration::from_secs(8)).unwrap();
    assert_eq!(second.id, edit.id);
    assert_eq!(
        second.event, "change",
        "in-place saves keep the watcher attached"
    );
    assert_eq!(fs::read(&edit.path).unwrap(), [3, 0, 254, 129, 7]);
    manager.stop(&edit.id).unwrap();
    while receive.try_recv().is_ok() {}
    fs::write(&edit.path, b"after-close").unwrap();
    assert!(receive.recv_timeout(Duration::from_millis(350)).is_err());
    fs::remove_dir_all(Path::new(&edit.path).parent().unwrap()).unwrap();
}
#[test]
fn watcher_detects_an_editor_replacing_the_file() {
    let manager = FileEditManager::default();
    let transfers = TransferManager::default();
    let edit = downloaded(&manager, &transfers, b"original");
    let (send, receive) = mpsc::channel();
    manager
        .watch(&edit.id, move |event| {
            let _ = send.send(event);
        })
        .unwrap();
    let replacement = Path::new(&edit.path).with_extension("replacement");
    fs::write(&replacement, [0, 255, 8]).unwrap();
    #[cfg(windows)]
    fs::remove_file(&edit.path).unwrap();
    fs::rename(&replacement, &edit.path).unwrap();
    let event = receive.recv_timeout(Duration::from_secs(8)).unwrap();
    assert_eq!(event.id, edit.id);
    assert_eq!(event.event, "rename");
    assert_eq!(fs::read(&edit.path).unwrap(), [0, 255, 8]);
    manager.stop(&edit.id).unwrap();
    assert_eq!(fs::read(&edit.path).unwrap(), [0, 255, 8]);
    fs::remove_dir_all(Path::new(&edit.path).parent().unwrap()).unwrap();
}
#[test]
fn unknown_workspace_ids_are_rejected_and_stop_is_idempotent() {
    let manager = FileEditManager::default();
    assert!(matches!(
        manager.ready("missing"),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        manager.watch("missing", |_| {}),
        Err(AppError::NotFound(_))
    ));
    manager.stop("missing").unwrap();
}
#[test]
fn incomplete_download_never_becomes_an_editable_file() {
    let manager = FileEditManager::default();
    let transfers = TransferManager::default();
    let edit = manager
        .prepare("note.txt", 0o600, Some(7), &transfers)
        .unwrap();
    assert!(manager.ready(&edit.id).is_err());
    transfers.cancel(&edit.transfer.id).unwrap();
    manager.stop(&edit.id).unwrap();
}

struct CallbackGate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl CallbackGate {
    fn new() -> Self {
        Self {
            state: Mutex::new((false, false)),
            changed: Condvar::new(),
        }
    }

    fn pause_callback(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.0 = true;
        self.changed.notify_all();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(8), |state| !state.1)
            .unwrap_or_else(|error| error.into_inner());
        drop(state);
    }

    fn wait_until_entered(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let (state, result) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(8), |state| !state.0)
            .unwrap_or_else(|error| error.into_inner());
        state.0 && !result.timed_out()
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.1 = true;
        self.changed.notify_all();
    }
}

struct WatchTestCleanup {
    manager: Arc<FileEditManager>,
    id: String,
    directory: PathBuf,
    gate: Arc<CallbackGate>,
}

impl Drop for WatchTestCleanup {
    fn drop(&mut self) {
        self.gate.release();
        let _ = self.manager.stop(&self.id);
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn stop_waits_for_inflight_callback_and_concurrent_same_id_stop() {
    let manager = Arc::new(FileEditManager::default());
    let transfers = TransferManager::default();
    let edit = downloaded(&manager, &transfers, b"before-stop");
    let directory = Path::new(&edit.path).parent().unwrap().to_path_buf();
    let gate = Arc::new(CallbackGate::new());
    let cleanup = WatchTestCleanup {
        manager: manager.clone(),
        id: edit.id.clone(),
        directory: directory.clone(),
        gate: gate.clone(),
    };
    let entered_once = Arc::new(AtomicBool::new(false));
    let callback_gate = gate.clone();
    let callback_entered_once = entered_once.clone();
    let (event_tx, event_rx) = mpsc::channel();
    manager
        .watch(&edit.id, move |event| {
            if !callback_entered_once.swap(true, Ordering::SeqCst) {
                callback_gate.pause_callback();
            }
            let _ = event_tx.send(event);
        })
        .unwrap();

    fs::write(&edit.path, b"during-stop").unwrap();
    if !gate.wait_until_entered() {
        gate.release();
        panic!("file watcher callback did not enter its gate within eight seconds");
    }

    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let mut stop_threads = Vec::new();
    for _ in 0..2 {
        let manager = manager.clone();
        let id = edit.id.clone();
        let attempt_tx = attempt_tx.clone();
        let done_tx = done_tx.clone();
        stop_threads.push(thread::spawn(move || {
            let _ = attempt_tx.send(());
            let succeeded = manager.stop(&id).is_ok();
            let _ = done_tx.send(succeeded);
        }));
    }
    drop(attempt_tx);
    drop(done_tx);

    let mut attempted = 0;
    for _ in 0..2 {
        if attempt_rx.recv_timeout(Duration::from_secs(8)).is_ok() {
            attempted += 1;
        }
    }

    // Observe both stop calls while the callback is still held. Do not assert yet: the gate
    // must be released and both threads joined even when the old implementation returns early.
    let observe_until = Instant::now() + Duration::from_millis(250);
    let mut completed_before_release = Vec::new();
    while completed_before_release.len() < 2 && Instant::now() < observe_until {
        let remaining = observe_until.saturating_duration_since(Instant::now());
        match done_rx.recv_timeout(remaining) {
            Ok(result) => completed_before_release.push(result),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    gate.release();
    let mut completion_results = completed_before_release.clone();
    while completion_results.len() < 2 {
        match done_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(result) => completion_results.push(result),
            Err(error) => {
                for handle in stop_threads {
                    let _ = handle.join();
                }
                panic!("stop threads did not finish after callback release: {error}");
            }
        }
    }
    for handle in stop_threads {
        handle.join().expect("stop thread panicked");
    }

    // The first callback notification was allowed to finish before stop returned. Drain any
    // queued pre-stop watcher events, then verify a later save is silent and the copy remains.
    while event_rx.try_recv().is_ok() {}
    fs::write(&edit.path, b"after-stop").unwrap();
    let post_stop_event = event_rx.recv_timeout(Duration::from_millis(350)).is_ok();
    let retained = fs::read(&edit.path)
        .map(|bytes| bytes == b"after-stop")
        .unwrap_or(false);

    assert_eq!(attempted, 2, "both same-id stop threads must start");
    assert!(
        completed_before_release.is_empty(),
        "neither stop call may finish before the callback is released"
    );
    assert!(completion_results.iter().all(|result| *result));
    assert!(!post_stop_event, "no watcher event may arrive after stop");
    assert!(retained, "stopping the watcher must retain the editor copy");
    drop(cleanup);
}
