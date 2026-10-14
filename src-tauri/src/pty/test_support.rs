use std::{
    io::{self, Read},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const WORKER_FLAG: &str = "TABBY_PTY_TEST_WORKER";
const DEADLINE: Duration = Duration::from_secs(15);

pub(crate) struct TraceReader<R> {
    inner: R,
    logged_chunks: usize,
}

impl<R> TraceReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            logged_chunks: 0,
        }
    }
}

impl<R: Read> Read for TraceReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = self.inner.read(buffer)?;
        if length > 0 && self.logged_chunks < 4 {
            let sample = &buffer[..length.min(64)];
            let mut hex = String::with_capacity(sample.len() * 2);
            for byte in sample {
                use std::fmt::Write as _;
                write!(&mut hex, "{byte:02X}").expect("writing to a String succeeds");
            }
            eprintln!(
                "PTY_READER_TRACE chunk={} sampled={} hex={hex}",
                self.logged_chunks + 1,
                sample.len()
            );
            self.logged_chunks += 1;
        }
        Ok(length)
    }
}

fn kill_and_reap(
    worker: &mut std::process::Child,
) -> (
    Option<std::process::ExitStatus>,
    std::io::Result<()>,
    std::io::Result<std::process::ExitStatus>,
) {
    #[cfg(windows)]
    let tree_status = {
        let pid = worker.id().to_string();
        Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .ok()
    };
    #[cfg(not(windows))]
    let tree_status = None;

    let kill = worker.kill();
    let reap = worker.wait();
    (tree_status, kill, reap)
}

/// Run an exact unit test in a subprocess so hangs are bounded on every platform.
/// Returns true in the worker and false in the supervising test process.
pub(crate) fn run_bounded(test_name: &str) -> bool {
    if std::env::var(WORKER_FLAG).as_deref() == Ok(test_name) {
        return true;
    }

    let executable = std::env::current_exe().expect("current test executable is available");
    let mut worker = Command::new(executable)
        .args(["--exact", test_name, "--nocapture"])
        .env(WORKER_FLAG, test_name)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("bounded test worker starts");
    let deadline = Instant::now() + DEADLINE;

    loop {
        match worker.try_wait() {
            Ok(Some(_)) => {
                let status = worker.wait().expect("test worker is reaped");
                assert!(
                    status.success(),
                    "bounded worker {test_name} failed: {status}"
                );
                return false;
            }
            Ok(None) => {}
            Err(error) => {
                let (tree_status, kill, reap) = kill_and_reap(&mut worker);
                panic!("could not inspect bounded worker {test_name}: {error}; tree={tree_status:?}; kill={kill:?}; reap={reap:?}");
            }
        }

        if Instant::now() >= deadline {
            let (tree_status, kill, reap) = kill_and_reap(&mut worker);
            panic!("bounded worker {test_name} exceeded 15-second deadline; tree={tree_status:?}; kill={kill:?}; reap={reap:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}
