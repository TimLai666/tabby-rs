// The framework boundary stays mocked; Win32 transport and child processes stay real.

#[path = "windows_instance_startup_tests.rs"]
mod startup_tests;

use super::*;
use crate::windows_instance_message::{
    JSON_DATA_ID, JSON_PROTOCOL_REPLY, LEGACY_DATA_ID, QUERY_MESSAGE,
};
use std::{
    ffi::{CStr, OsStr},
    os::windows::ffi::OsStrExt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::COPYDATASTRUCT,
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateMutexW, ReleaseMutex},
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetWindowTextW,
        PeekMessageW, PostMessageW, RegisterClassExW, SendMessageTimeoutW, TranslateMessage,
        UnregisterClassW, CREATESTRUCTW, GWLP_USERDATA, MSG, PM_REMOVE, SMTO_ABORTIFHUNG,
        WM_COPYDATA, WM_CREATE, WM_DESTROY, WM_NCCREATE, WNDCLASSEXW,
    },
};

const SERVER_STOP: u32 = 0x8000 + 0x5a1;
const MESSAGE_TIMEOUT_MS: u32 = 2_000;
const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn modern_owner_accepts_legacy_and_json_and_recovers_after_invalid_data() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private Windows instance namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);
    let cwd = owned_cwd(base.path(), "owner-cwd");
    let mut owner = spawn_worker(
        base.path(),
        "modern-owner",
        &identifier,
        None,
        &cwd,
        &[],
        None,
    );
    await_primary_ready(&mut owner, deadline);
    let hwnd = find_owner_window(&identifier);

    assert_eq!(query_format(hwnd), JSON_PROTOCOL_REPLY as usize);

    let legacy_cwd = owned_cwd(base.path(), "legacy-cwd");
    let legacy_args = vec![
        "program.exe".to_owned(),
        "--legacy".to_owned(),
        "plain text".to_owned(),
        "中文😀".to_owned(),
        String::new(),
    ];
    let legacy = legacy_bytes(&legacy_cwd, &legacy_args);
    assert_eq!(send_copydata(hwnd, LEGACY_DATA_ID, &legacy), Some(1));

    let json_cwd = owned_cwd(base.path(), "json-cwd-資料");
    let json_args = vec![
        "program.exe".to_owned(),
        "A|B".to_owned(),
        "two words \"quoted\" 中文".to_owned(),
        String::new(),
    ];
    let json = serde_json::to_vec(&(json_cwd.as_str(), &json_args)).expect("valid JSON payload");
    assert_eq!(send_copydata(hwnd, JSON_DATA_ID, &json), Some(1));

    let invalid = [
        (
            LEGACY_DATA_ID + 91,
            legacy_bytes(&cwd, &["wrong-id".to_owned()]),
        ),
        (LEGACY_DATA_ID, b"missing terminator".to_vec()),
        (JSON_DATA_ID, b"{".to_vec()),
        (JSON_DATA_ID, br#"["cwd",42]"#.to_vec()),
    ];
    for (data_id, bytes) in invalid {
        assert_eq!(send_copydata(hwnd, data_id, &bytes), Some(0));
    }
    assert!(
        matches!(send_empty_copydata(hwnd, JSON_DATA_ID), None | Some(0)),
        "an empty payload cannot be acknowledged as handled"
    );
    assert!(
        matches!(send_null_copydata_struct(hwnd), None | Some(0)),
        "a null WM_COPYDATA lParam cannot be acknowledged as handled"
    );
    assert!(
        matches!(send_null_copydata(hwnd, JSON_DATA_ID), None | Some(0)),
        "a null native payload pointer cannot be acknowledged as handled"
    );

    let final_cwd = owned_cwd(base.path(), "after-invalid");
    let final_args = vec!["still-alive".to_owned(), "tail".to_owned()];
    let final_json = serde_json::to_vec(&(final_cwd.as_str(), &final_args)).unwrap();
    assert_eq!(send_copydata(hwnd, JSON_DATA_ID, &final_json), Some(1));

    wait_for_native_records(
        &owner.receipt,
        deadline,
        &[
            (legacy_args.clone(), legacy_cwd.clone()),
            (json_args.clone(), json_cwd.clone()),
            (final_args.clone(), final_cwd.clone()),
        ],
    );
    let records = read_receipts(&owner.receipt);
    assert_eq!(
        records.len(),
        3,
        "invalid messages must not invoke the callback"
    );
    assert!(has_record(&records, &legacy_args, &legacy_cwd));
    assert!(has_record(&records, &json_args, &json_cwd));
    assert!(has_record(&records, &final_args, &final_cwd));
    stop_primary(&mut owner, deadline);
}

#[test]
fn modern_sender_preserves_representable_requests_for_a_legacy_owner() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private legacy fixture namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);
    let legacy = LegacyServer::start(&identifier, false);
    let unknown = b"old receiver ignores payload\0";
    assert_eq!(
        send_copydata(legacy.hwnd, LEGACY_DATA_ID + 77, unknown),
        Some(1)
    );
    assert!(
        legacy.try_receive().is_none(),
        "unknown data id returns 1 without callback"
    );
    let sender_cwd = owned_cwd(base.path(), "sender-cwd");
    let args = skip("Unicode 資料; two words and \"quotes\"");
    let mut sender = spawn_worker(
        base.path(),
        "legacy-target-sender",
        &identifier,
        None,
        &sender_cwd,
        &args,
        None,
    );
    let record = legacy
        .receive(deadline)
        .expect("legacy window receives one real WM_COPYDATA request");
    assert_eq!(
        record.argv, sender.expected_argv,
        "legacy callback receives full argv once"
    );
    assert!(
        working_directory_matches(&record.cwd, &sender_cwd),
        "legacy callback receives the complete cwd"
    );
    let status = sender
        .guard
        .wait_bounded(deadline, "legacy-compatible sender exit");
    assert_eq!(
        status.code(),
        Some(0),
        "representable legacy handoff exits successfully"
    );
    assert!(
        legacy.try_receive().is_none(),
        "one exited sender caused exactly one callback"
    );
    drop(legacy);
}

#[test]
fn modern_sender_rejects_unrepresentable_legacy_requests_without_callback() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private legacy rejection namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);
    let legacy = LegacyServer::start(&identifier, false);
    let sender_cwd = owned_cwd(base.path(), "reject-sender-cwd");
    let mut command = worker_command(
        base.path(),
        "legacy-reject-sender",
        &identifier,
        &sender_cwd,
        &skip("contains|pipe"),
    );
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut sender = ChildGuard::spawn(&mut command, "legacy-reject-sender")
        .expect("start sender child with captured diagnostic");
    let output = wait_for_child_output(&mut sender, deadline, "unrepresentable request rejection");
    assert!(
        !output.status.success(),
        "unsupported legacy payload must exit nonzero"
    );
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("restart") && diagnostic.contains("'|'"),
        "rejection must explain the unsupported separator and restart path; stderr={diagnostic:?}"
    );
    assert!(
        legacy.try_receive().is_none(),
        "unrepresentable request is never delivered"
    );

    let safe_args = skip("safe-after-rejection");
    let safe_cwd = owned_cwd(base.path(), "safe-after-rejection");
    let mut followup = spawn_worker(
        base.path(),
        "safe-followup-sender",
        &identifier,
        None,
        &safe_cwd,
        &safe_args,
        None,
    );
    let record = legacy
        .receive(deadline)
        .expect("legacy owner remains usable after rejection");
    assert_eq!(record.argv, followup.expected_argv);
    assert!(working_directory_matches(&record.cwd, &safe_cwd));
    let status = followup
        .guard
        .wait_bounded(deadline, "safe follow-up sender exit");
    assert_eq!(status.code(), Some(0));
    assert!(
        legacy.try_receive().is_none(),
        "safe follow-up is delivered only once"
    );
    drop(legacy);
}

#[test]
fn modern_sender_does_not_report_success_when_the_owner_rejects_the_message() {
    let deadline = Instant::now() + OVERALL_DEADLINE;
    let base = tempfile::tempdir().expect("private negative acknowledgement namespace");
    let token = unique_token();
    let identifier = format!("io.tabbyrs.test.{token}");
    assert_ne!(identifier, crate::identity::APP_IDENTIFIER);
    let rejecting = LegacyServer::start(&identifier, true);
    let sender_cwd = owned_cwd(base.path(), "negative-ack-sender-cwd");
    let mut command = worker_command(
        base.path(),
        "negative-ack-sender",
        &identifier,
        &sender_cwd,
        &skip("valid request"),
    );
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut sender = ChildGuard::spawn(&mut command, "negative-ack-sender")
        .expect("start sender child with captured diagnostic");
    let output = wait_for_child_output(&mut sender, deadline, "negative acknowledgement failure");
    assert!(
        !output.status.success(),
        "a zero WM_COPYDATA acknowledgement must not exit 0"
    );
    assert_eq!(
        rejecting.receive_observation(deadline),
        Some(JSON_DATA_ID),
        "the rejecting owner observed the actual JSON native payload"
    );
    assert!(
        rejecting.try_receive().is_none(),
        "the fixture rejects without callback"
    );
    drop(rejecting);
}

fn worker_command(
    base: &Path,
    label: &str,
    identifier: &str,
    cwd: &str,
    args: &[String],
) -> Command {
    let ready = base.join(format!("{label}.ready"));
    let receipt = base.join(format!("{label}.receipt.jsonl"));
    let stop = base.join(format!("{label}.stop"));
    let mut command = Command::new(env::current_exe().expect("current test executable"));
    command
        .arg("--exact")
        .arg(WORKER_TEST)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .args(args)
        .current_dir(cwd)
        .env(WORKER_MARKER, "1")
        .env(WORKER_IDENTIFIER, identifier)
        .env(WORKER_RECEIPT, receipt)
        .env(WORKER_STOP, stop)
        .env(WORKER_READY, ready)
        .env_remove(WORKER_ENTERING)
        .env_remove(WORKER_CONFIG)
        .env_remove(WORKER_OVERRIDE)
        .stdin(Stdio::null());
    command
}

fn wait_for_child_output(
    guard: &mut ChildGuard,
    deadline: Instant,
    what: &str,
) -> std::process::Output {
    loop {
        match guard.try_wait() {
            Ok(Some(_)) => {
                let child = guard.child.take().expect("completed child remains owned");
                return child.wait_with_output().expect("collect child output");
            }
            Ok(None) => {}
            Err(error) => panic!("could not inspect {what}: {error}"),
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

fn find_owner_window(identifier: &str) -> HWND {
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let class = wide(&format!("{identifier}-sic"));
    let title = wide(&format!("{identifier}-siw"));
    let hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    assert!(!hwnd.is_null(), "modern owner hidden window must exist");
    hwnd
}

fn query_format(hwnd: HWND) -> usize {
    send_message(hwnd, QUERY_MESSAGE, 0)
}

fn legacy_bytes(cwd: &str, args: &[String]) -> Vec<u8> {
    let mut fields = Vec::with_capacity(args.len() + 1);
    fields.push(cwd);
    fields.extend(args.iter().map(String::as_str));
    let mut bytes = fields.join("|").into_bytes();
    bytes.push(0);
    bytes
}

fn send_copydata(hwnd: HWND, data_id: usize, bytes: &[u8]) -> Option<usize> {
    let data = COPYDATASTRUCT {
        dwData: data_id,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr() as *mut _,
    };
    Some(send_message(hwnd, WM_COPYDATA, &data as *const _ as LPARAM))
}

fn send_empty_copydata(hwnd: HWND, data_id: usize) -> Option<usize> {
    let data = COPYDATASTRUCT {
        dwData: data_id,
        cbData: 0,
        lpData: std::ptr::null_mut(),
    };
    let mut result = 0usize;
    let delivered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            0,
            &data as *const _ as LPARAM,
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            &mut result,
        )
    };
    (delivered != 0).then_some(result)
}

fn send_null_copydata(hwnd: HWND, data_id: usize) -> Option<usize> {
    let data = COPYDATASTRUCT {
        dwData: data_id,
        cbData: 8,
        lpData: std::ptr::null_mut(),
    };
    let mut result = 0usize;
    let delivered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            0,
            &data as *const _ as LPARAM,
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            &mut result,
        )
    };
    (delivered != 0).then_some(result)
}

fn send_null_copydata_struct(hwnd: HWND) -> Option<usize> {
    let mut result = 0usize;
    let delivered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            0,
            0,
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            &mut result,
        )
    };
    (delivered != 0).then_some(result)
}

fn send_message(hwnd: HWND, message: u32, lparam: LPARAM) -> usize {
    let mut result = 0usize;
    let delivered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            message,
            0,
            lparam,
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            &mut result,
        )
    };
    assert_ne!(
        delivered, 0,
        "bounded native message delivery must complete"
    );
    result
}

fn wait_for_native_records(path: &Path, deadline: Instant, expected: &[(Vec<String>, String)]) {
    loop {
        let receipts = read_receipts(path);
        if expected
            .iter()
            .all(|(argv, cwd)| has_record(&receipts, argv, cwd))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for expected native callback receipts"
        );
        thread::sleep(POLL);
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[derive(Debug)]
struct NativeRecord {
    argv: Vec<String>,
    cwd: String,
}

struct WindowContext {
    records: SyncSender<NativeRecord>,
    observations: SyncSender<usize>,
    reject_payload: bool,
}

struct LegacyServer {
    hwnd: HWND,
    stop: Arc<AtomicBool>,
    records: Receiver<NativeRecord>,
    observations: Receiver<usize>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LegacyServer {
    fn start(identifier: &str, reject_payload: bool) -> Self {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (records_tx, records_rx) = mpsc::sync_channel(8);
        let (observations_tx, observations_rx) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let identifier = identifier.to_owned();
        let thread = thread::spawn(move || unsafe {
            let mutex_name = wide(&format!("{identifier}-sim"));
            let mutex = OwnedMutex(CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()));
            assert!(
                !mutex.0.is_null(),
                "legacy fixture owns its isolated named mutex"
            );
            let class_name = wide(&format!("{identifier}-sic"));
            let window_name = wide(&format!("{identifier}-siw"));
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(legacy_window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: GetModuleHandleW(std::ptr::null()),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            assert_ne!(
                RegisterClassExW(&class),
                0,
                "register private legacy window class"
            );
            let _class = OwnedClass {
                name: class_name.clone(),
                instance: GetModuleHandleW(std::ptr::null()),
            };
            let mut context = Box::new(WindowContext {
                records: records_tx,
                observations: observations_tx,
                reject_payload,
            });
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                (&mut *context as *mut WindowContext).cast(),
            );
            assert!(!hwnd.is_null(), "create owned legacy hidden window");
            std::mem::forget(context);
            let _window = OwnedWindow(hwnd);
            let mut stored_title = vec![0u16; window_name.len()];
            let title_len =
                GetWindowTextW(hwnd, stored_title.as_mut_ptr(), stored_title.len() as i32);
            assert_eq!(title_len as usize, window_name.len() - 1);
            assert_eq!(stored_title, window_name, "legacy window retains its title");
            assert_eq!(
                windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
                    class_name.as_ptr(),
                    window_name.as_ptr(),
                ),
                hwnd,
                "legacy owner is discoverable by the production class/title lookup"
            );
            ready_tx.send(hwnd as isize).expect("publish legacy HWND");
            let mut message: MSG = std::mem::zeroed();
            while !thread_stop.load(Ordering::Acquire) {
                while PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                thread::sleep(POLL);
            }
            drop(_window);
            drop(_class);
            drop(mutex);
        });
        let hwnd = match ready_rx.recv_timeout(SERVER_READY_TIMEOUT) {
            Ok(hwnd) => hwnd as HWND,
            Err(error) => {
                stop.store(true, Ordering::Release);
                thread
                    .join()
                    .expect("failed legacy fixture startup cleans up its resources");
                panic!("legacy hidden window and mutex did not become ready: {error}");
            }
        };
        Self {
            hwnd,
            stop,
            records: records_rx,
            observations: observations_rx,
            thread: Some(thread),
        }
    }

    fn receive(&self, deadline: Instant) -> Option<NativeRecord> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        self.records.recv_timeout(remaining).ok()
    }

    fn try_receive(&self) -> Option<NativeRecord> {
        self.records.try_recv().ok()
    }

    fn receive_observation(&self, deadline: Instant) -> Option<usize> {
        self.observations
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()
    }
}

impl Drop for LegacyServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = unsafe { PostMessageW(self.hwnd, SERVER_STOP, 0, 0) };
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .expect("legacy fixture destroys its HWND and releases mutex");
        }
    }
}

unsafe extern "system" fn legacy_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        QUERY_MESSAGE => {
            let context = &*window_context(hwnd);
            if context.reject_payload {
                JSON_PROTOCOL_REPLY as LRESULT
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_NCCREATE => {
            let create = &*(lparam as *const CREATESTRUCTW);
            set_window_context(hwnd, create.lpCreateParams.cast());
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_CREATE => 0,
        WM_COPYDATA => {
            let data = &*(lparam as *const COPYDATASTRUCT);
            let context = &*window_context(hwnd);
            let _ = context.observations.try_send(data.dwData);
            if data.dwData == LEGACY_DATA_ID {
                let text = CStr::from_ptr(data.lpData as *const i8).to_string_lossy();
                let mut fields = text.split('|');
                let cwd = fields.next().unwrap_or_default().to_owned();
                let argv = fields.map(str::to_owned).collect();
                let context = &*window_context(hwnd);
                let _ = context.records.try_send(NativeRecord { argv, cwd });
            }
            if context.reject_payload {
                0
            } else {
                1
            }
        }
        WM_DESTROY => {
            let context = window_context(hwnd);
            if !context.is_null() {
                drop(Box::from_raw(context));
                set_window_context(hwnd, std::ptr::null_mut());
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn set_window_context(hwnd: HWND, context: *mut WindowContext) {
    #[cfg(target_pointer_width = "32")]
    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongW(
        hwnd,
        GWLP_USERDATA,
        context as isize as i32,
    );
    #[cfg(target_pointer_width = "64")]
    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
        hwnd,
        GWLP_USERDATA,
        context as isize,
    );
}

unsafe fn window_context(hwnd: HWND) -> *mut WindowContext {
    #[cfg(target_pointer_width = "32")]
    {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongW(hwnd, GWLP_USERDATA) as isize
            as *mut WindowContext
    }
    #[cfg(target_pointer_width = "64")]
    {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
            as *mut WindowContext
    }
}

struct OwnedMutex(windows_sys::Win32::Foundation::HANDLE);

impl Drop for OwnedMutex {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
}

struct OwnedWindow(HWND);

impl Drop for OwnedWindow {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DestroyWindow(self.0) };
        }
    }
}

struct OwnedClass {
    name: Vec<u16>,
    instance: windows_sys::Win32::Foundation::HINSTANCE,
}

impl Drop for OwnedClass {
    fn drop(&mut self) {
        unsafe { UnregisterClassW(self.name.as_ptr(), self.instance) };
    }
}
