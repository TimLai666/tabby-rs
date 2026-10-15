#![cfg(all(test, any(target_os = "macos", windows, target_os = "linux")))]

// Framework boundary: Tauri MockRuntime. Native instance transport stays real.
// Parent/worker cases are filled independently from the plugin implementation.

// The native single-instance transport is exercised for real: a primary process
// owns the platform endpoint (macOS Unix socket, Windows hidden window + mutex,
// Linux session-bus name) and a secondary process hands its actual argv/cwd to
// the owning primary. Only Tauri's runtime boundary is mocked.
//
// Both apps in a pair share one freshly generated identifier; they are separated
// by the plugin's `instance_id` override, which is derived per portable root
// through `identity::portable_instance_id`. When the native transport ignores
// that override, the two primaries collide and this test fails for real instead
// of passing against a shim.

#[cfg(target_os = "linux")]
use std::sync::mpsc;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use tauri::{
    plugin::TauriPlugin,
    test::{mock_builder, mock_context, noop_assets, MockRuntime},
};

use tauri_plugin_single_instance as single_instance;

#[cfg(windows)]
#[path = "windows_instance_transport_tests.rs"]
mod windows_message_tests;

const WORKER_MARKER: &str = "TABBY_RS_INSTANCE_TRANSPORT_WORKER";
const WORKER_IDENTIFIER: &str = "TABBY_RS_INSTANCE_TRANSPORT_IDENTIFIER";
const WORKER_OVERRIDE: &str = "TABBY_RS_INSTANCE_TRANSPORT_OVERRIDE";
const WORKER_RECEIPT: &str = "TABBY_RS_INSTANCE_TRANSPORT_RECEIPT";
const WORKER_STOP: &str = "TABBY_RS_INSTANCE_TRANSPORT_STOP";
const WORKER_READY: &str = "TABBY_RS_INSTANCE_TRANSPORT_READY";
const WORKER_ENTERING: &str = "TABBY_RS_INSTANCE_TRANSPORT_ENTERING";
const WORKER_CONFIG: &str = "TABBY_RS_INSTANCE_TRANSPORT_CONFIG";
const WORKER_TEST: &str = "instance_transport_tests::native_instance_worker";

const POLL: Duration = Duration::from_millis(20);
const PRIMARY_READY_TIMEOUT: Duration = Duration::from_secs(8);
const WORKER_LIFETIME: Duration = Duration::from_secs(90);
const OVERALL_DEADLINE: Duration = Duration::from_secs(60);

#[test]
fn native_instance_transport_is_scoped_and_cleanup_preserves_other_instances() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private instance namespace");

    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    let control_identifier = identifier.clone();
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);
    assert_ne!(control_identifier, crate::identity::APP_IDENTIFIER);

    let id_a = derive_instance_id(&owned_root(base.path(), "a"), &identifier);
    let id_b = derive_instance_id(&owned_root(base.path(), "b"), &identifier);
    assert_ne!(
        id_a, id_b,
        "independent portable roots must not share an instance identity"
    );

    let mut endpoints = EndpointGuard::default();
    endpoints.track(&id_a);
    endpoints.track(&id_b);
    endpoints.track(&identifier);

    let session_bus = OwnedSessionBus::ensure();
    let bus = session_bus.address();

    let cwd_a2 = owned_cwd(base.path(), "a2");
    let cwd_b2 = owned_cwd(base.path(), "b2");
    let cwd_b3 = owned_cwd(base.path(), "b3");
    let cwd_c2 = owned_cwd(base.path(), "c2");

    let payload_a2 = format!("A2|\tpayload; with spaces, 中文 and Ünïcödé {token}");
    let payload_b2 = format!("|B2 payload||with spaces, 資料 and semicolons| {token}");
    let payload_b3 = format!("B3 payload; after A stopped | {token}");
    let payload_c2 = format!("CONTROL payload;\twith spaces {token}");

    let mut a = spawn_worker(
        base.path(),
        "a",
        &identifier,
        Some(&id_a),
        &cwd_a2,
        &[],
        bus,
    );
    await_primary_ready(&mut a, deadline);
    let mut b = spawn_worker(
        base.path(),
        "b",
        &identifier,
        Some(&id_b),
        &cwd_b2,
        &[],
        bus,
    );
    await_primary_ready(&mut b, deadline);

    assert!(
        a.guard.try_wait().expect("inspect A").is_none(),
        "primary A stays alive"
    );
    assert!(
        b.guard.try_wait().expect("inspect B").is_none(),
        "primary B stays alive"
    );

    let mut a2 = spawn_worker(
        base.path(),
        "a2",
        &identifier,
        Some(&id_a),
        &cwd_a2,
        &skip(&payload_a2),
        bus,
    );
    run_secondary(&mut a2, deadline);
    wait_for_receipt(
        &a.receipt,
        deadline,
        &a2.expected_argv,
        &cwd_a2,
        "A2 handed to A",
    );

    let mut b2 = spawn_worker(
        base.path(),
        "b2",
        &identifier,
        Some(&id_b),
        &cwd_b2,
        &skip(&payload_b2),
        bus,
    );
    run_secondary(&mut b2, deadline);
    wait_for_receipt(
        &b.receipt,
        deadline,
        &b2.expected_argv,
        &cwd_b2,
        "B2 handed to B",
    );

    assert!(
        !has_exact_argv(&read_receipts(&a.receipt), &payload_b2),
        "B2 must never reach A"
    );
    assert!(
        !has_exact_argv(&read_receipts(&b.receipt), &payload_a2),
        "A2 must never reach B"
    );

    let mut c = spawn_worker(
        base.path(),
        "c",
        &control_identifier,
        None,
        &cwd_a2,
        &[],
        bus,
    );
    await_primary_ready(&mut c, deadline);

    // Installed control: without an override the original same-identifier
    // handoff must still happen.
    let mut c2 = spawn_worker(
        base.path(),
        "c2",
        &control_identifier,
        None,
        &cwd_c2,
        &skip(&payload_c2),
        bus,
    );
    run_secondary(&mut c2, deadline);
    wait_for_receipt(
        &c.receipt,
        deadline,
        &c2.expected_argv,
        &cwd_c2,
        "default same-identifier handoff",
    );

    #[cfg(target_os = "macos")]
    assert!(
        endpoint(&c.identity).exists(),
        "C owns the default socket before A stops"
    );
    stop_primary(&mut a, deadline);
    #[cfg(target_os = "macos")]
    wait_until(
        deadline,
        || !endpoint(&a.identity).exists(),
        "A plugin destroy removed its owned socket node",
    );
    #[cfg(target_os = "macos")]
    assert!(
        endpoint(&c.identity).exists(),
        "A destroy must preserve C default endpoint"
    );
    assert!(
        b.guard
            .try_wait()
            .expect("inspect B after A destroy")
            .is_none(),
        "stopping A must not affect B"
    );
    let mut b3 = spawn_worker(
        base.path(),
        "b3",
        &identifier,
        Some(&id_b),
        &cwd_b3,
        &skip(&payload_b3),
        bus,
    );
    run_secondary(&mut b3, deadline);
    wait_for_receipt(
        &b.receipt,
        deadline,
        &b3.expected_argv,
        &cwd_b3,
        "B3 handed to B after A stopped",
    );
    let mut c2_again = spawn_worker(
        base.path(),
        "c2-again",
        &control_identifier,
        None,
        &cwd_c2,
        &skip(&format!("C2-again payload {token}")),
        bus,
    );
    run_secondary(&mut c2_again, deadline);
    wait_for_receipt(
        &c.receipt,
        deadline,
        &c2_again.expected_argv,
        &cwd_c2,
        "C2 handoff after A stopped",
    );
    stop_primary(&mut c, deadline);
    stop_primary(&mut b, deadline);
    #[cfg(target_os = "macos")]
    wait_until(
        deadline,
        || !endpoint(&b.identity).exists() && !endpoint(&c.identity).exists(),
        "plugin destroy removed every owned endpoint",
    );
}

#[test]
fn native_instance_worker() {
    // A normal test run must return immediately without the exact owned worker
    // environment.
    if env::var(WORKER_MARKER).ok().as_deref() != Some("1")
        || env::var_os(WORKER_IDENTIFIER).is_none()
        || env::var_os(WORKER_RECEIPT).is_none()
        || env::var_os(WORKER_STOP).is_none()
        || env::var_os(WORKER_READY).is_none()
    {
        return;
    }
    run_instance_worker();
}

fn run_instance_worker() {
    let identifier = env::var(WORKER_IDENTIFIER).expect("worker identifier");
    let override_id = env::var(WORKER_OVERRIDE).ok();
    let receipt = PathBuf::from(env::var(WORKER_RECEIPT).expect("worker receipt path"));
    let stop = PathBuf::from(env::var(WORKER_STOP).expect("worker stop path"));
    let ready = PathBuf::from(env::var(WORKER_READY).expect("worker ready path"));

    let plugin = build_plugin(override_id.as_deref(), receipt);
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = identifier;

    if let Some(path) = env::var_os(WORKER_ENTERING) {
        fs::write(path, b"entering plugin setup").expect("worker entering marker");
    }
    let app = mock_builder()
        .plugin(plugin)
        .build(context)
        .expect("worker builds the mock app around the real single-instance plugin");

    // The plugin returned success. A normal secondary handoff exits inside the
    // plugin setup before the build returns.
    if let Some(path) = env::var_os(WORKER_CONFIG) {
        crate::storage::config_file::write_config(
            Path::new(&path),
            &crate::storage::config_file::ConfigWriteRequest {
                yaml: format!("instance_worker: {}\n", std::process::id()),
                expected_revision: None,
                require_missing: true,
            },
        )
        .expect("owned worker writes through the actual configuration API");
    }
    fs::write(&ready, b"ready").expect("worker publishes its readiness marker");

    let deadline = Instant::now() + WORKER_LIFETIME;
    while !stop.exists() && Instant::now() < deadline {
        #[cfg(windows)]
        pump_native_messages();
        thread::sleep(POLL);
    }

    // Stop primaries through the plugin's own teardown path so endpoint cleanup
    // is real rather than process death.
    single_instance::destroy(&app);
}

fn build_plugin(override_id: Option<&str>, receipt: PathBuf) -> TauriPlugin<MockRuntime> {
    let callback = move |_app: &tauri::AppHandle<MockRuntime>, argv: Vec<String>, cwd: String| {
        let record = serde_json::json!({ "argv": argv, "cwd": cwd });
        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&receipt)
        {
            Ok(mut file) => {
                if let Err(error) = writeln!(file, "{record}") {
                    eprintln!("instance worker could not write a receipt: {error}");
                }
            }
            Err(error) => eprintln!("instance worker could not record a receipt: {error}"),
        }
    };

    let mut builder = single_instance::Builder::<MockRuntime>::new().callback(callback);
    if let Some(id) = override_id {
        builder = builder.instance_id(id);
    }
    builder.build()
}

#[cfg(windows)]
fn pump_native_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn spawn_worker(
    base: &Path,
    label: &str,
    identifier: &str,
    override_id: Option<&str>,
    cwd: &str,
    extra_args: &[String],
    bus: Option<&str>,
) -> Worker {
    let ready = base.join(format!("{label}.ready"));
    let receipt = base.join(format!("{label}.receipt.jsonl"));
    let stop = base.join(format!("{label}.stop"));
    for path in [&ready, &receipt, &stop] {
        let _ = fs::remove_file(path);
    }

    let mut command = Command::new(env::current_exe().expect("current test executable"));
    command
        .arg("--exact")
        .arg(WORKER_TEST)
        .arg("--nocapture")
        .arg("--test-threads=1");
    command.args(extra_args);
    command
        .current_dir(cwd)
        .env(WORKER_MARKER, "1")
        .env(WORKER_IDENTIFIER, identifier)
        .env(WORKER_RECEIPT, &receipt)
        .env(WORKER_STOP, &stop)
        .env(WORKER_READY, &ready)
        .env_remove(WORKER_ENTERING)
        .env_remove(WORKER_CONFIG)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    match override_id {
        Some(id) => {
            command.env(WORKER_OVERRIDE, id);
        }
        None => {
            command.env_remove(WORKER_OVERRIDE);
        }
    }
    if let Some(address) = bus {
        command.env("DBUS_SESSION_BUS_ADDRESS", address);
    }

    let expected_argv = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| {
            argument
                .to_str()
                .expect("worker arguments are valid Unicode")
                .to_owned()
        })
        .collect();

    let guard = ChildGuard::spawn(&mut command, label)
        .unwrap_or_else(|error| panic!("spawn instance worker {label}: {error}"));
    Worker {
        label: label.to_owned(),
        identity: override_id.unwrap_or(identifier).to_owned(),
        ready,
        receipt,
        stop,
        expected_argv,
        guard,
    }
}

fn await_primary_ready(worker: &mut Worker, deadline: Instant) {
    let ready_deadline = deadline.min(Instant::now() + PRIMARY_READY_TIMEOUT);
    loop {
        if worker.ready.exists() {
            break;
        }
        if let Some(status) = worker.guard.try_wait().expect("inspect primary readiness") {
            panic!("{} exited before readiness: {status}", worker.label);
        }
        assert!(
            Instant::now() < ready_deadline,
            "timed out waiting for {} readiness",
            worker.label
        );
        thread::sleep(POLL);
    }
    #[cfg(target_os = "macos")]
    loop {
        if endpoint(&worker.identity).exists() {
            break;
        }
        if let Some(status) = worker
            .guard
            .try_wait()
            .expect("inspect primary socket readiness")
        {
            panic!("{} exited before owning its socket: {status}", worker.label);
        }
        assert!(
            Instant::now() < ready_deadline,
            "timed out waiting for {} owned socket node",
            worker.label
        );
        thread::sleep(POLL);
    }
}

fn run_secondary(worker: &mut Worker, deadline: Instant) {
    let status = worker
        .guard
        .wait_bounded(deadline, &format!("{} handoff exit", worker.label));
    assert_eq!(
        status.code(),
        Some(0),
        "{} must exit through the plugin's real process exit: {status}",
        worker.label
    );
}

fn stop_primary(worker: &mut Worker, deadline: Instant) {
    fs::write(&worker.stop, b"stop")
        .unwrap_or_else(|error| panic!("stop {}: {error}", worker.label));
    let status = worker
        .guard
        .wait_bounded(deadline, &format!("{} plugin destroy", worker.label));
    assert_eq!(
        status.code(),
        Some(0),
        "{} must exit cleanly after plugin destroy: {status}",
        worker.label
    );
}

fn skip(payload: &str) -> Vec<String> {
    vec!["--skip".to_owned(), payload.to_owned(), String::new()]
}

fn unique_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("t{}_{}", std::process::id(), nanos)
}

fn derive_instance_id(root: &Path, identifier: &str) -> String {
    let id = crate::identity::portable_instance_id(identifier, Some(root))
        .expect("portable instance identity resolves")
        .expect("a portable root yields an instance identity");
    assert!(!id.is_empty(), "the instance identity must not be empty");
    id
}

fn owned_root(base: &Path, name: &str) -> PathBuf {
    let root = base.join(name);
    fs::create_dir_all(root.join("data")).expect("portable data directory");
    root
}

fn owned_cwd(base: &Path, name: &str) -> String {
    let dir = base.join("cwd").join(name);
    fs::create_dir_all(&dir).expect("owned working directory");
    fs::canonicalize(&dir)
        .expect("canonical working directory")
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Debug, serde::Deserialize)]
struct Receipt {
    argv: Vec<String>,
    cwd: String,
}

fn read_receipts(path: &Path) -> Vec<Receipt> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn has_exact_argv(receipts: &[Receipt], payload: &str) -> bool {
    receipts
        .iter()
        .any(|receipt| receipt.argv.iter().any(|arg| arg == payload))
}

fn has_record(receipts: &[Receipt], argv: &[String], cwd: &str) -> bool {
    receipts
        .iter()
        .any(|receipt| receipt.argv == argv && working_directory_matches(&receipt.cwd, cwd))
}

fn working_directory_matches(actual: &str, expected: &str) -> bool {
    #[cfg(windows)]
    {
        if !Path::new(actual).is_absolute() || !Path::new(expected).is_absolute() {
            return false;
        }
        match (fs::canonicalize(actual), fs::canonicalize(expected)) {
            (Ok(actual), Ok(expected)) => actual == expected,
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        actual == expected
    }
}

#[cfg(test)]
mod cwd_matching_tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    fn receipt(argv: &[String], cwd: &str) -> Receipt {
        Receipt {
            argv: argv.to_vec(),
            cwd: cwd.to_owned(),
        }
    }

    #[test]
    fn full_argv_and_identical_cwd_match() {
        let base = tempfile::tempdir().expect("private directory");
        let cwd = owned_cwd(base.path(), "cwd");
        let argv = args(&["tabby-rs", "--exact", "native_instance_worker"]);
        let receipts = vec![receipt(&argv, &cwd)];

        assert!(
            has_record(&receipts, &argv, &cwd),
            "identical argv and working directory must match"
        );
        assert!(working_directory_matches(&cwd, &cwd));
    }

    #[test]
    fn changed_missing_added_or_reordered_arguments_do_not_match() {
        let base = tempfile::tempdir().expect("private directory");
        let cwd = owned_cwd(base.path(), "cwd");
        let argv = args(&["tabby-rs", "--exact", "native_instance_worker"]);
        let receipts = vec![receipt(&argv, &cwd)];

        let mut changed = argv.clone();
        changed[2] = "another_worker".to_owned();
        assert!(
            !has_record(&receipts, &changed, &cwd),
            "a changed argument must not match"
        );

        let missing = argv[..argv.len() - 1].to_vec();
        assert!(
            !has_record(&receipts, &missing, &cwd),
            "a missing argument must not match"
        );

        let mut added = argv.clone();
        added.push("--nocapture".to_owned());
        assert!(
            !has_record(&receipts, &added, &cwd),
            "an added argument must not match"
        );

        let reordered = vec![argv[0].clone(), argv[2].clone(), argv[1].clone()];
        assert!(
            !has_record(&receipts, &reordered, &cwd),
            "a reordered argument list must not match"
        );
    }

    #[test]
    fn same_argv_in_a_different_directory_does_not_match() {
        let base = tempfile::tempdir().expect("private directory");
        let cwd_a = owned_cwd(base.path(), "a");
        let cwd_b = owned_cwd(base.path(), "b");
        assert_ne!(cwd_a, cwd_b, "two owned working directories must differ");

        let argv = args(&["tabby-rs", "--exact", "native_instance_worker"]);
        let receipts = vec![receipt(&argv, &cwd_a)];

        assert!(has_record(&receipts, &argv, &cwd_a));
        assert!(
            !has_record(&receipts, &argv, &cwd_b),
            "the same argv in a different directory must not match"
        );
        assert!(!working_directory_matches(&cwd_a, &cwd_b));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_matcher_keeps_exact_string_semantics() {
        let base = tempfile::tempdir().expect("private directory");
        let cwd = owned_cwd(base.path(), "cwd");
        assert!(working_directory_matches(&cwd, &cwd));

        let alias = format!("{cwd}/.");
        assert_ne!(alias, cwd, "the dot alias is a different string");
        assert!(
            !working_directory_matches(&alias, &cwd),
            "the matcher must not accept a dot alias on Unix"
        );
        assert!(!working_directory_matches(&cwd, &alias));
    }

    #[cfg(windows)]
    fn normal_spelling(verbatim: &str) -> String {
        if let Some(rest) = verbatim.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = verbatim.strip_prefix(r"\\?\") {
            rest.to_owned()
        } else {
            verbatim.to_owned()
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_verbatim_and_normal_spellings_of_one_directory_match_both_ways() {
        let base = tempfile::tempdir().expect("private directory");
        let verbatim = fs::canonicalize(base.path())
            .expect("canonical temp directory")
            .to_string_lossy()
            .into_owned();
        let normal = normal_spelling(&verbatim);

        assert!(
            !normal.starts_with(r"\\?\"),
            "the normal spelling must not carry the verbatim prefix"
        );
        assert!(Path::new(&normal).is_absolute());
        assert!(
            working_directory_matches(&verbatim, &normal),
            "a verbatim actual path must match its normal expected spelling"
        );
        assert!(
            working_directory_matches(&normal, &verbatim),
            "a normal actual spelling must match its verbatim expected path"
        );

        let argv = args(&["tabby-rs", "--exact", "native_instance_worker"]);
        assert!(
            has_record(&[receipt(&argv, &verbatim)], &argv, &normal),
            "a verbatim receipt cwd must match a normal expected cwd"
        );
        assert!(
            has_record(&[receipt(&argv, &normal)], &argv, &verbatim),
            "a normal receipt cwd must match a verbatim expected cwd"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_missing_paths_never_match_even_when_identical() {
        let base = tempfile::tempdir().expect("private directory");
        let existing = owned_cwd(base.path(), "existing");
        let missing = base
            .path()
            .join("does-not-exist")
            .to_string_lossy()
            .into_owned();
        assert!(fs::canonicalize(&existing).is_ok());
        assert!(!Path::new(&missing).exists());
        assert!(Path::new(&missing).is_absolute());

        assert!(
            !working_directory_matches(&missing, &missing),
            "identical strings for a missing path must not be treated as equal"
        );
        assert!(
            !working_directory_matches(&existing, &missing),
            "an existing directory must not match a missing path"
        );
        assert!(
            !working_directory_matches(&missing, &existing),
            "a missing path must not match an existing directory"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_non_absolute_spellings_never_match_an_existing_directory() {
        let base = tempfile::tempdir().expect("private directory");
        let existing = owned_cwd(base.path(), "existing");
        assert!(fs::canonicalize(&existing).is_ok());

        for candidate in ["", "folder", r"C:folder", r"\folder"] {
            assert!(
                !Path::new(candidate).is_absolute(),
                "{candidate:?} must not be treated as absolute"
            );
            assert!(
                !working_directory_matches(candidate, &existing),
                "{candidate:?} as actual must not match an existing directory"
            );
            assert!(
                !working_directory_matches(&existing, candidate),
                "{candidate:?} as expected must not match an existing directory"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_relative_path_never_matches_even_when_it_resolves_to_the_same_directory() {
        let cwd = env::current_dir().expect("current directory");
        let base = tempfile::tempdir_in(&cwd).expect("temp directory inside the current directory");
        let relative = base
            .path()
            .file_name()
            .expect("temp directory name")
            .to_string_lossy()
            .into_owned();
        let canonical = fs::canonicalize(base.path()).expect("canonical temp directory");

        assert!(!Path::new(&relative).is_absolute());
        assert_eq!(
            fs::canonicalize(&relative).expect("the relative spelling resolves"),
            canonical,
            "the relative spelling resolves to the same real directory"
        );

        let canonical = canonical.to_string_lossy().into_owned();
        assert!(
            !working_directory_matches(&relative, &canonical),
            "a relative actual path must not match an absolute working directory"
        );
        assert!(
            !working_directory_matches(&canonical, &relative),
            "an absolute actual path must not match a relative expected path"
        );
    }
}

fn wait_for_receipt(path: &Path, deadline: Instant, argv: &[String], cwd: &str, what: &str) {
    loop {
        let receipts = read_receipts(path);
        if has_record(&receipts, argv, cwd) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; expected argv={argv:?}, cwd={cwd:?}; received={receipts:#?}; receipt={}; raw={:?}",
            path.display(),
            fs::read_to_string(path).map(|raw| raw.chars().take(8192).collect::<String>())
        );
        thread::sleep(POLL);
    }
}

fn wait_until(deadline: Instant, mut condition: impl FnMut() -> bool, what: &str) {
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(POLL);
    }
    assert!(condition(), "timed out waiting for {what}");
}

struct Worker {
    label: String,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    identity: String,
    ready: PathBuf,
    receipt: PathBuf,
    stop: PathBuf,
    expected_argv: Vec<String>,
    guard: ChildGuard,
}

struct ChildGuard {
    child: Option<Child>,
    label: String,
}

impl ChildGuard {
    fn spawn(command: &mut Command, label: &str) -> io::Result<Self> {
        Ok(Self::adopt(command.spawn()?, label))
    }

    fn adopt(child: Child, label: &str) -> Self {
        Self {
            child: Some(child),
            label: label.to_owned(),
        }
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self.child.as_mut() {
            Some(child) => child.try_wait(),
            None => Ok(None),
        }
    }

    fn wait_bounded(&mut self, deadline: Instant, what: &str) -> ExitStatus {
        loop {
            match self.try_wait() {
                Ok(Some(status)) => {
                    self.child = None;
                    return status;
                }
                Ok(None) => {}
                Err(error) => panic!("could not inspect {what}: {error}"),
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(POLL);
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Err(error) = child.kill() {
                eprintln!("failed to stop instance worker {}: {error}", self.label);
            }
            let _ = child.wait();
        }
    }
}

#[derive(Default)]
struct EndpointGuard {
    paths: Vec<PathBuf>,
}

impl EndpointGuard {
    fn track(&mut self, identity: &str) {
        assert_ne!(
            identity,
            crate::identity::APP_IDENTIFIER,
            "refusing to own the installed application endpoint"
        );
        #[cfg(target_os = "macos")]
        {
            let path = endpoint(identity);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => panic!("owned socket path already exists: {}", path.display()),
            }
            self.paths.push(path);
        }
    }
}

impl Drop for EndpointGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::fs::FileTypeExt;
                if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn endpoint(identity: &str) -> PathBuf {
    PathBuf::from(format!(
        "/tmp/{}_si.sock",
        identity.replace(['.', '-'].as_ref(), "_")
    ))
}

#[cfg(target_os = "linux")]
struct OwnedSessionBus {
    child: Option<ChildGuard>,
    address: Option<String>,
}

#[cfg(target_os = "linux")]
impl OwnedSessionBus {
    fn ensure() -> Self {
        if env::var_os("DBUS_SESSION_BUS_ADDRESS")
            .filter(|value| !value.is_empty())
            .is_some()
        {
            return Self {
                child: None,
                address: None,
            };
        }

        let mut command = Command::new("dbus-daemon");
        command
            .args(["--session", "--nofork", "--print-address=1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut guard = ChildGuard::adopt(
            command.spawn().expect("owned dbus-daemon starts"),
            "dbus-daemon",
        );
        let stdout = guard
            .child
            .as_mut()
            .expect("guard owns daemon")
            .stdout
            .take()
            .expect("dbus-daemon address pipe");
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut line = String::new();
            let result = std::io::BufRead::read_line(&mut io::BufReader::new(stdout), &mut line)
                .map(|_| line.trim().to_owned());
            let _ = tx.send(result);
        });
        let address = match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(address)) if !address.is_empty() => {
                reader.join().expect("address reader exits");
                address
            }
            Ok(Ok(_)) => {
                drop(guard);
                reader.join().expect("address reader exits");
                panic!("dbus-daemon reported an empty session address");
            }
            Ok(Err(error)) => {
                drop(guard);
                reader.join().expect("address reader exits");
                panic!("reading dbus-daemon address: {error}");
            }
            Err(error) => {
                drop(guard);
                reader.join().expect("address reader exits");
                panic!("timed out reading dbus-daemon address: {error}");
            }
        };
        Self {
            child: Some(guard),
            address: Some(address),
        }
    }

    fn address(&self) -> Option<&str> {
        self.address.as_deref()
    }
}

#[cfg(not(target_os = "linux"))]
struct OwnedSessionBus;

#[cfg(not(target_os = "linux"))]
impl OwnedSessionBus {
    fn ensure() -> Self {
        Self
    }

    fn address(&self) -> Option<&str> {
        None
    }
}
