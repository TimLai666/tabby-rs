use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, ChildKiller, ExitStatus};

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

#[derive(Default)]
struct Observation {
    read_pending: AtomicBool,
    read_calls: AtomicUsize,
    read_bytes: AtomicU64,
    read_eof: AtomicBool,
    reader_dropped: AtomicBool,
    child_wait_done: AtomicBool,
    child_wait_result: Mutex<Option<(u32, Option<String>)>>,
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
    wait_until(
        || {
            observation.read_calls.load(Ordering::Acquire) >= 2
                && observation.read_pending.load(Ordering::Acquire)
        },
        what,
    );
    let calls = observation.read_calls.load(Ordering::Acquire);
    thread::sleep(Duration::from_millis(30));
    assert!(
        observation.read_pending.load(Ordering::Acquire),
        "{what}: Read remained active at second observation"
    );
    assert_eq!(
        observation.read_calls.load(Ordering::Acquire),
        calls,
        "{what}: no extra Read began"
    );
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

    if mode == "descendant" {
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
