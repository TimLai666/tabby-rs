// The framework boundary stays mocked; the real Win32 mutex, the real hidden
// owner window, and the real single-instance plugin startup path stay native.
// This case pins the startup interval between an existing named mutex and the
// owner window becoming discoverable: while the mutex is held but no window is
// findable, a launching secondary must neither exit nor proceed past startup.

use super::*;

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateEventW, OpenMutexW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE},
    },
    UI::WindowsAndMessaging::{
        DefWindowProcW, FindWindowW, RegisterClassExW, UnregisterClassW, WM_NCCREATE, WNDCLASSEXW,
    },
};

const ENTERING_OBSERVATION: Duration = Duration::from_secs(2);

#[test]
fn secondary_waits_for_a_delayed_owner_window_without_early_state() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private delayed-owner namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);

    // The main test thread owns the primary's named mutex, exactly as a starting
    // primary would before it has created its hidden owner window. No window is
    // built here, so the launch interval under test is the real one.
    let mutex_name = wide(&format!("{identifier}-sim"));
    let mutex = OwnedMutex(unsafe { CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) });
    assert!(
        !mutex.0.is_null(),
        "the test thread owns the primary's named mutex"
    );
    assert!(
        owner_window(&identifier).is_null(),
        "the owner window must not exist while only the mutex is held"
    );

    let label = "delayed-owner";
    let ready = base.path().join(format!("{label}.ready"));
    let config = base.path().join(format!("{label}.config.yaml"));
    let entering = base.path().join(format!("{label}.entering"));

    let cwd = owned_cwd(base.path(), "delayed-owner-cwd");
    let args = skip(&format!("delayed 資料 with  spaces {token}"));
    let mut command = worker_command(base.path(), label, &identifier, &cwd, &args);
    command.env(WORKER_ENTERING, &entering);
    command.env(WORKER_CONFIG, &config);
    let expected = expected_argv(&command);

    let mut worker = ChildGuard::spawn(&mut command, label).expect("start delayed-owner worker");
    await_entering(&entering, &mut worker, deadline);

    let observe_until = Instant::now() + ENTERING_OBSERVATION;
    loop {
        if let Some(status) = worker.try_wait().expect("inspect delayed-owner worker") {
            panic!(
                "a secondary must not exit while the owner window is unavailable: status={status}; \
                 ready={} config={}",
                ready.exists(),
                config.exists()
            );
        }
        if Instant::now() >= observe_until {
            break;
        }
        thread::sleep(POLL);
    }
    assert!(
        !ready.exists() && !config.exists(),
        "a secondary that cannot find the owner window must not publish readiness or write \
         configuration; ready={} config={}",
        ready.exists(),
        config.exists()
    );

    // Publishing the owner window must let the still-waiting secondary hand off
    // its complete original argv and cwd, then exit successfully.
    let legacy = LegacyServer::start(&identifier, false);
    let record = legacy
        .receive(deadline)
        .expect("the delayed owner window receives exactly one launch request");
    assert_eq!(
        record.argv, expected,
        "the delayed handoff must preserve the complete original argv"
    );
    assert!(
        working_directory_matches(&record.cwd, &cwd),
        "the delayed handoff must preserve the complete working directory; got {:?}",
        record.cwd
    );
    let status = worker.wait_bounded(deadline, "delayed-owner handoff exit");
    assert_eq!(
        status.code(),
        Some(0),
        "a representable delayed handoff must exit successfully: {status}"
    );
    assert!(
        !ready.exists() && !config.exists(),
        "a successful handoff must never publish readiness or write configuration; ready={} \
         config={}",
        ready.exists(),
        config.exists()
    );
    assert!(
        legacy.try_receive().is_none(),
        "the delayed handoff delivers exactly one callback"
    );

    drop(legacy);
    drop(worker);
    drop(mutex);
}

fn owner_window(identifier: &str) -> HWND {
    let class = wide(&format!("{identifier}-sic"));
    let title = wide(&format!("{identifier}-siw"));
    unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) }
}

fn expected_argv(command: &Command) -> Vec<String> {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| {
            argument
                .to_str()
                .expect("worker arguments are valid Unicode")
                .to_owned()
        })
        .collect()
}

fn await_entering(path: &Path, worker: &mut ChildGuard, deadline: Instant) {
    loop {
        if path.exists() {
            return;
        }
        if let Some(status) = worker.try_wait().expect("inspect delayed-owner worker") {
            panic!("the worker exited before entering plugin setup: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the worker to enter plugin setup"
        );
        thread::sleep(POLL);
    }
}

#[test]
fn graceful_mutex_release_admits_one_of_two_waiting_workers() {
    run_mutex_release_case(false);
}

#[test]
fn abandoned_mutex_admits_one_of_two_waiting_workers() {
    run_mutex_release_case(true);
}

fn run_mutex_release_case(abandon: bool) {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private mutex-release namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);

    let (acquired_tx, acquired_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let owner_identifier = identifier.clone();
    let owner_thread = thread::spawn(move || unsafe {
        let name = wide(&format!("{owner_identifier}-sim"));
        let handle = OwnedHandle(CreateMutexW(std::ptr::null(), 1, name.as_ptr()));
        assert!(!handle.0.is_null(), "owner thread creates named mutex");
        acquired_tx.send(()).expect("report mutex ownership");
        release_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("release or abandon owner thread before deadline");
        if !abandon {
            assert_ne!(ReleaseMutex(handle.0), 0, "owner thread releases mutex");
        }
        // In the abandoned case, the thread exits without ReleaseMutex.
    });
    acquired_rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("owner thread acquired named mutex");

    // Keep the same kernel object alive even if no worker has opened its handle
    // when the owner thread exits. The abandoned case must acquire that object.
    let mutex_name = wide(&format!("{identifier}-sim"));
    let _retained_mutex =
        OwnedHandle(unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, mutex_name.as_ptr()) });
    assert!(
        !_retained_mutex.0.is_null(),
        "retain the owner's mutex object"
    );

    let config_path = base.path().join("shared.config.yaml");
    let mut workers = Vec::new();
    let mut entering_paths = Vec::new();
    let mut ready_paths = Vec::new();
    let mut receipt_paths = Vec::new();
    let mut stop_paths = Vec::new();
    let mut expected_args = Vec::new();
    let mut expected_cwds = Vec::new();
    for label in ["release-worker-a", "release-worker-b"] {
        let entering = base.path().join(format!("{label}.entering"));
        let ready = base.path().join(format!("{label}.ready"));
        entering_paths.push(entering.clone());
        ready_paths.push(ready);
        receipt_paths.push(base.path().join(format!("{label}.receipt.jsonl")));
        stop_paths.push(base.path().join(format!("{label}.stop")));
        let cwd = owned_cwd(base.path(), &format!("{label}-cwd"));
        expected_cwds.push(cwd.clone());
        let args = skip(&format!("{label} {token}"));
        let mut command = worker_command(base.path(), label, &identifier, &cwd, &args);
        command
            .env(WORKER_ENTERING, &entering)
            .env(WORKER_CONFIG, &config_path);
        expected_args.push(expected_argv(&command));
        workers.push(ChildGuard::spawn(&mut command, label).expect("spawn competing worker"));
    }
    for (path, worker) in entering_paths.iter().zip(workers.iter_mut()) {
        await_entering(path, worker, deadline);
    }
    let observation_end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < observation_end {
        for worker in &mut workers {
            assert!(
                worker.try_wait().expect("inspect waiting worker").is_none(),
                "worker must wait while owner thread holds mutex"
            );
        }
        assert!(ready_paths.iter().all(|path| !path.exists()));
        assert!(
            !config_path.exists(),
            "config stays absent while mutex is held"
        );
        thread::sleep(POLL);
    }

    release_tx
        .send(())
        .expect("allow owner thread to exit or release");
    if abandon {
        owner_thread
            .join()
            .expect("owner thread exits while holding mutex");
    } else {
        owner_thread
            .join()
            .expect("owner thread releases and exits");
    }

    loop {
        if ready_paths.iter().any(|path| path.exists()) {
            break;
        }
        for worker in &mut workers {
            if let Some(status) = worker.try_wait().expect("inspect competing worker") {
                assert!(
                    status.success(),
                    "a waiting worker must not fail before a winner is ready"
                );
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for startup winner"
        );
        thread::sleep(POLL);
    }
    let ready_count = ready_paths.iter().filter(|path| path.exists()).count();
    assert_eq!(ready_count, 1, "exactly one worker owns startup");
    let yaml = fs::read_to_string(&config_path).expect("winner wrote shared config");
    let pid: u32 = yaml
        .strip_prefix("instance_worker: ")
        .and_then(|value| value.trim().parse().ok())
        .expect("shared config contains actual winner pid");
    let winner = workers
        .iter()
        .position(|worker| worker.child.as_ref().expect("worker remains guarded").id() == pid)
        .expect("config pid identifies one spawned worker");
    let loser = 1 - winner;
    let loser_status = workers[loser].wait_bounded(deadline, "losing worker handoff exit");
    assert_eq!(loser_status.code(), Some(0), "loser hands launch to winner");
    let receipts = read_receipts(&receipt_paths[winner]);
    assert_eq!(receipts.len(), 1, "winner receives exactly one callback");
    assert_eq!(
        receipts[0].argv, expected_args[loser],
        "handoff preserves complete argv"
    );
    assert!(
        working_directory_matches(&receipts[0].cwd, &expected_cwds[loser]),
        "handoff preserves original working directory"
    );
    assert!(
        !receipt_paths[loser].exists(),
        "loser has no callback receipt"
    );
    fs::write(&stop_paths[winner], b"stop").expect("stop winning worker");
    let winner_status = workers[winner].wait_bounded(deadline, "winning worker stop");
    assert_eq!(
        winner_status.code(),
        Some(0),
        "winner exits through worker teardown"
    );
    assert_eq!(
        ready_paths.iter().filter(|path| path.exists()).count(),
        1,
        "handoff never initialized the second worker"
    );
    // Both guards have now observed and reaped their process via wait_bounded.
}

#[test]
fn named_event_collision_makes_mutex_creation_fail_without_startup_state() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private event-collision namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    let name = wide(&format!("{identifier}-sim"));
    let event = OwnedHandle(unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) });
    assert!(!event.0.is_null(), "test owns colliding event name");

    let cwd = owned_cwd(base.path(), "event-collision-cwd");
    let ready = base.path().join("event-collision.ready");
    let config = base.path().join("event-collision.config.yaml");
    let entering = base.path().join("event-collision.entering");
    let mut command = worker_command(base.path(), "event-collision", &identifier, &cwd, &[]);
    command
        .env(WORKER_ENTERING, &entering)
        .env(WORKER_CONFIG, &config);
    let mut worker =
        ChildGuard::spawn(&mut command, "event-collision").expect("spawn collision worker");
    await_entering(&entering, &mut worker, deadline);
    loop {
        assert!(
            !ready.exists() && !config.exists(),
            "CreateMutexW name collision must fail before readiness or config"
        );
        if let Some(status) = worker.try_wait().expect("inspect collision worker") {
            assert!(
                !status.success(),
                "CreateMutexW collision with event must fail startup"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for CreateMutexW failure"
        );
        thread::sleep(POLL);
    }
}

#[test]
fn rejected_native_owner_window_fails_startup_and_identity_recovers() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private rejected-window namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    let cwd = owned_cwd(base.path(), "reject-window-cwd");
    let ready = base.path().join("reject-window.ready");
    let config = base.path().join("reject-window.config.yaml");
    let entering = base.path().join("reject-window.entering");
    let mut command = worker_command(base.path(), "reject-window", &identifier, &cwd, &[]);
    command
        .env(WORKER_REJECT_WINDOW, "1")
        .env(WORKER_ENTERING, &entering)
        .env(WORKER_CONFIG, &config);
    let mut failing =
        ChildGuard::spawn(&mut command, "reject-window").expect("spawn rejecting worker");
    await_entering(&entering, &mut failing, deadline);
    loop {
        assert!(
            !ready.exists() && !config.exists(),
            "rejected CreateWindowW must not publish readiness or config"
        );
        if let Some(status) = failing.try_wait().expect("inspect rejected-window worker") {
            assert!(!status.success(), "failed CreateWindowW must fail startup");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for CreateWindowW failure"
        );
        thread::sleep(POLL);
    }

    let mut command = worker_command(base.path(), "recovered-owner", &identifier, &cwd, &[]);
    let mut recovered =
        ChildGuard::spawn(&mut command, "recovered-owner").expect("spawn recovery worker");
    let recovered_ready = base.path().join("recovered-owner.ready");
    wait_until(
        deadline,
        || recovered_ready.exists(),
        "replacement worker readiness",
    );
    let recovery_receipt = base.path().join("recovered-owner.receipt.jsonl");
    let recovery_args = vec!["recovery-callback".to_owned(), token.clone()];
    let recovery_cwd = owned_cwd(base.path(), "recovery-callback-cwd");
    let payload = serde_json::to_vec(&(recovery_cwd.as_str(), &recovery_args))
        .expect("serialize real callback payload");
    let hwnd = owner_window(&identifier);
    assert!(
        !hwnd.is_null(),
        "replacement worker published its native owner window"
    );
    assert_eq!(
        send_copydata(hwnd, JSON_DATA_ID, &payload),
        Some(1),
        "replacement owner accepts a real native callback"
    );
    wait_for_native_records(
        &recovery_receipt,
        deadline,
        &[(recovery_args, recovery_cwd)],
    );
    fs::write(base.path().join("recovered-owner.stop"), b"stop").expect("stop recovered owner");
    let recovered_status = recovered.wait_bounded(deadline, "replacement worker stop");
    assert_eq!(recovered_status.code(), Some(0));
}

pub(in crate::instance_transport_tests) fn reject_owner_window_creation(
    identifier: &str,
) -> RejectingOwnerClass {
    let name = wide(&format!("{identifier}-sic"));
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(rejecting_window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: name.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    assert_ne!(
        unsafe { RegisterClassExW(&class) },
        0,
        "register rejecting owner class"
    );
    RejectingOwnerClass { name, instance }
}

pub(in crate::instance_transport_tests) struct RejectingOwnerClass {
    name: Vec<u16>,
    instance: HINSTANCE,
}

impl Drop for RejectingOwnerClass {
    fn drop(&mut self) {
        unsafe { UnregisterClassW(self.name.as_ptr(), self.instance) };
    }
}

unsafe extern "system" fn rejecting_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        0
    } else {
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}
