//! Best-effort power-save inhibit for long file transfers.
//!
//! Mirrors the Electron host's `powerSaveBlocker.start('prevent-app-suspension')`
//! while an SFTP or file transfer runs. Every acquire is reference counted: the
//! first acquire takes one native hold, the last release drops it. Native
//! failures are logged and never fail the caller, because inhibit is only a
//! convenience for the user's transfer.

use std::{collections::BTreeSet, sync::Mutex};

/// Upper bound on outstanding ids, so a renderer that never releases cannot
/// grow the bookkeeping without limit.
pub const MAX_OUTSTANDING_INHIBITS: usize = 1024;

/// A platform backend that takes one native power-save hold. Dropping the
/// returned hold releases it.
pub trait InhibitBackend {
    type Hold: Send;

    fn acquire(&self) -> Result<Self::Hold, String>;
}

struct State<H> {
    last_id: u64,
    outstanding: BTreeSet<u64>,
    hold: Option<H>,
}

pub struct PowerInhibitor<B: InhibitBackend> {
    backend: B,
    state: Mutex<State<B::Hold>>,
}

impl<B: InhibitBackend> PowerInhibitor<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            state: Mutex::new(State {
                last_id: 0,
                outstanding: BTreeSet::new(),
                hold: None,
            }),
        }
    }

    /// Registers one interest in inhibiting power save and returns its id.
    /// A native failure still returns an id, so the caller's release stays
    /// balanced; the native hold is retried on the next acquire.
    pub fn acquire(&self) -> Result<u64, String> {
        let mut state = self.lock();
        if state.outstanding.len() >= MAX_OUTSTANDING_INHIBITS {
            return Err(format!(
                "too many outstanding power inhibits (limit {MAX_OUTSTANDING_INHIBITS})"
            ));
        }
        if state.hold.is_none() {
            match self.backend.acquire() {
                Ok(hold) => state.hold = Some(hold),
                Err(error) => eprintln!("power inhibit unavailable: {error}"),
            }
        }
        state.last_id += 1;
        let id = state.last_id;
        state.outstanding.insert(id);
        Ok(id)
    }

    /// Releases one id. Unknown or already released ids are ignored and return
    /// false. The native hold is dropped once, when the last id is released.
    pub fn release(&self, id: u64) -> bool {
        let dropped_hold = {
            let mut state = self.lock();
            if !state.outstanding.remove(&id) {
                return false;
            }
            if state.outstanding.is_empty() {
                state.hold.take()
            } else {
                None
            }
        };
        // Dropping outside the lock keeps slow native teardown from blocking acquires.
        drop(dropped_hold);
        true
    }

    #[cfg(test)]
    pub fn outstanding(&self) -> usize {
        self.lock().outstanding.len()
    }

    #[cfg(test)]
    pub fn is_holding(&self) -> bool {
        self.lock().hold.is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<B::Hold>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(target_os = "linux")]
mod native {
    use std::time::Duration;

    use dbus::{arg::OwnedFd, blocking::Connection};

    use super::InhibitBackend;

    const TIMEOUT: Duration = Duration::from_secs(2);
    const APP_NAME: &str = "Tabby RS";
    const REASON: &str = "File transfer in progress";

    pub struct NativeBackend;

    /// The variants exist only to be dropped, which releases the native hold.
    #[allow(dead_code)]
    pub enum Hold {
        /// Closing this descriptor releases the logind sleep inhibitor.
        Logind(OwnedFd),
        /// Calls UnInhibit on the session bus when dropped.
        ScreenSaver(ScreenSaverHold),
    }

    pub struct ScreenSaverHold {
        connection: Connection,
        cookie: u32,
    }

    impl Drop for ScreenSaverHold {
        fn drop(&mut self) {
            let proxy = self.connection.with_proxy(
                "org.freedesktop.ScreenSaver",
                "/org/freedesktop/ScreenSaver",
                TIMEOUT,
            );
            if let Err(error) = proxy.method_call::<(), _, _, _>(
                "org.freedesktop.ScreenSaver",
                "UnInhibit",
                (self.cookie,),
            ) {
                eprintln!("failed to release ScreenSaver inhibit: {error}");
            }
        }
    }

    fn logind_inhibit() -> Result<OwnedFd, String> {
        let connection = Connection::new_system().map_err(|error| error.to_string())?;
        let proxy =
            connection.with_proxy("org.freedesktop.login1", "/org/freedesktop/login1", TIMEOUT);
        let (fd,): (OwnedFd,) = proxy
            .method_call(
                "org.freedesktop.login1.Manager",
                "Inhibit",
                ("sleep:idle", APP_NAME, REASON, "block"),
            )
            .map_err(|error| error.to_string())?;
        Ok(fd)
    }

    fn screensaver_inhibit() -> Result<ScreenSaverHold, String> {
        let connection = Connection::new_session().map_err(|error| error.to_string())?;
        let proxy = connection.with_proxy(
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            TIMEOUT,
        );
        let (cookie,): (u32,) = proxy
            .method_call("org.freedesktop.ScreenSaver", "Inhibit", (APP_NAME, REASON))
            .map_err(|error| error.to_string())?;
        Ok(ScreenSaverHold { connection, cookie })
    }

    impl InhibitBackend for NativeBackend {
        type Hold = Hold;

        fn acquire(&self) -> Result<Hold, String> {
            match logind_inhibit() {
                Ok(fd) => Ok(Hold::Logind(fd)),
                Err(logind_error) => screensaver_inhibit()
                    .map(Hold::ScreenSaver)
                    .map_err(|screensaver_error| {
                        format!(
                            "login1 Inhibit failed ({logind_error}); ScreenSaver Inhibit failed ({screensaver_error})"
                        )
                    }),
            }
        }
    }
}

#[cfg(windows)]
mod native {
    use std::{
        sync::{mpsc, OnceLock},
        thread,
    };

    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };

    use super::InhibitBackend;

    pub struct NativeBackend;

    /// Releases the system-required state when dropped.
    pub struct Hold;

    /// Execution state is per thread, so one worker thread owns it and applies
    /// requests in order. `true` requires the system to stay awake.
    fn worker() -> &'static Result<mpsc::Sender<bool>, String> {
        static WORKER: OnceLock<Result<mpsc::Sender<bool>, String>> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<bool>();
            thread::Builder::new()
                .name("tabby-power-inhibit".into())
                .spawn(move || {
                    for require in receiver {
                        let flags = if require {
                            ES_CONTINUOUS | ES_SYSTEM_REQUIRED
                        } else {
                            ES_CONTINUOUS
                        };
                        // SAFETY: the call takes only a flag value and touches no memory.
                        if unsafe { SetThreadExecutionState(flags) } == 0 {
                            eprintln!("SetThreadExecutionState failed");
                        }
                    }
                })
                .map(|_| sender)
                .map_err(|error| format!("failed to start power inhibit thread: {error}"))
        })
    }

    fn request(require: bool) -> Result<(), String> {
        match worker() {
            Ok(sender) => sender
                .send(require)
                .map_err(|_| "power inhibit thread stopped".to_owned()),
            Err(error) => Err(error.clone()),
        }
    }

    impl InhibitBackend for NativeBackend {
        type Hold = Hold;

        fn acquire(&self) -> Result<Hold, String> {
            request(true)?;
            Ok(Hold)
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            if let Err(error) = request(false) {
                eprintln!("failed to release system-required state: {error}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::process::{Child, Command, Stdio};

    use super::InhibitBackend;

    pub struct NativeBackend;

    /// A `caffeinate -i` child. `-w` makes it exit when this process exits, so a
    /// crash cannot leave the Mac awake.
    pub struct Hold(Child);

    impl InhibitBackend for NativeBackend {
        type Hold = Hold;

        fn acquire(&self) -> Result<Hold, String> {
            let pid = std::process::id().to_string();
            Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", pid.as_str()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(Hold)
                .map_err(|error| format!("failed to start caffeinate: {error}"))
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod native {
    use super::InhibitBackend;

    pub struct NativeBackend;

    impl InhibitBackend for NativeBackend {
        type Hold = ();

        fn acquire(&self) -> Result<(), String> {
            Err("power inhibit is not implemented on this platform".into())
        }
    }
}

pub use native::NativeBackend;

/// The process-wide inhibitor used by the `power.inhibit*` commands.
pub type NativePowerInhibitor = PowerInhibitor<NativeBackend>;

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    use super::{InhibitBackend, PowerInhibitor, MAX_OUTSTANDING_INHIBITS};

    #[derive(Clone, Default)]
    struct FakeBackend {
        starts: Arc<AtomicUsize>,
        releases: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }

    struct FakeHold {
        releases: Arc<AtomicUsize>,
    }

    impl Drop for FakeHold {
        fn drop(&mut self) {
            self.releases.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl InhibitBackend for FakeBackend {
        type Hold = FakeHold;

        fn acquire(&self) -> Result<FakeHold, String> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err("denied by fake".into());
            }
            Ok(FakeHold {
                releases: self.releases.clone(),
            })
        }
    }

    fn inhibitor() -> (PowerInhibitor<FakeBackend>, FakeBackend) {
        let backend = FakeBackend::default();
        (PowerInhibitor::new(backend.clone()), backend)
    }

    #[test]
    fn first_acquire_starts_one_native_hold_and_ids_increase() {
        let (power, backend) = inhibitor();
        let first = power.acquire().unwrap();
        let second = power.acquire().unwrap();
        assert!(second > first);
        assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
        assert!(power.is_holding());
        assert_eq!(power.outstanding(), 2);
    }

    #[test]
    fn last_release_drops_the_native_hold_exactly_once() {
        let (power, backend) = inhibitor();
        let a = power.acquire().unwrap();
        let b = power.acquire().unwrap();
        assert!(power.release(a));
        assert_eq!(backend.releases.load(Ordering::SeqCst), 0);
        assert!(power.is_holding());
        assert!(power.release(b));
        assert_eq!(backend.releases.load(Ordering::SeqCst), 1);
        assert!(!power.is_holding());
    }

    #[test]
    fn duplicate_and_unknown_releases_are_ignored() {
        let (power, backend) = inhibitor();
        let id = power.acquire().unwrap();
        assert!(power.release(id));
        assert!(!power.release(id));
        assert!(!power.release(id + 100));
        assert_eq!(backend.releases.load(Ordering::SeqCst), 1);
        assert_eq!(power.outstanding(), 0);
    }

    #[test]
    fn a_new_acquire_after_zero_starts_again() {
        let (power, backend) = inhibitor();
        let id = power.acquire().unwrap();
        power.release(id);
        power.acquire().unwrap();
        assert_eq!(backend.starts.load(Ordering::SeqCst), 2);
        assert_eq!(backend.releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_failure_still_returns_a_balanced_id_and_retries() {
        let (power, backend) = inhibitor();
        backend.fail.store(true, Ordering::SeqCst);
        let id = power
            .acquire()
            .expect("inhibit failure must not fail the caller");
        assert!(!power.is_holding());
        assert!(power.release(id));
        assert_eq!(backend.releases.load(Ordering::SeqCst), 0);

        backend.fail.store(false, Ordering::SeqCst);
        let retried = power.acquire().unwrap();
        assert!(power.is_holding());
        assert!(power.release(retried));
        assert_eq!(backend.starts.load(Ordering::SeqCst), 2);
        assert_eq!(backend.releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn outstanding_ids_are_capped_and_recover_after_release() {
        let (power, backend) = inhibitor();
        let ids: Vec<u64> = (0..MAX_OUTSTANDING_INHIBITS)
            .map(|_| power.acquire().unwrap())
            .collect();
        assert!(power.acquire().is_err());
        assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
        assert!(power.release(ids[0]));
        assert!(power.acquire().is_ok());
        for id in ids.into_iter().skip(1) {
            power.release(id);
        }
    }
}
