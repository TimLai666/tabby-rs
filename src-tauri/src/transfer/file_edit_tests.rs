use super::*;
use std::{fs, path::Path, sync::mpsc, time::Duration};

fn downloaded(
    manager: &FileEditManager,
    transfers: &TransferManager,
    bytes: &[u8],
) -> EditableFile {
    let edit = manager
        .prepare("../edit.bin", 0o640, bytes.len() as u64, transfers)
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
    let edit = manager.prepare("note.txt", 0o600, 7, &transfers).unwrap();
    assert!(manager.ready(&edit.id).is_err());
    transfers.cancel(&edit.transfer.id).unwrap();
    manager.stop(&edit.id).unwrap();
}
