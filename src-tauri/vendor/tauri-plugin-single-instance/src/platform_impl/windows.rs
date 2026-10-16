// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

#[cfg(feature = "semver")]
use crate::semver_compat::semver_compat_string;

use crate::windows_message::{
    decode_payload, encode_payload, WireFormat, JSON_DATA_ID, JSON_PROTOCOL_REPLY, LEGACY_DATA_ID,
    QUERY_MESSAGE,
};
use crate::SingleInstanceCallback;
use std::sync::Mutex;
use tauri::{
    plugin::{self, TauriPlugin},
    AppHandle, Manager, RunEvent, Runtime,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_CLASS_ALREADY_EXISTS,
        HWND, LPARAM, LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
    },
    System::{
        DataExchange::COPYDATASTRUCT,
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
    UI::WindowsAndMessaging::{
        self as w32wm, CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW,
        RegisterClassExW, SendMessageW, CREATESTRUCTW, GWLP_USERDATA, GWL_STYLE,
        WINDOW_LONG_PTR_INDEX, WM_COPYDATA, WM_CREATE, WM_DESTROY, WNDCLASSEXW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_OVERLAPPED, WS_POPUP, WS_VISIBLE,
    },
};

// How long a secondary waits on the shared mutex before looking for the owner
// window again. A live owner that has not published its window keeps the launch
// pending; ownership is never assumed from a timeout.
const OWNER_POLL_MILLISECONDS: u32 = 20;

struct MutexGuard {
    handle: isize,
    acquired: bool,
}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        if self.handle == 0 {
            return;
        }
        if self.acquired {
            unsafe { ReleaseMutex(self.handle as _) };
        }
        unsafe { CloseHandle(self.handle as _) };
    }
}

// The receiver window and its mutex ownership live together so teardown cannot
// split them or release a handle that has already been reused.
struct OwnedInstance {
    window: isize,
    _mutex: MutexGuard,
}

struct UserData<R: Runtime> {
    app: AppHandle<R>,
    callback: Box<SingleInstanceCallback<R>>,
}

impl<R: Runtime> UserData<R> {
    unsafe fn from_hwnd_raw(hwnd: HWND) -> *mut Self {
        GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Self
    }

    unsafe fn from_hwnd<'a>(hwnd: HWND) -> Option<&'a mut Self> {
        Self::from_hwnd_raw(hwnd).as_mut()
    }

    fn run_callback(&mut self, args: Vec<String>, cwd: String) {
        (self.callback)(&self.app, args, cwd)
    }
}

fn owner_window(class_name: &[u16], window_name: &[u16]) -> Option<HWND> {
    let hwnd = unsafe { FindWindowW(class_name.as_ptr(), window_name.as_ptr()) };
    (!hwnd.is_null()).then_some(hwnd)
}

fn forward_to_owner(hwnd: HWND) -> std::io::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let cwd = cwd.to_str().unwrap_or_default();

    let args = std::env::args().collect::<Vec<String>>();
    let format = if unsafe { SendMessageW(hwnd, QUERY_MESSAGE, 0, 0) } == JSON_PROTOCOL_REPLY {
        WireFormat::Json
    } else {
        WireFormat::Legacy
    };
    let bytes = encode_payload(cwd, &args, format)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let cb_data = u32::try_from(bytes.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Windows instance payload is too large",
        )
    })?;
    let cds = COPYDATASTRUCT {
        dwData: match format {
            WireFormat::Json => JSON_DATA_ID,
            WireFormat::Legacy => LEGACY_DATA_ID,
        },
        cbData: cb_data,
        lpData: bytes.as_ptr() as _,
    };

    let result = unsafe { SendMessageW(hwnd, WM_COPYDATA, 0, &cds as *const _ as _) };
    if result != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "the running instance rejected the launch request",
        ));
    }
    Ok(())
}

pub fn init<R: Runtime>(
    callback: Box<SingleInstanceCallback<R>>,
    instance_id: Option<String>,
) -> TauriPlugin<R> {
    plugin::Builder::new("single-instance")
        .setup(move |app, _api| {
            #[allow(unused_mut)]
            let mut id = instance_id.unwrap_or_else(|| app.config().identifier.clone());
            #[cfg(feature = "semver")]
            {
                id.push('_');
                id.push_str(semver_compat_string(&app.package_info().version).as_str());
            }

            let class_name = encode_wide(format!("{id}-sic"));
            let window_name = encode_wide(format!("{id}-siw"));
            let mutex_name = encode_wide(format!("{id}-sim"));

            unsafe { SetLastError(0) };
            let hmutex =
                unsafe { CreateMutexW(std::ptr::null(), true.into(), mutex_name.as_ptr()) };
            if hmutex.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }

            // A handle to an existing mutex does not grant ownership. Retain the
            // handle and either hand off to the owner or wait for ownership.
            let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            let mut mutex = MutexGuard {
                handle: hmutex as _,
                acquired: !already_exists,
            };

            if already_exists {
                loop {
                    if let Some(hwnd) = owner_window(&class_name, &window_name) {
                        forward_to_owner(hwnd)?;
                        app.cleanup_before_exit();
                        std::process::exit(0);
                    }

                    match unsafe { WaitForSingleObject(mutex.handle as _, OWNER_POLL_MILLISECONDS) }
                    {
                        WAIT_TIMEOUT => {}
                        WAIT_OBJECT_0 | WAIT_ABANDONED => {
                            mutex.acquired = true;
                            if let Some(hwnd) = owner_window(&class_name, &window_name) {
                                forward_to_owner(hwnd)?;
                                app.cleanup_before_exit();
                                std::process::exit(0);
                            }
                            break;
                        }
                        _ => return Err(std::io::Error::last_os_error().into()),
                    }
                }
            }

            let userdata = UserData {
                app: app.clone(),
                callback,
            };
            let hwnd = match create_event_target_window::<R>(&class_name, &window_name, userdata) {
                Ok(hwnd) => hwnd,
                Err(error) => return Err(error.into()),
            };
            app.manage(Mutex::new(Some(OwnedInstance {
                window: hwnd as _,
                _mutex: mutex,
            })));

            Ok(())
        })
        .on_event(|app, event| {
            if let RunEvent::Exit = event {
                destroy(app);
            }
        })
        .build()
}

pub fn destroy<R: Runtime, M: Manager<R>>(manager: &M) {
    let Some(state) = manager.try_state::<Mutex<Option<OwnedInstance>>>() else {
        return;
    };
    let mut instance = match state.lock() {
        Ok(instance) => instance,
        Err(poisoned) => poisoned.into_inner(),
    };
    let window = match instance.as_ref() {
        Some(owned) => owned.window,
        None => return,
    };

    // The receiver must be destroyed before its mutex ownership is released.
    // DestroyWindow can only succeed on the owning thread, so a wrong-thread
    // failure keeps the instance for that thread to clean up later.
    if unsafe { DestroyWindow(window as _) } == 0 {
        let error = unsafe { GetLastError() };
        tracing::error!(
            "single-instance: could not destroy the receiver window (OS error {error}); \
             retaining the receiver and its mutex ownership"
        );
        return;
    }
    drop(instance.take());
}

unsafe extern "system" fn single_instance_window_proc<R: Runtime>(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        QUERY_MESSAGE => JSON_PROTOCOL_REPLY,

        WM_CREATE => {
            let create_struct = &*(lparam as *const CREATESTRUCTW);
            let slot = create_struct.lpCreateParams as *mut Option<Box<UserData<R>>>;
            if slot.is_null() {
                return -1;
            }
            let Some(userdata) = (*slot).take() else {
                return -1;
            };
            let userdata = Box::into_raw(userdata);

            SetLastError(0);
            let previous = SetWindowLongPtrW(hwnd, GWLP_USERDATA, userdata as isize);
            if previous == 0 && GetLastError() != 0 {
                // Installation failed, so the caller still owns this allocation.
                *slot = Some(Box::from_raw(userdata));
                return -1;
            }
            0
        }

        WM_COPYDATA => {
            if lparam == 0 {
                return 0;
            }
            let cds_ptr = lparam as *const COPYDATASTRUCT;
            let cds = &*cds_ptr;
            let format = match cds.dwData {
                LEGACY_DATA_ID => WireFormat::Legacy,
                JSON_DATA_ID => WireFormat::Json,
                _ => return 0,
            };
            if cds.lpData.is_null() || cds.cbData == 0 || cds.cbData as usize > isize::MAX as usize
            {
                return 0;
            }
            let bytes = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
            let Some((args, cwd)) = decode_payload(bytes, format) else {
                return 0;
            };
            let Some(userdata) = UserData::<R>::from_hwnd(hwnd) else {
                return 0;
            };
            userdata.run_callback(args, cwd);
            1
        }

        WM_DESTROY => {
            let userdata = UserData::<R>::from_hwnd_raw(hwnd);
            if !userdata.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(userdata));
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn create_event_target_window<R: Runtime>(
    class_name: &[u16],
    window_name: &[u16],
    userdata: UserData<R>,
) -> std::io::Result<HWND> {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(single_instance_window_proc::<R>),
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

        SetLastError(0);
        if RegisterClassExW(&class) == 0 {
            let error = GetLastError();
            if error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(std::io::Error::from_raw_os_error(error as i32));
            }
        }

        // The allocation is owned here until WM_CREATE transfers it exactly once
        // with Box::into_raw. Any failure before that transfer drops it here.
        let mut slot: Option<Box<UserData<R>>> = Some(Box::new(userdata));
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE
            | WS_EX_TRANSPARENT
            | WS_EX_LAYERED
            // WS_EX_TOOLWINDOW prevents this window from ever showing up in the taskbar, which
            // we want to avoid. If you remove this style, this window won't show up in the
            // taskbar *initially*, but it can show up at some later point. This can sometimes
            // happen on its own after several hours have passed, although this has proven
            // difficult to reproduce. Alternatively, it can be manually triggered by killing
            // `explorer.exe` and then starting the process back up.
            // It is unclear why the bug is triggered by waiting for several hours.
            | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            &mut slot as *mut Option<Box<UserData<R>>> as _,
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            // The window technically has to be visible to receive WM_PAINT messages (which are used
            // for delivering events during resizes), but it isn't displayed to the user because of
            // the LAYERED style.
            (WS_VISIBLE | WS_POPUP) as isize,
        );
        Ok(hwnd)
    }
}

pub fn encode_wide(string: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    std::os::windows::prelude::OsStrExt::encode_wide(string.as_ref())
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(target_pointer_width = "32")]
#[allow(non_snake_case)]
unsafe fn SetWindowLongPtrW(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX, value: isize) -> isize {
    w32wm::SetWindowLongW(hwnd, index, value as _) as _
}

#[cfg(target_pointer_width = "64")]
#[allow(non_snake_case)]
unsafe fn SetWindowLongPtrW(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX, value: isize) -> isize {
    w32wm::SetWindowLongPtrW(hwnd, index, value)
}

#[cfg(target_pointer_width = "32")]
#[allow(non_snake_case)]
unsafe fn GetWindowLongPtrW(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    w32wm::GetWindowLongW(hwnd, index) as _
}

#[cfg(target_pointer_width = "64")]
#[allow(non_snake_case)]
unsafe fn GetWindowLongPtrW(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    w32wm::GetWindowLongPtrW(hwnd, index)
}
