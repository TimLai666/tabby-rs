use std::collections::HashMap;

use super::{resolved_shell_identity, ProcessIdentity};

fn identity(pid: u32, start_time: u64) -> ProcessIdentity {
    ProcessIdentity { pid, start_time }
}

fn starts(entries: &[(u32, u64)]) -> HashMap<u32, u64> {
    entries.iter().copied().collect()
}

#[test]
fn cached_shell_remains_selected_when_a_unique_job_is_added_below_it() {
    let parents = HashMap::from([(20, 10), (30, 20)]);
    let start_times = starts(&[(10, 100), (20, 110), (30, 120)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        Some(identity(20, 110)),
    );
}

#[test]
fn cached_shell_remains_selected_across_multiple_later_job_layers() {
    let parents = HashMap::from([(20, 10), (30, 20), (40, 30), (50, 40)]);
    let start_times = starts(&[(10, 100), (20, 110), (30, 120), (40, 130), (50, 140)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        Some(identity(20, 110)),
    );
}

#[test]
fn missing_cached_process_is_rejected() {
    let parents = HashMap::from([(20, 10), (30, 20)]);
    let start_times = starts(&[(10, 100), (30, 120)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        None,
    );
}

#[test]
fn reused_cached_pid_is_rejected_by_start_time() {
    let parents = HashMap::from([(20, 10), (30, 20)]);
    let start_times = starts(&[(10, 100), (20, 115), (30, 120)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        None,
    );
}

#[test]
fn reused_root_pid_is_rejected() {
    let parents = HashMap::from([(20, 10), (30, 20)]);
    let start_times = starts(&[(10, 101), (20, 110), (30, 120)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        None,
    );
}

#[test]
fn cached_process_outside_the_root_ancestry_is_rejected() {
    let parents = HashMap::from([(20, 99), (30, 20)]);
    let start_times = starts(&[(10, 100), (20, 110), (30, 120)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        None,
    );
}

#[test]
fn cyclic_cached_ancestry_is_rejected() {
    let parents = HashMap::from([(20, 30), (30, 20)]);
    let start_times = starts(&[(10, 100), (20, 110), (30, 120)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &parents,
            &start_times
        ),
        None,
    );
}

#[test]
fn absent_cache_keeps_existing_unique_chain_behavior() {
    let parents = HashMap::from([(20, 10), (30, 20), (40, 30)]);
    let start_times = starts(&[(10, 100), (20, 110), (30, 120), (40, 130)]);

    assert_eq!(
        resolved_shell_identity(10, Some(100), None, &parents, &start_times),
        Some(identity(40, 130)),
    );
}

#[test]
fn absent_cache_keeps_existing_fork_behavior() {
    let parents = HashMap::from([(20, 10), (30, 20), (40, 20)]);
    let start_times = starts(&[(10, 100), (20, 110), (30, 120), (40, 121)]);

    assert_eq!(
        resolved_shell_identity(10, Some(100), None, &parents, &start_times),
        Some(identity(20, 110)),
    );
}

#[test]
fn separate_snapshots_keep_their_cached_shells_independent() {
    let first_snapshot_parents = HashMap::from([(20, 10), (30, 20)]);
    let first_snapshot_starts = starts(&[(10, 100), (20, 110), (30, 120)]);
    let second_snapshot_parents = HashMap::from([(50, 10), (60, 50)]);
    let second_snapshot_starts = starts(&[(10, 100), (50, 115), (60, 125)]);

    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(20, 110)),
            &first_snapshot_parents,
            &first_snapshot_starts,
        ),
        Some(identity(20, 110)),
    );
    assert_eq!(
        resolved_shell_identity(
            10,
            Some(100),
            Some(identity(50, 115)),
            &second_snapshot_parents,
            &second_snapshot_starts,
        ),
        Some(identity(50, 115)),
    );
}
