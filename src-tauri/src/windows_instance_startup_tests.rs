// The framework boundary stays mocked; the real Win32 mutex, the real hidden
// owner window, and the real single-instance plugin startup path stay native.
// This case pins the startup interval between an existing named mutex and the
// owner window becoming discoverable: while the mutex is held but no window is
// findable, a launching secondary must neither exit nor proceed past startup.

use super::*;

use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

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
