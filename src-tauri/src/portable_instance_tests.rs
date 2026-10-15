// Behavior tests are filled independently from the resolver implementation.

use std::fs;

use super::portable_instance_id;

fn root_with_data() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("data")).unwrap();
    root
}

fn assert_safe_instance_id(id: &str) {
    assert!(!id.is_empty(), "the instance id must not be empty");
    assert!(id.is_ascii(), "the instance id must be ASCII: {id:?}");
    assert!(
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'),
        "the instance id must only contain [A-Za-z0-9._-]: {id:?}"
    );
    assert!(
        id.len() <= 80,
        "the instance id must be at most 80 characters, got {}: {id:?}",
        id.len()
    );
    assert!(
        id.split('.').all(|part| {
            part.as_bytes().first().is_some_and(|first| {
                first.is_ascii_alphabetic() || *first == b'_' || *first == b'-'
            })
        }),
        "the Linux D-Bus name must not have an empty or digit-leading component: {id:?}"
    );
}

#[test]
fn none_root_returns_none_for_any_identifier() {
    assert_eq!(portable_instance_id("io.tabbyrs.app", None).unwrap(), None);
    assert_eq!(portable_instance_id("", None).unwrap(), None);
    assert_eq!(
        portable_instance_id("完全不同的識別碼", None).unwrap(),
        None
    );
}

#[test]
fn repeated_calls_for_the_same_identifier_and_data_directory_are_stable() {
    let root = root_with_data();
    let sentinel_path = root.path().join("data").join("sentinel.bin");
    let sentinel = b"sentinel bytes must stay untouched".to_vec();
    fs::write(&sentinel_path, &sentinel).unwrap();

    let first = portable_instance_id("io.tabbyrs.app", Some(root.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");
    assert_safe_instance_id(&first);
    assert_eq!(fs::read(&sentinel_path).unwrap(), sentinel);

    let second = portable_instance_id("io.tabbyrs.app", Some(root.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");
    assert_eq!(first, second);
    assert_eq!(fs::read(&sentinel_path).unwrap(), sentinel);
}

#[test]
fn distinct_data_directories_get_distinct_ids_even_with_equal_contents() {
    let first = root_with_data();
    let second = root_with_data();
    let payload = b"identical payload in both data directories".to_vec();
    fs::write(first.path().join("data").join("payload.bin"), &payload).unwrap();
    fs::write(second.path().join("data").join("payload.bin"), &payload).unwrap();

    let first_id = portable_instance_id("io.tabbyrs.app", Some(first.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");
    let second_id = portable_instance_id("io.tabbyrs.app", Some(second.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");

    assert_ne!(
        first_id, second_id,
        "distinct data directories must not share an instance id"
    );
}

#[test]
fn distinct_identifiers_for_one_data_directory_get_distinct_ids() {
    let root = root_with_data();

    let first_id = portable_instance_id("io.tabbyrs.app", Some(root.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");
    let second_id = portable_instance_id("io.tabbyrs.other", Some(root.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");

    assert_ne!(
        first_id, second_id,
        "distinct identifiers must not share an instance id"
    );
}

#[test]
fn equivalent_directory_spellings_resolve_to_the_same_id() {
    let root = root_with_data();
    fs::create_dir(root.path().join("child")).unwrap();

    let direct = portable_instance_id("io.tabbyrs.app", Some(root.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");
    let dotted = portable_instance_id("io.tabbyrs.app", Some(&root.path().join(".")))
        .unwrap()
        .expect("a portable root must produce an instance id");
    let parented = portable_instance_id(
        "io.tabbyrs.app",
        Some(&root.path().join("child").join("..")),
    )
    .unwrap()
    .expect("a portable root must produce an instance id");

    assert_eq!(direct, dotted);
    assert_eq!(direct, parented);
}

#[cfg(unix)]
#[test]
fn symlinked_data_directories_resolve_to_the_same_id() {
    let shared = tempfile::tempdir().unwrap();
    let shared_data = shared.path().join("real-data");
    fs::create_dir(&shared_data).unwrap();
    fs::write(shared_data.join("marker.bin"), b"shared").unwrap();

    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(&shared_data, first.path().join("data")).unwrap();
    std::os::unix::fs::symlink(&shared_data, second.path().join("data")).unwrap();

    let first_id = portable_instance_id("io.tabbyrs.app", Some(first.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");
    let second_id = portable_instance_id("io.tabbyrs.app", Some(second.path()))
        .unwrap()
        .expect("a portable root must produce an instance id");

    assert_eq!(
        first_id, second_id,
        "data symlinks to one directory must share an instance id"
    );
}

#[test]
fn non_ascii_paths_and_oversized_identifiers_still_produce_safe_ids() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("資料 夾 with spaces");
    fs::create_dir_all(root.join("data")).unwrap();

    let long_ascii = "identifier-".repeat(512);
    let long_non_ascii = "識別碼".repeat(512);

    for identifier in [long_ascii.as_str(), long_non_ascii.as_str()] {
        let id = portable_instance_id(identifier, Some(&root))
            .unwrap()
            .expect("a portable root must produce an instance id");
        assert_safe_instance_id(&id);
        assert!(
            !id.contains("資料"),
            "the instance id must not embed raw path text: {id:?}"
        );
        assert!(
            !id.contains(&*root.to_string_lossy()),
            "the instance id must not embed the raw path: {id:?}"
        );
    }
}

#[test]
fn missing_data_directory_is_rejected_without_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let sentinel_path = root.path().join("sentinel.bin");
    let sentinel = b"root sentinel must stay untouched".to_vec();
    fs::write(&sentinel_path, &sentinel).unwrap();

    let result = portable_instance_id("io.tabbyrs.app", Some(root.path()));

    assert!(
        result.is_err(),
        "a missing data directory must be rejected, got {result:?}"
    );
    assert!(
        !root.path().join("data").exists(),
        "resolution must not create a data directory"
    );
    assert_eq!(fs::read(&sentinel_path).unwrap(), sentinel);
}

#[test]
fn data_as_a_regular_file_is_rejected_without_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let data_path = root.path().join("data");
    let original = b"a regular file is not a data directory".to_vec();
    fs::write(&data_path, &original).unwrap();

    let result = portable_instance_id("io.tabbyrs.app", Some(root.path()));

    assert!(
        result.is_err(),
        "a non-directory data entry must be rejected, got {result:?}"
    );
    assert_eq!(fs::read(&data_path).unwrap(), original);
    assert!(!data_path.is_dir());
}
