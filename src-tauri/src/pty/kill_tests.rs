use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    sync::Arc,
    thread,
    time::Duration,
};

use portable_pty::{Child, ChildKiller, MasterPty};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE, STILL_ACTIVE, WAIT_OBJECT_0},
    System::Threading::{
        GetExitCodeProcess, OpenProcess, TerminateProcess, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    },
};

use super::PtyManager;
use crate::pty::{
    backend::{PortablePtyBackend, PtyBackend, SpawnedPty},
    model::{PtySpawnRequest, SpawnSpec},
};
use crate::shell::PreparedSpawnRequest;

const OBSERVE_FOR: Duration = Duration::from_secs(4);

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(pid: u32, access: u32) -> Self {
        let handle = unsafe { OpenProcess(access, 0, pid) };
        assert!(
            !handle.is_null(),
            "OpenProcess({pid}) failed: {}",
            io::Error::last_os_error()
        );
        Self(handle)
    }

    fn wait_exit(&self) -> io::Result<u32> {
        let result = unsafe { WaitForSingleObject(self.0, OBSERVE_FOR.as_millis() as u32) };
        if result != WAIT_OBJECT_0 {
            return Err(if result == u32::MAX {
                io::Error::last_os_error()
            } else {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not exit within four seconds",
                )
            });
        }
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.0, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(code)
    }

    fn is_alive(&self) -> io::Result<bool> {
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.0, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(code == STILL_ACTIVE as u32)
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        let mut exit_code = 0;
        if unsafe { GetExitCodeProcess(self.0, &mut exit_code) } != 0
            && exit_code == STILL_ACTIVE as u32
            && unsafe { TerminateProcess(self.0, 1) } != 0
        {
            let _ = unsafe { WaitForSingleObject(self.0, OBSERVE_FOR.as_millis() as u32) };
        }
        unsafe { CloseHandle(self.0) };
    }
}

fn cleanup_process(process: &ProcessHandle) -> io::Result<()> {
    if process.is_alive()? && unsafe { TerminateProcess(process.0, 1) } == 0 {
        return Err(io::Error::last_os_error());
    }
    process.wait_exit().map(|_| ())
}

struct FixtureCleanup {
    process: ProcessHandle,
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    master: Option<Box<dyn MasterPty + Send>>,
    drain: Option<thread::JoinHandle<io::Result<u64>>>,
}

impl FixtureCleanup {
    fn new(spawned: SpawnedPty) -> (Self, Box<dyn ChildKiller + Send + Sync>) {
        let process = ProcessHandle::open(
            spawned.pid,
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
        );
        let drain = drain_output(spawned.reader);
        (
            Self {
                process,
                child: Some(spawned.child),
                writer: Some(spawned.writer),
                master: Some(spawned.master),
                drain: Some(drain),
            },
            spawned.killer,
        )
    }

    fn process(&self) -> &ProcessHandle {
        &self.process
    }

    fn finish(&mut self, terminate_if_alive: bool) -> io::Result<u64> {
        if terminate_if_alive && self.process.is_alive()? {
            cleanup_process(&self.process)?;
        }
        self.process.wait_exit()?;
        if let Some(mut child) = self.child.take() {
            child.wait()?;
        }
        self.writer.take();
        self.master.take();
        self.drain
            .take()
            .expect("PTY output drainer is present")
            .join()
            .map_err(|_| io::Error::other("PTY output drainer panicked"))?
    }
}

impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        if self.process.is_alive().unwrap_or(false) {
            let _ = cleanup_process(&self.process);
        }
        if self.process.wait_exit().is_ok() {
            if let Some(mut child) = self.child.take() {
                let _ = child.wait();
            }
        }
        self.writer.take();
        self.master.take();
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}

fn command_spec(arguments: &[&str]) -> SpawnSpec {
    let mut environment = BTreeMap::new();
    for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(name.to_owned(), value.to_string_lossy().into_owned());
        }
    }
    let executable = std::env::var_os("COMSPEC")
        .or_else(|| std::env::var_os("ComSpec"))
        .expect("Windows PTY test requires COMSPEC")
        .to_string_lossy()
        .into_owned();
    SpawnSpec {
        executable,
        arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        cwd: None,
        environment,
        columns: 80,
        rows: 24,
        shell_type: None,
        sudo: None,
    }
}

fn drain_output(reader: Box<dyn Read + Send>) -> thread::JoinHandle<io::Result<u64>> {
    thread::spawn(move || io::copy(&mut io::BufReader::new(reader), &mut io::sink()))
}

fn pty_request(arguments: &[&str]) -> PtySpawnRequest {
    let spec = command_spec(arguments);
    PtySpawnRequest {
        prepared: PreparedSpawnRequest {
            executable: spec.executable,
            arguments: spec.arguments,
            cwd: spec.cwd,
            environment: spec.environment,
            shell_type: spec.shell_type,
            login_shell: false,
            cwd_fallback: false,
        },
        columns: spec.columns,
        rows: spec.rows,
        sudo: None,
    }
}

#[test]
fn ordinary_windows_kill_clone_reports_success_and_child_exits() {
    let test_name =
        "pty::manager::kill_tests::ordinary_windows_kill_clone_reports_success_and_child_exits";
    if !crate::pty::test_support::run_bounded(test_name) {
        return;
    }

    let spawned = PortablePtyBackend
        .spawn(&command_spec(&["/d", "/c", "pause"]))
        .expect("real cmd.exe PTY starts");
    let (mut fixture, killer) = FixtureCleanup::new(spawned);
    let initial = fixture.process().is_alive();
    let kill = if matches!(initial, Ok(true)) {
        let mut killer = killer;
        killer.kill()
    } else {
        Err(io::Error::other("child was not alive before kill"))
    };
    let exited = fixture.process().wait_exit();
    let drained = fixture.finish(true);

    assert!(
        matches!(initial, Ok(true)),
        "fixture must be alive before kill: {initial:?}"
    );
    assert!(
        kill.is_ok(),
        "cloned killer should report success: {kill:?}"
    );
    assert!(
        matches!(exited, Ok(1)),
        "child should exit with code 1 after kill: {exited:?}"
    );
    assert!(drained.is_ok(), "PTY output should drain: {drained:?}");
}

#[test]
fn ordinary_windows_kill_clone_preserves_native_failure_after_exit() {
    let test_name =
        "pty::manager::kill_tests::ordinary_windows_kill_clone_preserves_native_failure_after_exit";
    if !crate::pty::test_support::run_bounded(test_name) {
        return;
    }

    let spawned = PortablePtyBackend
        .spawn(&command_spec(&["/d", "/c", "exit 7"]))
        .expect("real cmd.exe PTY starts");
    let (mut fixture, mut killer) = FixtureCleanup::new(spawned);
    let exited = fixture.process().wait_exit();
    let direct_kill = fixture.child.as_mut().expect("child retained").kill();
    let clone_kill = killer.kill();
    let drained = fixture.finish(false);

    assert!(
        matches!(exited, Ok(7)),
        "fixture must exit naturally with code 7: {exited:?}"
    );
    assert!(
        direct_kill.is_ok(),
        "direct child's best-effort kill remains successful"
    );
    assert_eq!(
        clone_kill.err().and_then(|error| error.raw_os_error()),
        Some(ERROR_ACCESS_DENIED as i32)
    );
    assert!(drained.is_ok(), "PTY output should drain: {drained:?}");
}

#[test]
fn ordinary_windows_kill_manager_returns_success_and_preserves_other_session() {
    let test_name = "pty::manager::kill_tests::ordinary_windows_kill_manager_returns_success_and_preserves_other_session";
    if !crate::pty::test_support::run_bounded(test_name) {
        return;
    }

    let app = tauri::test::mock_app();
    let manager = PtyManager::new(Arc::new(PortablePtyBackend));
    let first = manager
        .spawn(app.handle().clone(), pty_request(&["/d", "/c", "pause"]))
        .expect("first real manager session starts");
    let first_process = ProcessHandle::open(
        first.pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
    );
    let second = manager
        .spawn(app.handle().clone(), pty_request(&["/d", "/c", "pause"]))
        .expect("second real manager session starts");
    let second_process = ProcessHandle::open(
        second.pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
    );
    let alive_before = (first_process.is_alive(), second_process.is_alive());
    let killed = manager.kill(&first.id, None);
    let first_exit = first_process.wait_exit();
    let second_alive = second_process.is_alive();

    if first_process.is_alive().unwrap_or(false) {
        let _ = manager.discard(&first.id);
        let _ = cleanup_process(&first_process);
    }
    if second_process.is_alive().unwrap_or(false) {
        let _ = manager.discard(&second.id);
        let _ = cleanup_process(&second_process);
    }
    let first_cleanup = manager.discard(&first.id);
    let second_cleanup = manager.discard(&second.id);

    assert!(
        matches!(alive_before, (Ok(true), Ok(true))),
        "both fixture processes must start alive: {alive_before:?}"
    );
    assert!(
        killed.is_ok(),
        "manager kill should report success: {killed:?}"
    );
    assert!(
        matches!(first_exit, Ok(1)),
        "first child should exit with code 1 after manager kill: {first_exit:?}"
    );
    assert!(
        matches!(second_alive, Ok(true)),
        "killing one session must leave the other alive: {second_alive:?}"
    );
    assert!(
        first_cleanup.is_ok(),
        "first session cleanup should succeed"
    );
    assert!(
        second_cleanup.is_ok(),
        "second session cleanup should succeed"
    );
}
