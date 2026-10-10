use std::{
    fs::{self, OpenOptions},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::tempdir;

use super::manager::TransferManager;

const UMASK_CHILD_ENV: &str = "TABBY_DOWNLOAD_MODE_UMASK_CHILD";
const MODE_TEST_PATH: &str =
    "transfer::download_mode_tests::downloaded_files_respect_the_original_creation_umask";

#[test]
fn downloaded_files_respect_the_original_creation_umask() {
    if std::env::var_os(UMASK_CHILD_ENV).is_some() {
        compare_download_modes_to_fs_open_semantics();
        return;
    }

    let executable = std::env::current_exe().expect("current test executable");
    let mut command = Command::new(executable);
    command
        .arg("--exact")
        .arg(MODE_TEST_PATH)
        .env(UMASK_CHILD_ENV, "1");
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o027);
            Ok(())
        });
    }
    let output = command
        .output()
        .expect("spawn the mode/umask comparison child");
    assert!(
        output.status.success(),
        "post-close chmod bypasses the creation umask: the mode/umask comparison child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn compare_download_modes_to_fs_open_semantics() {
    const PAYLOAD: &[u8] = &[0x00, 0x7f, 0x80, 0xff, 0x41];
    let mut mismatches = Vec::new();
    for requested_mode in [0o120777u32, 0o644, 0o755] {
        let sandbox = tempdir().expect("mode comparison sandbox");
        let reference = sandbox.path().join("fs_open_reference.txt");
        let destination = sandbox.path().join("downloaded.txt");

        let reference_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(requested_mode)
            .open(&reference)
            .expect("reference file using original fs.open creation semantics");
        drop(reference_file);
        let expected_mode = fs::metadata(&reference)
            .expect("reference metadata")
            .permissions()
            .mode();

        let manager = TransferManager::default();
        let descriptor = manager
            .open_download(
                "downloaded.txt",
                requested_mode,
                Some(PAYLOAD.len() as u64),
                destination.to_str().expect("destination path"),
                None,
                None,
            )
            .expect("open download");
        manager
            .write(&descriptor.id, PAYLOAD)
            .expect("write payload");
        manager.close(&descriptor.id).expect("close download");

        assert_eq!(
            fs::read(&destination).expect("read destination"),
            PAYLOAD,
            "exact payload must reach the destination for requested mode {requested_mode:#o}"
        );
        let actual_mode = fs::metadata(&destination)
            .expect("destination metadata")
            .permissions()
            .mode();
        if actual_mode != expected_mode {
            mismatches.push((requested_mode, expected_mode, actual_mode));
        }
    }
    assert!(
        mismatches.is_empty(),
        "post-close chmod bypasses the creation umask: {}",
        mismatches
            .iter()
            .map(|(requested, expected, actual)| format!(
                "requested {requested:#o}: destination {actual:#o} != original fs.open(path, w, ...) creation semantics {expected:#o} under the deterministic child umask 027"
            ))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

#[test]
fn download_exceeding_advertised_size_is_refused() {
    let sandbox = tempdir().expect("sandbox");
    let destination = sandbox.path().join("file.bin");
    let manager = TransferManager::default();
    let descriptor = manager
        .open_download(
            "file.bin",
            0o644,
            Some(3),
            destination.to_str().unwrap(),
            None,
            None,
        )
        .expect("open download");
    manager
        .write(&descriptor.id, b"xyz")
        .expect("write within the advertised size");
    let error = manager
        .write(&descriptor.id, b"!")
        .expect_err("a write above the advertised size must be refused");
    assert!(
        format!("{error}").contains("transfer exceeds advertised size"),
        "the production known-size guard must refuse oversized writes, got: {error}"
    );
    manager.close(&descriptor.id).expect("close accepted bytes");
    assert_eq!(
        fs::read(&destination).expect("read destination"),
        b"xyz",
        "only accepted bytes may reach the destination"
    );
}

#[test]
fn cancelled_download_preserves_the_old_destination_and_staging() {
    let sandbox = tempdir().expect("sandbox");
    let destination = sandbox.path().join("report.txt");
    fs::write(&destination, b"old").expect("pre-existing destination");
    let manager = TransferManager::default();
    let descriptor = manager
        .open_download(
            "report.txt",
            0o644,
            Some(3),
            destination.to_str().unwrap(),
            None,
            None,
        )
        .expect("open download");
    manager
        .write(&descriptor.id, b"new")
        .expect("partial write");
    manager.cancel(&descriptor.id).expect("cancel");

    assert_eq!(
        fs::read(&destination).expect("read old destination"),
        b"old",
        "cancellation must preserve the old destination"
    );
    let leftovers = staging_entries(sandbox.path(), &destination);
    assert!(
        leftovers.is_empty(),
        "cancelled transfer leaves staging entries: {leftovers:?}"
    );
}

#[test]
fn dropped_manager_removes_staging_and_preserves_the_old_destination() {
    let sandbox = tempdir().expect("sandbox");
    let destination = sandbox.path().join("report.txt");
    fs::write(&destination, b"old").expect("pre-existing destination");
    {
        let manager = TransferManager::default();
        let descriptor = manager
            .open_download(
                "report.txt",
                0o644,
                Some(3),
                destination.to_str().unwrap(),
                None,
                None,
            )
            .expect("open download");
        manager
            .write(&descriptor.id, b"new")
            .expect("partial write");
        assert!(!staging_entries(sandbox.path(), &destination).is_empty());
    }
    assert_eq!(
        fs::read(&destination).expect("read old destination"),
        b"old",
        "an uncommitted transfer must not replace the old destination"
    );
    let leftovers = staging_entries(sandbox.path(), &destination);
    assert!(
        leftovers.is_empty(),
        "dropping the transfer manager leaves staging entries: {leftovers:?}"
    );
}

#[test]
fn download_is_committed_only_on_close_and_leaves_no_staging() {
    let sandbox = tempdir().expect("sandbox");
    let destination = sandbox.path().join("file.bin");
    let manager = TransferManager::default();
    let descriptor = manager
        .open_download(
            "file.bin",
            0o644,
            Some(3),
            destination.to_str().unwrap(),
            None,
            None,
        )
        .expect("open download");
    manager.write(&descriptor.id, b"new").expect("write");

    assert!(
        !destination.exists(),
        "the destination must not be committed before close"
    );
    manager.close(&descriptor.id).expect("close download");
    assert_eq!(fs::read(&destination).expect("read destination"), b"new");

    let leftovers = staging_entries(sandbox.path(), &destination);
    assert!(
        leftovers.is_empty(),
        "completed transfer leaves staging entries: {leftovers:?}"
    );
}

#[test]
fn incomplete_close_preserves_the_old_destination_and_staging() {
    let sandbox = tempdir().expect("sandbox");
    let destination = sandbox.path().join("file.bin");
    fs::write(&destination, b"old").expect("pre-existing destination");
    let manager = TransferManager::default();
    let descriptor = manager
        .open_download(
            "file.bin",
            0o644,
            Some(5),
            destination.to_str().unwrap(),
            None,
            None,
        )
        .expect("open download");
    manager
        .write(&descriptor.id, b"new")
        .expect("partial write");
    let error = manager
        .close(&descriptor.id)
        .expect_err("closing below the advertised size must fail");
    assert!(
        format!("{error}").contains("size does not match"),
        "the production close guard must reject an incomplete download, got: {error}"
    );
    assert_eq!(
        fs::read(&destination).expect("read old destination"),
        b"old",
        "a failed close must preserve the old destination"
    );

    let leftovers = staging_entries(sandbox.path(), &destination);
    assert!(
        leftovers.is_empty(),
        "failed close leaves staging entries: {leftovers:?}"
    );
}

#[test]
fn staging_content_is_not_readable_by_group_or_other_below_destination_parent() {
    let sandbox = tempdir().expect("sandbox");
    let parent = sandbox.path().join("downloads");
    fs::create_dir(&parent).expect("create destination parent");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755))
        .expect("world-traversable download-folder fixture");
    let destination = parent.join("file.bin");
    let manager = TransferManager::default();
    let descriptor = manager
        .open_download(
            "file.bin",
            0o644,
            Some(64),
            destination.to_str().unwrap(),
            None,
            None,
        )
        .expect("open download");
    manager
        .write(&descriptor.id, &[0xab; 16])
        .expect("write incomplete content");

    let staging = staging_entries(&parent, &destination);
    assert!(
        !staging.is_empty(),
        "an in-flight transfer must hold a staging entry below the destination parent"
    );
    for entry in &staging {
        if !entry.is_file() {
            continue;
        }
        assert!(
            !group_or_other_can_read_through(&parent, entry),
            "incomplete content is readable by group or other through the staging path {}",
            entry.display()
        );
    }
    manager.cancel(&descriptor.id).expect("cancel");
}

fn staging_entries(parent: &Path, destination: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    let mut stack = vec![PathBuf::from(parent)];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).expect("read staging parent") {
            let path = entry.expect("staging entry").path();
            if path == destination {
                continue;
            }
            if path.is_dir() {
                stack.push(path.clone());
            }
            entries.push(path);
        }
    }
    entries
}

fn group_or_other_can_read_through(parent: &Path, path: &Path) -> bool {
    let relative = path
        .strip_prefix(parent)
        .expect("staging entry must live below the destination parent");
    let parts: Vec<_> = relative.components().collect();
    [3, 0].into_iter().any(|shift| {
        let mut current = PathBuf::from(parent);
        for (index, part) in parts.iter().enumerate() {
            current.push(part.as_os_str());
            let mode = fs::metadata(&current)
                .expect("staging entry metadata")
                .permissions()
                .mode()
                >> shift;
            if index + 1 == parts.len() {
                return mode & 0o4 != 0;
            }
            if mode & 0o1 == 0 {
                return false;
            }
        }
        false
    })
}
