//! Native regression for the macOS application-quit adapter.
//!
//! This example loads the real production module (`src/platform/macos_quit.rs`)
//! and drives a real `NSApplication`. Only the Tauri `AppHandle` boundary is
//! replaced (a cloneable recorder that logs `exit(i32)`); every Objective-C
//! call is genuine Cocoa. Each scenario runs in its own helper process so the
//! global `NSApp`/class registration cannot leak between cases.

#![allow(dead_code)]

#[cfg(target_os = "macos")]
mod tauri_stub {
    use std::sync::Mutex;

    static EXIT_CODES: Mutex<Vec<i32>> = Mutex::new(Vec::new());

    #[derive(Clone)]
    pub struct AppHandle;

    impl AppHandle {
        pub fn new() -> Self {
            Self
        }

        pub fn exit(&self, code: i32) {
            EXIT_CODES.lock().expect("exit recorder").push(code);
        }
    }

    pub fn recorded_exits() -> Vec<i32> {
        EXIT_CODES.lock().expect("exit recorder").clone()
    }
}

#[cfg(target_os = "macos")]
mod error {
    #[derive(Debug)]
    pub enum AppError {
        InvalidArgument(String),
        InvalidData(String),
        Conflict(String),
        NotFound(String),
        PermissionDenied(String),
        Io(String),
        Unsupported(String),
    }

    impl std::fmt::Display for AppError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidArgument(message)
                | Self::InvalidData(message)
                | Self::Conflict(message)
                | Self::NotFound(message)
                | Self::PermissionDenied(message)
                | Self::Io(message)
                | Self::Unsupported(message) => f.write_str(message),
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod subject {
    use crate::tauri_stub as tauri;

    include!("../src/platform/macos_quit.rs");
}

#[cfg(target_os = "macos")]
mod probes {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply};
    use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

    #[derive(Default)]
    pub struct ProbeState {
        will_terminate: AtomicBool,
        reopen_count: AtomicUsize,
        sentinel: AtomicU64,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "TabbyRSQuitProbeOriginal"]
        #[thread_kind = MainThreadOnly]
        #[ivars = ProbeState]
        pub struct Original;

        unsafe impl NSObjectProtocol for Original {}

        unsafe impl NSApplicationDelegate for Original {
            #[unsafe(method(applicationWillTerminate:))]
            fn will_terminate(&self, _notification: &NSNotification) {
                self.ivars().will_terminate.store(true, Ordering::SeqCst);
            }

            #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
            fn should_handle_reopen(
                &self,
                _sender: &NSApplication,
                _has_visible_windows: bool,
            ) -> bool {
                self.ivars().reopen_count.fetch_add(1, Ordering::SeqCst);
                true
            }
        }

        impl Original {
            #[unsafe(method(tabbyRSProbeSentinel))]
            fn probe_sentinel(&self) -> u64 {
                self.ivars().sentinel.load(Ordering::SeqCst)
            }

            #[unsafe(method(tabbyRSProbeReopenCount))]
            fn probe_reopen_count(&self) -> usize {
                self.ivars().reopen_count.load(Ordering::SeqCst)
            }

            #[unsafe(method(tabbyRSProbeWillTerminate))]
            fn probe_will_terminate(&self) -> bool {
                self.ivars().will_terminate.load(Ordering::SeqCst)
            }
        }
    );

    impl Original {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let allocated = Self::alloc(mtm).set_ivars(ProbeState::default());
            // SAFETY: NSObject's initializer runs on our allocated subclass.
            unsafe { msg_send![super(allocated), init] }
        }

        pub fn new_with_sentinel(mtm: MainThreadMarker, sentinel: u64) -> Retained<Self> {
            let state = ProbeState {
                sentinel: AtomicU64::new(sentinel),
                ..ProbeState::default()
            };
            let allocated = Self::alloc(mtm).set_ivars(state);
            // SAFETY: NSObject's initializer runs on our allocated subclass.
            unsafe { msg_send![super(allocated), init] }
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "TabbyRSQuitProbeConflict"]
        #[thread_kind = MainThreadOnly]
        #[ivars = ProbeState]
        pub struct Conflict;

        unsafe impl NSObjectProtocol for Conflict {}

        unsafe impl NSApplicationDelegate for Conflict {
            #[unsafe(method(applicationShouldTerminate:))]
            fn should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
                NSApplicationTerminateReply::TerminateCancel
            }
        }
    );

    impl Conflict {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let allocated = Self::alloc(mtm).set_ivars(ProbeState::default());
            // SAFETY: NSObject's initializer runs on our allocated subclass.
            unsafe { msg_send![super(allocated), init] }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macos_quit_native: Cocoa terminate checks require macOS; no native checks ran");
}

#[cfg(target_os = "macos")]
fn main() {
    let case = std::env::args().nth(1).unwrap_or_default();
    match run_case(&case) {
        Ok(evidence) => println!("TABBY_QUIT_NATIVE_EVIDENCE {evidence}"),
        Err(error) => {
            eprintln!("macos_quit_native: {case}: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(target_os = "macos")]
fn run_case(case: &str) -> Result<serde_json::Value, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "the helper must start on the main thread".to_string())?;
    let app = NSApplication::sharedApplication(mtm);

    match case {
        "terminate_cancels_direct_exit" => case_terminate_cancels(&app, mtm),
        "repeated_terminate_stays_alive" => case_repeated_terminate(&app, mtm),
        "repeat_install_preserves_delegate" => case_repeat_install(&app, mtm),
        "preserves_original_callbacks_and_ivars" => case_preserve_callbacks(&app, mtm),
        "changed_delegate_install_errors" => case_changed_delegate(&app, mtm),
        "non_main_thread_install_errors" => case_non_main_thread(&app, mtm),
        "no_delegate_install_errors" => case_no_delegate(&app),
        "conflicting_delegate_install_errors" => case_conflict(&app, mtm),
        other => Err(format!("unknown case: {other}")),
    }
}

#[cfg(target_os = "macos")]
fn install_with(handle: &crate::tauri_stub::AppHandle) -> Result<(), String> {
    crate::subject::install(handle).map_err(|error| format!("install failed: {error:?}"))
}

#[cfg(target_os = "macos")]
fn case_terminate_cancels(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use objc2::msg_send;
    use objc2::runtime::ProtocolObject;
    use serde_json::json;

    let original = crate::probes::Original::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*original)));
    let handle = crate::tauri_stub::AppHandle::new();
    install_with(&handle)?;

    app.terminate(None);

    let will_terminate: bool = unsafe { msg_send![&*original, tabbyRSProbeWillTerminate] };
    let exit_codes = crate::tauri_stub::recorded_exits();
    let exited_zero = exit_codes == [0];
    Ok(json!({
        "case": "terminate_cancels_direct_exit",
        "passed": !will_terminate && exited_zero,
        "exitCodes": exit_codes,
        "willTerminate": will_terminate,
        "aliveAfterTerminate": true,
    }))
}

#[cfg(target_os = "macos")]
fn case_repeated_terminate(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use objc2::msg_send;
    use objc2::runtime::ProtocolObject;
    use serde_json::json;

    let original = crate::probes::Original::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*original)));
    let handle = crate::tauri_stub::AppHandle::new();
    install_with(&handle)?;

    for _ in 0..3 {
        app.terminate(None);
    }

    let will_terminate: bool = unsafe { msg_send![&*original, tabbyRSProbeWillTerminate] };
    let exit_codes = crate::tauri_stub::recorded_exits();
    let exited_zero = exit_codes == [0, 0, 0];
    Ok(json!({
        "case": "repeated_terminate_stays_alive",
        "passed": !will_terminate && exited_zero,
        "exitCodes": exit_codes,
        "terminateCalls": 3,
        "willTerminate": will_terminate,
        "aliveAfterTerminate": true,
    }))
}

#[cfg(target_os = "macos")]
fn case_repeat_install(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use objc2::runtime::ProtocolObject;
    use serde_json::json;

    let original = crate::probes::Original::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*original)));
    let handle = crate::tauri_stub::AppHandle::new();

    let delegate_before = delegate_ptr(app)?;
    let first_ok = crate::subject::install(&handle).is_ok();
    let delegate_after_first = delegate_ptr(app)?;
    let class_after_first = delegate_class_ptr(app)?;

    let second_ok = crate::subject::install(&handle).is_ok();
    let delegate_after_second = delegate_ptr(app)?;
    let class_after_second = delegate_class_ptr(app)?;

    let delegate_preserved =
        delegate_before == delegate_after_first && delegate_after_first == delegate_after_second;
    let class_preserved = class_after_first == class_after_second;
    app.terminate(None);
    let requested_exit = crate::tauri_stub::recorded_exits() == [0];
    Ok(json!({
        "case": "repeat_install_preserves_delegate",
        "passed": first_ok && second_ok && delegate_preserved && class_preserved && requested_exit,
        "firstInstallOk": first_ok,
        "secondInstallOk": second_ok,
        "delegatePreserved": delegate_preserved,
        "classPreserved": class_preserved,
    }))
}

#[cfg(target_os = "macos")]
fn case_preserve_callbacks(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use objc2::msg_send;
    use objc2::runtime::ProtocolObject;
    use serde_json::json;

    let original = crate::probes::Original::new_with_sentinel(mtm, 0x00AB_CDEF);
    app.setDelegate(Some(ProtocolObject::from_ref(&*original)));
    let handle = crate::tauri_stub::AppHandle::new();
    install_with(&handle)?;

    let current = app.delegate().ok_or("delegate missing after install")?;
    let delegate_preserved =
        delegate_ptr(app)? == objc2::rc::Retained::as_ptr(&original).cast::<()>() as usize;
    let _: bool = unsafe {
        msg_send![&*current, applicationShouldHandleReopen: app, hasVisibleWindows: false]
    };
    let _: bool = unsafe {
        msg_send![&*current, applicationShouldHandleReopen: app, hasVisibleWindows: false]
    };
    let reopen_count: usize = unsafe { msg_send![&*current, tabbyRSProbeReopenCount] };
    let sentinel: u64 = unsafe { msg_send![&*current, tabbyRSProbeSentinel] };
    app.terminate(None);
    let requested_exit = crate::tauri_stub::recorded_exits() == [0];
    Ok(json!({
        "case": "preserves_original_callbacks_and_ivars",
        "passed": delegate_preserved && requested_exit && reopen_count == 2 && sentinel == 0x00AB_CDEF,
        "reopenCount": reopen_count,
        "sentinel": sentinel,
        "delegatePreserved": delegate_preserved,
    }))
}

#[cfg(target_os = "macos")]
fn case_changed_delegate(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use objc2::runtime::{AnyObject, ProtocolObject};
    use serde_json::json;

    let original = crate::probes::Original::new(mtm);
    let replacement = crate::probes::Original::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*original)));
    let handle = crate::tauri_stub::AppHandle::new();
    install_with(&handle)?;
    let original_object: &AnyObject = &original;
    let replacement_object: &AnyObject = &replacement;
    let installed_class = original_object.class();
    let original_class = replacement_object.class();
    if !installed_class
        .superclass()
        .is_some_and(|class| std::ptr::eq(class, original_class))
        || installed_class.instance_size() != original_class.instance_size()
    {
        return Err("installed class is not a layout-preserving delegate subclass".into());
    }
    // SAFETY: Both objects are the same original class; the installed class is
    // its zero-ivar subclass. This isolated probe mutates only on the main thread.
    unsafe { AnyObject::set_class(replacement_object, installed_class) };
    app.setDelegate(Some(ProtocolObject::from_ref(&*replacement)));
    let before = delegate_ptr(app)?;
    let errored = crate::subject::install(&handle).is_err();
    Ok(json!({
        "case": "changed_delegate_install_errors",
        "passed": errored && delegate_ptr(app)? == before,
        "result": if errored { "err" } else { "ok" },
    }))
}

#[cfg(target_os = "macos")]
fn case_non_main_thread(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use objc2::runtime::ProtocolObject;
    use serde_json::json;

    let original = crate::probes::Original::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*original)));

    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let handle = crate::tauri_stub::AppHandle::new();
        let _ = sender.send(crate::subject::install(&handle).is_err());
    });
    let errored = receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("off-main install wait: {error}"))?;
    Ok(json!({
        "case": "non_main_thread_install_errors",
        "passed": errored,
        "result": if errored { "err" } else { "ok" },
    }))
}

#[cfg(target_os = "macos")]
fn case_no_delegate(app: &objc2_app_kit::NSApplication) -> Result<serde_json::Value, String> {
    use serde_json::json;

    app.setDelegate(None);
    let handle = crate::tauri_stub::AppHandle::new();
    let errored = crate::subject::install(&handle).is_err();
    Ok(json!({
        "case": "no_delegate_install_errors",
        "passed": errored,
        "result": if errored { "err" } else { "ok" },
    }))
}

#[cfg(target_os = "macos")]
fn case_conflict(
    app: &objc2_app_kit::NSApplication,
    mtm: objc2::MainThreadMarker,
) -> Result<serde_json::Value, String> {
    use objc2::msg_send;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::NSApplicationTerminateReply;
    use serde_json::json;

    let conflict = crate::probes::Conflict::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*conflict)));

    let delegate_before = delegate_ptr(app)?;
    let class_before = delegate_class_ptr(app)?;

    let handle = crate::tauri_stub::AppHandle::new();
    let errored = crate::subject::install(&handle).is_err();

    let delegate_after = delegate_ptr(app)?;
    let class_after = delegate_class_ptr(app)?;
    let reply: NSApplicationTerminateReply =
        unsafe { msg_send![&*conflict, applicationShouldTerminate: app] };

    let delegate_preserved = delegate_before == delegate_after && class_before == class_after;
    let reply_preserved = reply == NSApplicationTerminateReply::TerminateCancel;
    Ok(json!({
        "case": "conflicting_delegate_install_errors",
        "passed": errored && delegate_preserved && reply_preserved,
        "result": if errored { "err" } else { "ok" },
        "delegatePreserved": delegate_preserved,
        "shouldTerminateReplyPreserved": reply_preserved,
    }))
}

#[cfg(target_os = "macos")]
fn delegate_ptr(app: &objc2_app_kit::NSApplication) -> Result<usize, String> {
    use objc2::rc::Retained;

    let delegate = app
        .delegate()
        .ok_or_else(|| "NSApp has no delegate".to_string())?;
    Ok(Retained::as_ptr(&delegate).cast::<()>() as usize)
}

#[cfg(target_os = "macos")]
fn delegate_class_ptr(app: &objc2_app_kit::NSApplication) -> Result<usize, String> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let delegate = app
        .delegate()
        .ok_or_else(|| "NSApp has no delegate".to_string())?;
    let class: *const AnyObject = unsafe { msg_send![&*delegate, class] };
    Ok(class as usize)
}
