use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, ChildKiller, ExitStatus};
use tauri::{EventId, Listener};

use super::PtyManager;
use crate::{
    pty::{
        backend::{PortablePtyBackend, PtyBackend, PtyError, SpawnedPty},
        model::PtySpawnRequest,
    },
    shell::PreparedSpawnRequest,
};

const FIXTURE_ENV: &str = "TABBY_PTY_READER_FIXTURE_DIR";
const FIXTURE_MODE: &str = "TABBY_PTY_READER_FIXTURE_MODE";
const FIXTURE_TEST: &str = "pty::manager::reader_tests::reader_fixture_process";
const OBSERVE_FOR: Duration = Duration::from_secs(4);
const ROUNDTRIP_BYTES: usize = 1024 * 1024;
const BLOCKED_WRITE_BYTES: usize = 2 * 1024 * 1024;
const NATURAL_TAIL: &[u8] = b"READER_TAIL_5d7194";
const LATE_ATTACH_TAIL: [u8; 768] = [b'Z'; 768];

enum PtyEvent {
    Output(crate::pty::model::PtyOutputEvent),
    Exit(crate::pty::model::PtyExitEvent),
}

fn listen_pty_events(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> (EventId, EventId, Receiver<PtyEvent>) {
    let (sender, receiver) = mpsc::channel();
    let output_sender = sender.clone();
    let output = app.listen("pty:output", move |event| {
        let payload = serde_json::from_str(event.payload()).expect("valid pty:output payload");
        let _ = output_sender.send(PtyEvent::Output(payload));
    });
    let exit = app.listen("pty:exit", move |event| {
        let payload = serde_json::from_str(event.payload()).expect("valid pty:exit payload");
        let _ = sender.send(PtyEvent::Exit(payload));
    });
    (output, exit, receiver)
}

#[derive(Default)]
struct Observation {
    read_pending: AtomicBool,
    read_calls: AtomicUsize,
    read_bytes: AtomicU64,
    read_eof: AtomicBool,
    reader_dropped: AtomicBool,
    child_wait_done: AtomicBool,
    child_wait_result: Mutex<Option<(u32, Option<String>)>>,
    write_pending: AtomicBool,
    write_calls: AtomicUsize,
    write_bytes: AtomicU64,
}

struct ObservedReader {
    inner: Option<Box<dyn Read + Send>>,
    observation: Arc<Observation>,
}

impl Read for ObservedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.observation.read_pending.store(true, Ordering::Release);
        self.observation.read_calls.fetch_add(1, Ordering::AcqRel);
        let result = self
            .inner
            .as_mut()
            .expect("reader remains present")
            .read(buffer);
        if let Ok(length) = &result {
            self.observation
                .read_bytes
                .fetch_add(*length as u64, Ordering::AcqRel);
        }
        self.observation
            .read_pending
            .store(false, Ordering::Release);
        if matches!(result, Ok(0)) {
            self.observation.read_eof.store(true, Ordering::Release);
        }
        result
    }
}

impl Drop for ObservedReader {
    fn drop(&mut self) {
        drop(self.inner.take());
        self.observation
            .reader_dropped
            .store(true, Ordering::Release);
    }
}

struct ObservedWriter {
    inner: Box<dyn Write + Send>,
    observation: Arc<Observation>,
}

impl Write for ObservedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.observation
            .write_pending
            .store(true, Ordering::Release);
        self.observation.write_calls.fetch_add(1, Ordering::AcqRel);
        let result = self.inner.write(buffer);
        if let Ok(length) = &result {
            self.observation
                .write_bytes
                .fetch_add(*length as u64, Ordering::AcqRel);
        }
        self.observation
            .write_pending
            .store(false, Ordering::Release);
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct ObservedChild {
    inner: Box<dyn Child + Send + Sync>,
    observation: Arc<Observation>,
}

impl std::fmt::Debug for ObservedChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservedChild")
            .finish_non_exhaustive()
    }
}

impl ChildKiller for ObservedChild {
    fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        self.inner.clone_killer()
    }
}

impl Child for ObservedChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let result = self.inner.wait();
        if let Ok(status) = &result {
            *self.observation.child_wait_result.lock().unwrap() =
                Some((status.exit_code(), status.signal().map(str::to_owned)));
        }
        self.observation
            .child_wait_done
            .store(true, Ordering::Release);
        result
    }

    fn process_id(&self) -> Option<u32> {
        self.inner.process_id()
    }

    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        self.inner.as_raw_handle()
    }
}

struct ObservedBackend {
    observations: Mutex<VecDeque<Arc<Observation>>>,
}

impl PtyBackend for ObservedBackend {
    fn spawn(&self, spec: &super::super::model::SpawnSpec) -> Result<SpawnedPty, PtyError> {
        let observation = self
            .observations
            .lock()
            .unwrap()
            .pop_front()
            .expect("test supplies one observation for each spawn");
        let mut spawned = PortablePtyBackend.spawn(spec)?;
        spawned.reader = Box::new(ObservedReader {
            inner: Some(spawned.reader),
            observation: Arc::clone(&observation),
        });
        spawned.writer = Box::new(ObservedWriter {
            inner: spawned.writer,
            observation: Arc::clone(&observation),
        });
        spawned.child = Box::new(ObservedChild {
            inner: spawned.child,
            observation,
        });
        Ok(spawned)
    }
}

struct OwnedPids {
    pids: Vec<u32>,
    forgotten: HashSet<u32>,
    descendant_marker: PathBuf,
}

impl OwnedPids {
    fn new(descendant_marker: PathBuf) -> Self {
        Self {
            pids: Vec::new(),
            forgotten: HashSet::new(),
            descendant_marker,
        }
    }

    fn track(&mut self, pid: u32) {
        self.pids.push(pid);
    }

    fn forget(&mut self, pid: u32) {
        self.pids.retain(|owned| *owned != pid);
        self.forgotten.insert(pid);
    }
}

impl Drop for OwnedPids {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(&self.descendant_marker) {
            if let Ok(pid) = pid.trim().parse() {
                if !self.pids.contains(&pid) && !self.forgotten.contains(&pid) {
                    self.pids.push(pid);
                }
            }
        }
        for pid in self.pids.drain(..) {
            if let Err(error) = terminate_fixture_pid(pid) {
                eprintln!("failed to clean up owned PTY fixture PID {pid}: {error}");
            }
        }
    }
}

fn terminate_fixture_pid(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, STILL_ACTIVE, WAIT_OBJECT_0},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, TerminateProcess, WaitForSingleObject,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
            },
        };
        let process = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if process.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(())
            } else {
                Err(error)
            };
        }
        let mut exit_code = 0;
        let queried = unsafe { GetExitCodeProcess(process, &mut exit_code) };
        let mut termination_error = if queried == 0 {
            Some(io::Error::last_os_error())
        } else if exit_code == STILL_ACTIVE as u32 {
            if unsafe { TerminateProcess(process, 1) } == 0 {
                Some(io::Error::last_os_error())
            } else {
                None
            }
        } else {
            None
        };
        if termination_error.is_none() {
            let waited = unsafe { WaitForSingleObject(process, 4_000) };
            if waited != WAIT_OBJECT_0 {
                termination_error = Some(if waited == u32::MAX {
                    io::Error::last_os_error()
                } else {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("process {pid} did not exit within 4 seconds"),
                    )
                });
            }
        }
        let closed = unsafe { CloseHandle(process) };
        if let Some(error) = termination_error {
            Err(error)
        } else if closed == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + OBSERVE_FOR;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(condition(), "timed out waiting for {what}");
}

fn touch(path: &Path) {
    fs::write(path, b"ready").expect("fixture marker is writable");
}

fn wait_for_file(path: &Path, what: &str) {
    wait_until(|| path.exists(), what);
}

fn numeric_heartbeat(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn wait_for_heartbeat_progress(path: &Path, previous: u64, what: &str) -> u64 {
    let mut current = None;
    wait_until(
        || {
            current = numeric_heartbeat(path).filter(|value| *value > previous);
            current.is_some()
        },
        what,
    );
    current.expect("wait_until observed numeric heartbeat progress")
}

fn receive_pty_event(receiver: &Receiver<PtyEvent>, what: &str) -> PtyEvent {
    receiver
        .recv_timeout(OBSERVE_FOR)
        .unwrap_or_else(|error| panic!("timed out waiting for {what}: {error}"))
}

fn ack_output(manager: &PtyManager, id: &str, output: crate::pty::model::PtyOutputEvent) {
    assert_eq!(output.id, id, "output belongs to requested PTY");
    manager
        .ack(id, output.data.len())
        .expect("ack exactly the emitted output chunk");
}

fn drain_startup_output(
    manager: &PtyManager,
    id: &str,
    observation: &Observation,
    receiver: &Receiver<PtyEvent>,
) {
    wait_native_read_blocked(
        observation,
        "native Read stabilizes after test-harness output",
    );
    let mut bytes = 0;
    while let Ok(event) = receiver.try_recv() {
        match event {
            PtyEvent::Output(output) if output.id == id => {
                bytes += output.data.len();
                ack_output(manager, id, output);
            }
            PtyEvent::Output(_) => panic!("unexpected PTY output id"),
            PtyEvent::Exit(_) => panic!("fixture exited before its behavior started"),
        }
    }
    assert!(bytes > 0, "actual pty:output delivered test-harness bytes");
}

fn observe_stable_blocked_write(
    manager: &PtyManager,
    id: &str,
    observation: &Observation,
    receiver: &Receiver<PtyEvent>,
) -> (Option<(usize, u64)>, bool) {
    let deadline = Instant::now() + OBSERVE_FOR;
    let mut saw_exit = false;
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok(PtyEvent::Output(output)) => ack_output(manager, id, output),
            Ok(PtyEvent::Exit(_)) => saw_exit = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if !observation.write_pending.load(Ordering::Acquire) {
            continue;
        }
        let calls = observation.write_calls.load(Ordering::Acquire);
        let bytes = observation.write_bytes.load(Ordering::Acquire);
        if calls == 0 {
            continue;
        }
        let stable_deadline = Instant::now() + Duration::from_millis(40);
        let mut stable = true;
        while Instant::now() < stable_deadline {
            match receiver.recv_timeout(Duration::from_millis(5)) {
                Ok(PtyEvent::Output(output)) => ack_output(manager, id, output),
                Ok(PtyEvent::Exit(_)) => saw_exit = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !observation.write_pending.load(Ordering::Acquire)
                || observation.write_calls.load(Ordering::Acquire) != calls
                || observation.write_bytes.load(Ordering::Acquire) != bytes
            {
                stable = false;
                break;
            }
        }
        if stable
            && observation.write_pending.load(Ordering::Acquire)
            && observation.write_calls.load(Ordering::Acquire) == calls
            && observation.write_bytes.load(Ordering::Acquire) == bytes
        {
            return (Some((calls, bytes)), saw_exit);
        }
    }
    (None, saw_exit)
}

fn manager_and_app(
    observations: Vec<Arc<Observation>>,
) -> (PtyManager, tauri::App<tauri::test::MockRuntime>) {
    let app = tauri::test::mock_app();
    let manager = PtyManager::new(Arc::new(ObservedBackend {
        observations: Mutex::new(observations.into()),
    }));
    (manager, app)
}

fn spawn_fixture(
    manager: &PtyManager,
    app: &tauri::App<tauri::test::MockRuntime>,
    dir: &Path,
    mode: &str,
) -> (
    String,
    u32,
    std::sync::Weak<super::super::session::PtySession>,
) {
    let mut environment = BTreeMap::new();
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "DYLD_LIBRARY_PATH",
        "LD_LIBRARY_PATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            environment.insert(key.into(), value.to_string_lossy().into_owned());
        }
    }
    environment.insert(FIXTURE_ENV.into(), dir.to_string_lossy().into_owned());
    environment.insert(FIXTURE_MODE.into(), mode.into());
    let request = PtySpawnRequest {
        prepared: PreparedSpawnRequest {
            executable: std::env::current_exe()
                .expect("test executable path")
                .to_string_lossy()
                .into_owned(),
            arguments: vec!["--exact".into(), FIXTURE_TEST.into(), "--nocapture".into()],
            cwd: None,
            environment,
            shell_type: None,
            login_shell: false,
            cwd_fallback: false,
        },
        columns: 80,
        rows: 24,
        sudo: None,
    };
    let response = manager
        .spawn(app.handle().clone(), request)
        .expect("production manager spawns fixture");
    let session = manager
        .sessions
        .read()
        .unwrap()
        .get(&response.id)
        .expect("spawn registers session")
        .clone();
    let weak = Arc::downgrade(&session);
    drop(session);
    (response.id, response.pid, weak)
}

fn assert_reader_released(
    observation: &Observation,
    weak: &std::sync::Weak<super::super::session::PtySession>,
) {
    wait_until(
        || observation.reader_dropped.load(Ordering::Acquire),
        "production reader drop",
    );
    wait_until(
        || weak.upgrade().is_none(),
        "final Session reference release",
    );
}

fn wait_result(observation: &Observation) -> (u32, Option<String>) {
    wait_until(
        || observation.child_wait_done.load(Ordering::Acquire),
        "production child wait result",
    );
    observation
        .child_wait_result
        .lock()
        .unwrap()
        .clone()
        .expect("child wait recorded its real result")
}

fn wait_native_read_blocked(observation: &Observation, what: &str) {
    let deadline = Instant::now() + OBSERVE_FOR;
    while Instant::now() < deadline {
        if observation.read_calls.load(Ordering::Acquire) < 2
            || !observation.read_pending.load(Ordering::Acquire)
        {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let calls = observation.read_calls.load(Ordering::Acquire);
        thread::sleep(Duration::from_millis(30));
        if observation.read_pending.load(Ordering::Acquire)
            && observation.read_calls.load(Ordering::Acquire) == calls
        {
            return;
        }
    }
    assert!(
        observation.read_calls.load(Ordering::Acquire) >= 2
            && observation.read_pending.load(Ordering::Acquire),
        "timed out waiting for {what}: native Read is not pending"
    );
    panic!("timed out waiting for {what}: no stable 30 ms pending Read sample");
}

fn ack_initial_output(manager: &PtyManager, id: &str, observation: &Observation) {
    wait_native_read_blocked(observation, "native Read blocks after initial PTY output");
    let bytes = observation.read_bytes.load(Ordering::Acquire) as usize;
    assert!(bytes > 0, "real helper test harness output was observed");
    manager.ack(id, bytes).expect("ack observed output bytes");
}

#[test]
fn discard_quiet_native_session_releases_production_reader() {
    let name =
        "pty::manager::reader_tests::discard_quiet_native_session_releases_production_reader";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let control_observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![
        Arc::clone(&observation),
        Arc::clone(&control_observation),
    ]);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "quiet");
    owned.track(pid);
    manager
        .attach(&app.handle(), &id)
        .expect("attach before fixture output");
    wait_for_file(&temp.path().join("ready"), "quiet helper readiness");
    ack_initial_output(&manager, &id, &observation);
    wait_until(
        || observation.read_pending.load(Ordering::Acquire),
        "native PTY Read blocked on quiet child",
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("pid")).unwrap().trim(),
        pid.to_string()
    );

    // A second real session proves discard is scoped to its requested ID.
    let control_dir = tempfile::tempdir().expect("private control fixture directory");
    let (control_id, control_pid, control_weak) =
        spawn_fixture(&manager, &app, control_dir.path(), "quiet");
    owned.track(control_pid);
    manager
        .attach(&app.handle(), &control_id)
        .expect("attach control before fixture output");
    wait_for_file(
        &control_dir.path().join("ready"),
        "control helper readiness",
    );
    ack_initial_output(&manager, &control_id, &control_observation);
    manager.discard(&id).expect("discard quiet session");
    let result = wait_result(&observation);
    owned.forget(pid);
    assert!(
        result.1.is_some() || result.0 != 0,
        "discarded direct child must report its actual termination"
    );
    assert!(!manager.exists(&id), "discard removes restore eligibility");
    assert_reader_released(&observation, &weak);
    assert!(
        manager.exists(&control_id),
        "discard preserves another session record"
    );
    assert!(
        manager.is_alive(&control_id),
        "independent child remains alive"
    );
    assert!(
        control_weak.upgrade().is_some(),
        "control session remains strongly owned by manager"
    );
    touch(&control_dir.path().join("release"));
    wait_result(&control_observation);
    owned.forget(control_pid);
    wait_until(
        || control_observation.read_eof.load(Ordering::Acquire),
        "control reader EOF",
    );
    wait_until(
        || !manager.exists(&control_id),
        "control waiter has published natural exit before detach",
    );
    manager
        .detach(&control_id)
        .expect("natural control exit completes after detach");
    assert_reader_released(&control_observation, &control_weak);
}

#[test]
fn discard_quiet_native_session_unblocks_actual_pending_writer() {
    let name =
        "pty::manager::reader_tests::discard_quiet_native_session_unblocks_actual_pending_writer";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![Arc::clone(&observation)]);
    let (output_listener, exit_listener, events) = listen_pty_events(&app);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "blocked-write-quiet");
    owned.track(pid);
    manager
        .attach(&app.handle(), &id)
        .expect("attach before quiet writer fixture starts");
    wait_for_file(
        &temp.path().join("ready"),
        "blocked-writer helper readiness",
    );
    drain_startup_output(&manager, &id, &observation, &events);
    let payload = vec![b'Q'; BLOCKED_WRITE_BYTES];

    let (probe, child_exited_early, write_result, elapsed, discard_result) =
        thread::scope(|scope| {
            let writer = scope.spawn(|| {
                manager
                    .write(&id, &payload)
                    .map_err(|error| error.to_string())
            });
            let (probe, child_exited_early) =
                observe_stable_blocked_write(&manager, &id, &observation, &events);
            let discarded_at = Instant::now();
            let discard_result = manager.discard(&id).map_err(|error| error.to_string());
            let write_result = writer.join().expect("manager.write thread did not panic");
            (
                probe,
                child_exited_early,
                write_result,
                discarded_at.elapsed(),
                discard_result,
            )
        });

    discard_result.expect("discard unblocks the pending native writer");
    assert!(
        elapsed < OBSERVE_FOR,
        "discard and manager.write completion remain bounded"
    );
    assert!(
        probe.is_some(),
        "fixture limitation: this platform's PTY accepted the complete 2 MiB payload without a stable pending native write"
    );
    assert!(
        !child_exited_early,
        "quiet child remains alive until discard"
    );
    assert!(
        write_result.is_err(),
        "discard interrupts the incomplete manager.write request"
    );
    assert!(
        observation.write_bytes.load(Ordering::Acquire) < payload.len() as u64,
        "the actual native writer did not complete the finite payload"
    );
    let result = wait_result(&observation);
    owned.forget(pid);
    assert!(
        result.1.is_some() || result.0 != 0,
        "discarded child wait records its actual termination"
    );
    assert!(!manager.exists(&id), "discard removes restore eligibility");
    assert!(
        !manager.sessions.read().unwrap().contains_key(&id),
        "discard removes the session from the manager"
    );
    assert_reader_released(&observation, &weak);
    app.unlisten(output_listener);
    app.unlisten(exit_listener);
}

#[test]
fn discard_native_session_with_retained_descendant_releases_reader() {
    let name = "pty::manager::reader_tests::discard_native_session_with_retained_descendant_releases_reader";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![Arc::clone(&observation)]);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "descendant");
    owned.track(pid);
    manager
        .attach(&app.handle(), &id)
        .expect("attach before descendant fixture output");
    wait_for_file(
        &temp.path().join("ready"),
        "parent and retained descendant readiness",
    );
    ack_initial_output(&manager, &id, &observation);
    wait_until(
        || observation.read_pending.load(Ordering::Acquire),
        "native PTY Read blocked with slave retained",
    );
    let descendant_pid: u32 = fs::read_to_string(temp.path().join("descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    owned.track(descendant_pid);
    assert!(
        descendant_pid > 0,
        "descendant inherited the PTY handles and wrote its PID marker"
    );
    let heartbeat_path = temp.path().join("descendant.alive");
    let first_heartbeat = wait_for_heartbeat_progress(
        &heartbeat_path,
        0,
        "initial descendant heartbeat contains a complete numeric value",
    );
    let stable_heartbeat = wait_for_heartbeat_progress(
        &heartbeat_path,
        first_heartbeat,
        "descendant heartbeat advances before discard",
    );
    #[cfg(windows)]
    let _ = stable_heartbeat;
    assert!(
        observation.read_pending.load(Ordering::Acquire),
        "production native Read is pending before discard"
    );

    manager.discard(&id).expect("discard parent session");
    #[cfg(unix)]
    let _post_discard_heartbeat = wait_for_heartbeat_progress(
        &heartbeat_path,
        stable_heartbeat,
        "Unix direct-child discard leaves the descendant running",
    );
    let result = wait_result(&observation);
    owned.forget(pid);
    assert!(
        result.1.is_some() || result.0 != 0,
        "direct child wait records termination"
    );
    assert!(!manager.exists(&id), "discard removes restore eligibility");
    assert_reader_released(&observation, &weak);
    // The descendant is deliberately left running until after reader/final-Session assertions.
    touch(&temp.path().join("descendant.release"));
    #[cfg(unix)]
    wait_for_file(
        &temp.path().join("descendant.done"),
        "descendant release after behavior assertions",
    );
    #[cfg(unix)]
    owned.forget(descendant_pid);
}

#[test]
fn natural_child_exit_finishes_production_reader_after_detach() {
    let name =
        "pty::manager::reader_tests::natural_child_exit_finishes_production_reader_after_detach";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![Arc::clone(&observation)]);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "natural");
    owned.track(pid);
    manager
        .attach(&app.handle(), &id)
        .expect("attach before natural fixture output");
    wait_for_file(&temp.path().join("ready"), "natural helper readiness");
    ack_initial_output(&manager, &id, &observation);
    wait_until(
        || observation.read_pending.load(Ordering::Acquire),
        "native PTY Read blocked before release",
    );
    touch(&temp.path().join("release"));

    let result = wait_result(&observation);
    owned.forget(pid);
    assert_eq!(
        result,
        (0, None),
        "natural exit preserves helper's successful status"
    );
    wait_until(
        || observation.read_eof.load(Ordering::Acquire),
        "real native reader EOF after child exit",
    );
    assert!(
        observation.read_calls.load(Ordering::Acquire) >= 1,
        "reader performed native reads"
    );
    wait_until(
        || observation.reader_dropped.load(Ordering::Acquire),
        "production reader drop before detach",
    );
    wait_until(
        || !manager.exists(&id),
        "production waiter has published and emitted natural exit before detach",
    );
    manager
        .detach(&id)
        .expect("delivered natural exit completes after detach");
    assert_reader_released(&observation, &weak);
    assert!(
        !manager.exists(&id),
        "detached completed session is removed"
    );
}

#[cfg(unix)]
#[test]
fn natural_parent_exit_with_retained_descendant_emits_tail_and_releases_reader() {
    let name = "pty::manager::reader_tests::natural_parent_exit_with_retained_descendant_emits_tail_and_releases_reader";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![Arc::clone(&observation)]);
    let (output_listener, exit_listener, events) = listen_pty_events(&app);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "natural-descendant-tail");
    owned.track(pid);
    manager
        .attach(&app.handle(), &id)
        .expect("attach before fixture output");
    wait_for_file(
        &temp.path().join("ready"),
        "parent and descendant readiness",
    );
    drain_startup_output(&manager, &id, &observation, &events);
    let descendant_pid: u32 = fs::read_to_string(temp.path().join("descendant.pid"))
        .expect("descendant PID marker")
        .trim()
        .parse()
        .expect("numeric descendant PID");
    owned.track(descendant_pid);
    let heartbeat = wait_for_heartbeat_progress(
        &temp.path().join("descendant.alive"),
        0,
        "descendant is alive before parent release",
    );

    touch(&temp.path().join("tail.release"));
    let mut emitted = Vec::new();
    while !emitted
        .windows(NATURAL_TAIL.len())
        .any(|window| window == NATURAL_TAIL)
    {
        match receive_pty_event(&events, "parent tail through pty:output") {
            PtyEvent::Output(output) => {
                emitted.extend_from_slice(&output.data);
                ack_output(&manager, &id, output);
            }
            PtyEvent::Exit(_) => panic!("exit was emitted before required tail output"),
        }
    }
    let result = wait_result(&observation);
    owned.forget(pid);
    assert_eq!(result, (0, None), "natural parent exits successfully");
    let exit = loop {
        match receive_pty_event(&events, "normal pty:exit publication") {
            PtyEvent::Exit(exit) if exit.id == id => break exit,
            PtyEvent::Output(output) => ack_output(&manager, &id, output),
            PtyEvent::Exit(_) => panic!("exit event belongs to another PTY"),
        }
    };
    assert_eq!(exit.exit_code, Some(0));
    assert_eq!(exit.signal, None);
    wait_until(
        || observation.reader_dropped.load(Ordering::Acquire),
        "production reader handle drops before detach",
    );
    manager.detach(&id).expect("detach delivered natural exit");
    let weak_deadline = Instant::now() + OBSERVE_FOR;
    while weak.upgrade().is_some() && Instant::now() < weak_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let registry_refs = manager
        .sessions
        .read()
        .unwrap()
        .get(&id)
        .map(Arc::strong_count);
    let weak_refs = weak.upgrade().map(|session| Arc::strong_count(&session));
    assert!(
        weak_refs.is_none(),
        "final Session Arc release after detach; registry refs={registry_refs:?}, weak refs={weak_refs:?}"
    );
    assert!(
        !manager.exists(&id),
        "completed parent is no longer registered"
    );
    wait_for_heartbeat_progress(
        &temp.path().join("descendant.alive"),
        heartbeat,
        "descendant remains alive through all reader and detach assertions",
    );

    touch(&temp.path().join("descendant.release"));
    wait_for_file(
        &temp.path().join("descendant.done"),
        "descendant cleanup after behavior assertions",
    );
    owned.forget(descendant_pid);
    app.unlisten(output_listener);
    app.unlisten(exit_listener);
}

#[cfg(unix)]
#[test]
fn live_child_round_trips_large_binary_data_through_native_pty() {
    let name =
        "pty::manager::reader_tests::live_child_round_trips_large_binary_data_through_native_pty";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![Arc::clone(&observation)]);
    let (output_listener, exit_listener, events) = listen_pty_events(&app);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "roundtrip");
    owned.track(pid);
    manager
        .attach(&app.handle(), &id)
        .expect("attach before fixture output");
    wait_for_file(&temp.path().join("ready"), "raw-mode echo child readiness");
    drain_startup_output(&manager, &id, &observation, &events);

    let expected = (0..ROUNDTRIP_BYTES)
        .map(|index| index.wrapping_mul(37).wrapping_add(index >> 8) as u8)
        .collect::<Vec<_>>();
    let mut actual = Vec::with_capacity(expected.len());
    let write_result = thread::scope(|scope| {
        let writer = scope.spawn(|| manager.write(&id, &expected));
        while actual.len() < expected.len() {
            match receive_pty_event(&events, "binary echo via pty:output") {
                PtyEvent::Output(output) => {
                    ack_output(&manager, &id, output.clone());
                    actual.extend_from_slice(&output.data);
                    assert!(
                        actual.len() <= expected.len(),
                        "PTY did not echo bytes beyond the requested payload"
                    );
                }
                PtyEvent::Exit(_) => panic!("child exited before completing binary echo"),
            }
        }
        writer.join().expect("manager.write thread did not panic")
    });
    write_result.expect("production manager.write delivered all binary bytes");
    assert_eq!(actual.len(), ROUNDTRIP_BYTES);
    assert_eq!(actual, expected, "native child echoed each byte unchanged");
    assert!(manager.is_alive(&id), "child remains live after exact echo");

    touch(&temp.path().join("release"));
    assert_eq!(wait_result(&observation), (0, None));
    owned.forget(pid);
    wait_until(
        || observation.read_eof.load(Ordering::Acquire),
        "native reader receives real EOF after child exit",
    );
    let exit = loop {
        match receive_pty_event(&events, "normal pty:exit after binary echo") {
            PtyEvent::Exit(exit) if exit.id == id => break exit,
            PtyEvent::Output(output) => ack_output(&manager, &id, output),
            PtyEvent::Exit(_) => panic!("exit event belongs to another PTY"),
        }
    };
    assert_eq!(exit.exit_code, Some(0));
    assert_eq!(exit.signal, None);
    manager.detach(&id).expect("detach delivered natural exit");
    assert_reader_released(&observation, &weak);
    app.unlisten(output_listener);
    app.unlisten(exit_listener);
}

#[test]
fn natural_exit_before_late_attach_preserves_complete_tail_output() {
    let name = "pty::manager::reader_tests::natural_exit_before_late_attach_preserves_complete_tail_output";
    if !crate::pty::test_support::run_bounded(name) {
        return;
    }

    let temp = tempfile::tempdir().expect("private fixture directory");
    let mut owned = OwnedPids::new(temp.path().join("descendant.pid"));
    let observation = Arc::new(Observation::default());
    let (manager, app) = manager_and_app(vec![Arc::clone(&observation)]);
    let (output_listener, exit_listener, events) = listen_pty_events(&app);
    let (id, pid, weak) = spawn_fixture(&manager, &app, temp.path(), "late-attach-tail");
    owned.track(pid);

    wait_for_file(&temp.path().join("ready"), "late-attach fixture readiness");
    wait_until(
        || observation.read_bytes.load(Ordering::Acquire) > 0,
        "first detached PTY read",
    );
    assert!(
        matches!(events.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "detached output is not published before attach"
    );

    touch(&temp.path().join("tail.release"));
    assert_eq!(
        wait_result(&observation),
        (0, None),
        "parent exits naturally"
    );
    owned.forget(pid);
    thread::sleep(Duration::from_millis(350));
    assert!(
        matches!(events.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "natural-exit output and exit stay unpublished before late attach"
    );
    manager
        .attach(&app.handle(), &id)
        .expect("attach after the natural child exit");

    let mut emitted = Vec::new();
    let exit = loop {
        match receive_pty_event(&events, "late-attach PTY output and exit") {
            PtyEvent::Output(output) => {
                assert_eq!(output.id, id);
                emitted.extend_from_slice(&output.data);
                ack_output(&manager, &id, output);
            }
            PtyEvent::Exit(exit) if exit.id == id => break exit,
            PtyEvent::Exit(_) => panic!("exit event belongs to another PTY"),
        }
    };
    assert!(
        emitted
            .windows(LATE_ATTACH_TAIL.len())
            .any(|window| window == LATE_ATTACH_TAIL),
        "late attach must deliver every byte of the fixed natural-exit tail"
    );
    assert_eq!(exit.exit_code, Some(0));
    assert_eq!(exit.signal, None);
    wait_until(
        || observation.reader_dropped.load(Ordering::Acquire),
        "production reader handle drops after late attach",
    );
    manager
        .detach(&id)
        .expect("detach after delivered natural exit");
    assert_reader_released(&observation, &weak);
    assert!(!manager.exists(&id), "completed session is removed");
    app.unlisten(output_listener);
    app.unlisten(exit_listener);
}

#[test]
fn reader_fixture_process() {
    let Ok(directory) = std::env::var(FIXTURE_ENV) else {
        return;
    };
    let dir = PathBuf::from(directory);
    let mode = std::env::var(FIXTURE_MODE).expect("fixture mode set");
    assert!(
        std::io::stdin().is_terminal(),
        "fixture stdin is the PTY slave"
    );
    assert!(
        std::io::stdout().is_terminal(),
        "fixture stdout is the PTY slave"
    );
    assert!(
        std::io::stderr().is_terminal(),
        "fixture stderr is the PTY slave"
    );
    if mode != "retained-descendant" {
        fs::write(dir.join("pid"), std::process::id().to_string()).expect("write direct PID");
    }

    if mode == "descendant" || mode == "natural-descendant-tail" {
        let descendant = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", FIXTURE_TEST, "--nocapture"])
            .env(FIXTURE_ENV, &dir)
            .env(FIXTURE_MODE, "retained-descendant")
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start test-owned terminal descendant");
        fs::write(dir.join("descendant.pid"), descendant.id().to_string())
            .expect("record descendant PID for panic cleanup");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !dir.join("descendant.ready").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            dir.join("descendant.ready").exists(),
            "descendant reports terminal readiness"
        );
    }

    #[cfg(unix)]
    if mode == "roundtrip" {
        set_raw_stdin().expect("put fixture PTY into raw mode");
        touch(&dir.join("ready"));
        let mut received = vec![0; ROUNDTRIP_BYTES];
        std::io::stdin()
            .read_exact(&mut received)
            .expect("read the complete binary payload from the PTY");
        std::io::stdout()
            .write_all(&received)
            .expect("write the exact binary payload to the PTY");
        std::io::stdout().flush().expect("flush binary echo");
        wait_fixture_release(&dir.join("release"));
        return;
    }

    if mode == "blocked-write-quiet" {
        #[cfg(unix)]
        set_raw_stdin().expect("put quiet blocked-writer PTY into raw mode");
        touch(&dir.join("ready"));
        wait_fixture_release(&dir.join("release"));
        return;
    }

    if mode == "late-attach-tail" {
        touch(&dir.join("ready"));
        wait_fixture_release(&dir.join("tail.release"));
        std::io::stdout()
            .write_all(&LATE_ATTACH_TAIL)
            .expect("write fixed ASCII tail before natural exit");
        std::io::stdout().flush().expect("flush late-attach tail");
        return;
    }

    if mode == "natural-descendant-tail" {
        touch(&dir.join("ready"));
        wait_fixture_release(&dir.join("tail.release"));
        std::io::stdout()
            .write_all(NATURAL_TAIL)
            .expect("write fixed tail before natural parent exit");
        std::io::stdout().flush().expect("flush natural tail");
        return;
    }

    if mode == "retained-descendant" {
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
        fs::write(dir.join("descendant.pid"), std::process::id().to_string())
            .expect("descendant PID marker");
        fs::write(dir.join("descendant.ready"), b"ready").expect("descendant readiness marker");
        wait_descendant_release(&dir);
        return;
    }

    touch(&dir.join("ready"));
    wait_fixture_release(&dir.join("release"));
    if mode == "descendant" {
        // The descendant exits through its own bounded release marker after assertions.
        wait_until(
            || dir.join("descendant.done").exists(),
            "descendant completion marker",
        );
    }
}

fn wait_fixture_release(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "fixture's fixed 60-second lifetime expired");
}

fn wait_descendant_release(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut heartbeat = 0u64;
    while !dir.join("descendant.release").exists() && Instant::now() < deadline {
        heartbeat += 1;
        fs::write(dir.join("descendant.alive"), heartbeat.to_string())
            .expect("descendant heartbeat remains writable");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        dir.join("descendant.release").exists(),
        "descendant lifetime is bounded to 60 seconds"
    );
    touch(&dir.join("descendant.done"));
}

#[cfg(unix)]
fn set_raw_stdin() -> io::Result<()> {
    let mut settings = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, settings.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let mut settings = unsafe { settings.assume_init() };
    unsafe { libc::cfmakeraw(&mut settings) };
    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &settings) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
