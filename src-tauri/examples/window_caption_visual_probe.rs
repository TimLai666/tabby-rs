#[cfg(target_os = "windows")]
#[path = "window_caption_visual_probe/button.rs"]
mod button;

#[cfg(not(target_os = "windows"))]
fn main() {
    println!("Windows only; no native checks ran. --help describes the Windows hover/pixel probe.");
}

#[cfg(target_os = "windows")]
fn main() {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("window_caption_visual_probe: disposable Windows Wry standard Button with disabled layered visual experiment. Creates one isolated undecorated WebView and one native owner-drawn Button plus one disabled layered visual child. Checks exact UIA ElementFromPoint and raw/control/content trees before/after visible visual, alpha26 hover compositing over changing HTML, and retained Button input. Three external UIA calls each Invoke only the exact owned Button once; root independently requires exactly three BN_CLICKED notifications. Checks actual white/configured-color glyph pixels and blue/orange HTML through the hollow glyph. Uses tagged native SendInput MOVE, independently verifies child mouse source before relay, observes forwarded parent non-client hover, and verifies HTML compositor pixels before/after repaint. Sends no mouse-down. No click/Snap/DPI/product acceptance. Exit 0 requires every observation and owned cleanup; failures return 1. No parameters. Required environment: CAPTION_PROBE_UIA_HELPER is the owned UIA helper; CAPTION_PROBE_EVIDENCE is the existing owned output root, CAPTION_PROBE_CAPTURE_HELPER is the existing native capture script path. Run on an interactive Windows desktop.");
        return;
    }
    if let Err(error) = probe::run() {
        eprintln!("caption_visual_probe FAILED: {error}");
        std::process::exit(1);
    }
    println!("caption_visual_probe completed: verified=true fullFrameAccepted=false");
}

#[cfg(target_os = "windows")]
mod probe {
    use std::sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::sync_channel,
        Arc,
    };
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, GetDC, GetPixel, ReleaseDC};
    use windows_sys::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetClassNameW, GetForegroundWindow, GetMessageExtraInfo, GetParent,
        GetSystemMetrics, GetWindowThreadProcessId, IsWindow, WindowFromPoint, HTMAXBUTTON,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WM_COMMAND,
        WM_NCMOUSEMOVE,
    };

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows_sys::Win32::UI::Input::{
        GetCurrentInputMessageSource, IMDT_MOUSE, IMO_INJECTED, INPUT_MESSAGE_SOURCE,
    };

    static OWNED_CHILD: AtomicUsize = AtomicUsize::new(0);
    static CHILD_COMMANDS: AtomicUsize = AtomicUsize::new(0);
    static CHILD_REAL_MOVES: AtomicUsize = AtomicUsize::new(0);
    static CHILD_REJECTED_MOVES: AtomicUsize = AtomicUsize::new(0);
    static INPUT_TAG: AtomicUsize = AtomicUsize::new(0);
    const CHILD_OBSERVER: usize = 0x5441428a;
    static ROOT_CHILD_SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    static ROOT_MOVES: AtomicUsize = AtomicUsize::new(0);
    static EXPECTED_POINT: AtomicU32 = AtomicU32::new(0);
    const OBSERVER: usize = 0x54414289;
    const WAIT: Duration = Duration::from_secs(10);
    unsafe extern "system" fn observe(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if message == WM_COMMAND
            && (w >> 16) == 0
            && l as usize != 0
            && l as usize == OWNED_CHILD.load(Ordering::Acquire)
        {
            CHILD_COMMANDS.fetch_add(1, Ordering::AcqRel);
        }
        if message == WM_NCMOUSEMOVE
            && w == HTMAXBUTTON as usize
            && l as u32 == EXPECTED_POINT.load(Ordering::Acquire)
            && CHILD_REAL_MOVES.load(Ordering::Acquire) > 0
        {
            ROOT_CHILD_SEQUENCE.store(CHILD_REAL_MOVES.load(Ordering::Acquire), Ordering::Release);
            ROOT_MOVES.fetch_add(1, Ordering::Relaxed);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
    unsafe extern "system" fn observe_child(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if message == WM_NCMOUSEMOVE
            && w == HTMAXBUTTON as usize
            && l as u32 == EXPECTED_POINT.load(Ordering::Acquire)
        {
            let mut source: INPUT_MESSAGE_SOURCE = std::mem::zeroed();
            let available = GetCurrentInputMessageSource(&mut source) != 0;
            let tag = GetMessageExtraInfo() as usize;
            if available
                && source.deviceType == IMDT_MOUSE
                && source.originId == IMO_INJECTED
                && tag == INPUT_TAG.load(Ordering::Acquire)
            {
                let sequence = CHILD_REAL_MOVES.fetch_add(1, Ordering::AcqRel) + 1;
                eprintln!("caption_visual_probe actual child input: seq={sequence} hwnd={hwnd:?} device={} origin={} tag={tag:#x} point={l:#x}", source.deviceType, source.originId);
            } else if CHILD_REJECTED_MOVES.fetch_add(1, Ordering::Relaxed) == 0 {
                eprintln!("caption_visual_probe rejected child input source: available={available} device={} origin={} tag={tag:#x}",source.deviceType,source.originId);
            }
        }
        DefSubclassProc(hwnd, message, w, l)
    }
    fn move_cursor(point: POINT) -> Result<(), String> {
        unsafe {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if width < 2
                || height < 2
                || point.x < left
                || point.x >= left + width
                || point.y < top
                || point.y >= top + height
            {
                return Err("input point outside virtual desktop".into());
            }
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: (((point.x - left) as i64 * 65536 + 32768) / width as i64) as i32,
                        dy: (((point.y - top) as i64 * 65536 + 32768) / height as i64) as i32,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        time: 0,
                        dwExtraInfo: INPUT_TAG.load(Ordering::Acquire),
                    },
                },
            };
            if SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) != 1 {
                return Err(format!(
                    "owned mouse MOVE failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(())
        }
    }
    fn wait(
        label: &str,
        mut condition: impl FnMut() -> Result<bool, String>,
    ) -> Result<(), String> {
        let start = Instant::now();
        while start.elapsed() < WAIT {
            if condition()? {
                eprintln!("caption_visual_probe PASS: {label}");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err(format!("timed out: {label}"))
    }
    fn pixel(point: POINT) -> Result<u32, String> {
        unsafe {
            let dc = GetDC(std::ptr::null_mut());
            if dc.is_null() {
                return Err("GetDC desktop failed".into());
            }
            let color = GetPixel(dc, point.x, point.y);
            let released = ReleaseDC(std::ptr::null_mut(), dc);
            if color == u32::MAX || released == 0 {
                return Err("desktop pixel read or release failed".into());
            }
            Ok(color)
        }
    }
    fn capture_owned(window: &tauri::WebviewWindow, label: &str) -> Result<(), String> {
        let root = std::env::var_os("CAPTION_PROBE_EVIDENCE")
            .ok_or("CAPTION_PROBE_EVIDENCE is required")?;
        let helper = std::env::var_os("CAPTION_PROBE_CAPTURE_HELPER")
            .ok_or("CAPTION_PROBE_CAPTURE_HELPER is required")?;
        let directory = std::path::PathBuf::from(root).join(label);
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let stdout = std::fs::File::create_new(directory.join("capture-stdout.log"))
            .map_err(|e| e.to_string())?;
        let stderr = std::fs::File::create_new(directory.join("capture-stderr.log"))
            .map_err(|e| e.to_string())?;
        let mut child = std::process::Command::new("pwsh")
            .arg("-NoProfile")
            .arg("-File")
            .arg(helper)
            .arg("-AppProcessId")
            .arg(std::process::id().to_string())
            .arg("-WindowHandle")
            .arg((window.hwnd().map_err(|e| e.to_string())?.0 as usize).to_string())
            .arg("-OutputDirectory")
            .arg(&directory)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("owned capture {label} failed: {status}"))
                };
            }
            if start.elapsed() > Duration::from_secs(45) {
                child.kill().map_err(|e| e.to_string())?;
                child.wait().map_err(|e| e.to_string())?;
                return Err(format!(
                    "owned capture {label} timed out and was terminated"
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn check_accessible_button(
        window: &tauri::WebviewWindow,
        child: usize,
        label: &str,
    ) -> Result<(), String> {
        let root = std::env::var_os("CAPTION_PROBE_EVIDENCE").ok_or("evidence root required")?;
        let helper = std::env::var_os("CAPTION_PROBE_UIA_HELPER").ok_or("UIA helper required")?;
        let root = std::path::PathBuf::from(root);
        let output = std::fs::File::create_new(root.join(format!("uia-{label}-stdout.log")))
            .map_err(|e| e.to_string())?;
        let error = std::fs::File::create_new(root.join(format!("uia-{label}-stderr.log")))
            .map_err(|e| e.to_string())?;
        let mut process = std::process::Command::new("pwsh")
            .arg("-NoProfile")
            .arg("-File")
            .arg(helper)
            .arg("-AppProcessId")
            .arg(std::process::id().to_string())
            .arg("-ParentHandle")
            .arg((window.hwnd().map_err(|e| e.to_string())?.0 as usize).to_string())
            .arg("-ButtonHandle")
            .arg(child.to_string())
            .arg("-OutputPath")
            .arg(root.join(format!("button-accessibility-{label}.json")))
            .stdout(output)
            .stderr(error)
            .spawn()
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        loop {
            if let Some(status) = process.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("UIA helper failed: {status}"))
                };
            }
            if started.elapsed() > Duration::from_secs(45) {
                process.kill().map_err(|e| e.to_string())?;
                process.wait().map_err(|e| e.to_string())?;
                return Err("owned UIA helper exceeded45s and was terminated".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn near_color(actual: u32, expected: u32) -> bool {
        [0, 8, 16].into_iter().all(|shift| {
            let observed = ((actual >> shift) & 255) as i32;
            let target = ((expected >> shift) & 255) as i32;
            (observed - target).abs() <= 1
        })
    }
    fn verify_real_hover(point: POINT, button: usize) -> Result<(), String> {
        EXPECTED_POINT.store(
            (point.x as u16 as u32) | ((point.y as u16 as u32) << 16),
            Ordering::Release,
        );
        let before_child = CHILD_REAL_MOVES.load(Ordering::Acquire);
        let before_root = ROOT_MOVES.load(Ordering::Acquire);
        move_cursor(POINT {
            x: point.x - 100,
            y: point.y + 100,
        })?;
        std::thread::sleep(Duration::from_millis(100));
        move_cursor(point)?;
        let under = unsafe { WindowFromPoint(point) } as usize;
        if under != button {
            return Err(format!(
                "visual stole actual WindowFromPoint: expected={button:#x} actual={under:#x}"
            ));
        }
        wait(
            "fresh real child hover relayed after the same child sequence",
            || {
                Ok(CHILD_REAL_MOVES.load(Ordering::Acquire) > before_child
                    && ROOT_MOVES.load(Ordering::Acquire) > before_root
                    && ROOT_CHILD_SEQUENCE.load(Ordering::Acquire) > before_child)
            },
        )
    }
    fn cleanup_data(data: tempfile::TempDir) -> Result<(), String> {
        let path = data.path().to_path_buf();
        let started = Instant::now();
        loop {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    let _ = data.keep();
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _ = data.keep();
                    return Ok(());
                }
                Err(error) if started.elapsed() >= Duration::from_secs(5) => {
                    let retained = data.keep();
                    return Err(format!(
                        "isolated data cleanup failed; retained {}: {error}",
                        retained.display()
                    ));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
    fn on_ui<T: Send + 'static>(
        window: &tauri::WebviewWindow,
        task: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (send, receive) = sync_channel(1);
        window
            .run_on_main_thread(move || {
                let _ = send.send(task());
            })
            .map_err(|e| e.to_string())?;
        receive.recv_timeout(WAIT).map_err(|e| e.to_string())?
    }
    fn checks(window: &tauri::WebviewWindow, loaded: &AtomicBool) -> Result<(), String> {
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.is_null() {
            return Err("could not set input-thread DPI awareness".into());
        }
        let result = (|| {
            wait("actual page load", || Ok(loaded.load(Ordering::Acquire)))?;
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
            wait("owned parent foreground", || {
                Ok(unsafe { GetForegroundWindow() } as usize == parent)
            })?;
            let scale = window.scale_factor().map_err(|e| e.to_string())?;
            let rect = RECT {
                left: (200.0 * scale).round() as i32,
                top: (16.0 * scale).round() as i32,
                right: (246.0 * scale).round() as i32,
                bottom: (48.0 * scale).round() as i32,
            };
            let mut point = POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            };
            if unsafe { ClientToScreen(parent as HWND, &mut point) } == 0 {
                return Err("ClientToScreen failed".into());
            }
            eprintln!("caption_visual_probe identity: pid={} parent={parent:#x} dpiScale={scale} point={},{}",std::process::id(),point.x,point.y);
            let glyph = POINT {
                x: point.x - 5,
                y: point.y - 5,
            };
            let corner = POINT {
                x: point.x - (rect.right - rect.left) / 2 + 3,
                y: point.y - (rect.bottom - rect.top) / 2 + 3,
            };

            let mut baseline_color = 0;
            let baseline = wait("baseline actual HTML pixel", || {
                baseline_color = pixel(point)?;
                Ok(baseline_color == 0x00cc8800)
            });
            eprintln!("caption_visual_probe baseline actual color={baseline_color:#08x}, expected=0xcc8800");
            let captured = capture_owned(window, "baseline");
            if let Err(error) = &captured {
                eprintln!("caption_visual_probe baseline capture error: {error}");
            }
            baseline?;
            captured?;
            EXPECTED_POINT.store(
                (point.x as u16 as u32) | ((point.y as u16 as u32) << 16),
                Ordering::Release,
            );
            let tag = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos() as u32) as usize
                | 1;
            INPUT_TAG.store(tag, Ordering::Release);
            eprintln!("caption_visual_probe expected input tag={tag:#x}");
            let overlay = on_ui(window, move || unsafe {
                let class: Vec<u16> = "WRY_WEBVIEW\0".encode_utf16().collect();
                let container = FindWindowExW(
                    parent as HWND,
                    std::ptr::null_mut(),
                    class.as_ptr(),
                    std::ptr::null(),
                );
                if container.is_null() {
                    return Err("owned WRY_WEBVIEW container not found".into());
                }
                let mut owner = 0;
                let root_tid = GetWindowThreadProcessId(parent as HWND, &mut owner);
                let mut child_owner = 0;
                let child_tid = GetWindowThreadProcessId(container, &mut child_owner);
                if owner != std::process::id() || child_owner != owner || root_tid != child_tid {
                    return Err("container ownership/thread mismatch".into());
                }
                if SetWindowSubclass(parent as HWND, Some(observe), OBSERVER, 0) == 0 {
                    return Err("root observer install failed".into());
                }
                let child = super::button::install(parent as HWND, container, rect)?;
                let mut child_pid = 0;
                let child_tid = GetWindowThreadProcessId(child, &mut child_pid);
                if child.is_null()
                    || child_pid != owner
                    || child_tid != root_tid
                    || GetParent(child) != parent as HWND
                {
                    return Err("slot did not return its owned same-thread child".into());
                }
                if SetWindowSubclass(child, Some(observe_child), CHILD_OBSERVER, 0) == 0 {
                    return Err("independent child input observer install failed".into());
                }
                Ok(child as usize)
            })?;
            eprintln!("caption_visual_probe overlay={overlay:#x}");
            let mut class = [0u16; 64];
            let len =
                unsafe { GetClassNameW(overlay as HWND, class.as_mut_ptr(), class.len() as i32) };
            if len <= 0 || String::from_utf16_lossy(&class[..len as usize]) != "Button" {
                return Err("overlay is not a native standard Button".into());
            }
            OWNED_CHILD.store(overlay, Ordering::Release);
            check_accessible_button(window, overlay, "baseline")?;
            wait(
                "exactly one UIA Invoke reaches owned root as BN_CLICKED",
                || Ok(CHILD_COMMANDS.load(Ordering::Acquire) == 1),
            )?;

            on_ui(window, move || unsafe {
                super::button::enable_visual(overlay as HWND)
            })?;
            check_accessible_button(window, overlay, "visual")?;
            wait("exactly two UIA Invokes reach owned root", || {
                Ok(CHILD_COMMANDS.load(Ordering::Acquire) == 2)
            })?;
            for hit in [point, glyph, corner] {
                verify_real_hover(hit, overlay)?;
            }
            wait("HTML pixels survive transparent hit overlay", || {
                Ok(pixel(point)? == 0x00cc8800 && pixel(corner)? == 0x00cc8800)
            })?;
            wait("actual white self-painted glyph pixel", || {
                Ok(pixel(glyph)? == 0x00ffffff
                    && pixel(point)? == 0x00cc8800
                    && pixel(corner)? == 0x00cc8800)
            })?;
            capture_owned(window, "white-symbol")?;
            let layer = on_ui(window, move || unsafe {
                let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                let visual = FindWindowExW(
                    overlay as HWND,
                    std::ptr::null_mut(),
                    class.as_ptr(),
                    std::ptr::null(),
                );
                if visual.is_null() {
                    return Err("owned visual is absent for error checks".into());
                }
                for (width, height) in [
                    (0, 32),
                    (46, 0),
                    (-1, 32),
                    (9, 32),
                    (46, 9),
                    (i32::MAX, i32::MAX),
                ] {
                    if super::button::visual::render(visual, width, height, 0x00ffffff, false)
                        .is_ok()
                    {
                        return Err(format!(
                            "invalid visual dimensions accepted: {width}x{height}"
                        ));
                    }
                }
                if super::button::visual::render(std::ptr::null_mut(), 46, 32, 0x00ffffff, false)
                    .is_ok()
                {
                    return Err("null visual accepted".into());
                }
                Ok(visual as usize)
            })?;
            if unsafe { super::button::visual::render(layer as HWND, 46, 32, 0x00ffffff, false) }
                .is_ok()
            {
                return Err("visual renderer accepted wrong UI thread".into());
            }
            wait(
                "invalid render requests preserve white glyph and transparent corners",
                || {
                    Ok(pixel(glyph)? == 0x00ffffff
                        && pixel(point)? == 0x00cc8800
                        && pixel(corner)? == 0x00cc8800)
                },
            )?;

            on_ui(window, move || unsafe {
                super::button::set_symbol_color(overlay as HWND, 0x00d050b0)
            })?;
            wait("actual configured symbol color", || {
                Ok(pixel(glyph)? == 0x00d050b0)
            })?;
            wait("hollow glyph center preserves blue HTML", || {
                Ok(pixel(point)? == 0x00cc8800 && pixel(corner)? == 0x00cc8800)
            })?;
            on_ui(window, move || unsafe {
                super::button::set_hover(overlay as HWND, true)
            })?;
            wait(
                "actual white-alpha26 hover over blue and retained configured glyph",
                || {
                    Ok(near_color(pixel(point)?, 0x00d1941a)
                        && near_color(pixel(corner)?, 0x00d1941a)
                        && pixel(glyph)? == 0x00d050b0)
                },
            )?;
            capture_owned(window, "blue-hover")?;
            for hit in [point, glyph, corner] {
                verify_real_hover(hit, overlay)?;
            }
            check_accessible_button(window, overlay, "hover")?;
            wait("exactly three UIA Invokes reach owned root", || {
                Ok(CHILD_COMMANDS.load(Ordering::Acquire) == 3)
            })?;
            window
                .eval("document.getElementById('probe').style.background='#d84e32'")
                .map_err(|e| e.to_string())?;
            wait(
                "HTML repaint preserves orange-alpha26 hover blend and configured glyph",
                || {
                    Ok(near_color(pixel(point)?, 0x004760dc)
                        && near_color(pixel(corner)?, 0x004760dc)
                        && pixel(glyph)? == 0x00d050b0)
                },
            )?;
            capture_owned(window, "repainted-hover")?;
            on_ui(window, move || unsafe {
                super::button::set_hover(overlay as HWND, false)
            })?;
            wait(
                "cleared hover preserves exact orange HTML and configured glyph",
                || {
                    Ok(pixel(point)? == 0x00324ed8
                        && pixel(corner)? == 0x00324ed8
                        && pixel(glyph)? == 0x00d050b0)
                },
            )?;
            for hit in [point, glyph, corner] {
                verify_real_hover(hit, overlay)?;
            }
            capture_owned(window, "repainted-clear")?;
            if CHILD_COMMANDS.load(Ordering::Acquire) != 3 {
                return Err("UIA command count changed or duplicated".into());
            }
            Ok(())
        })();
        if unsafe { SetThreadDpiAwarenessContext(previous) }.is_null() {
            return Err("could not restore input-thread DPI awareness".into());
        }
        result
    }
    pub fn run() -> Result<(), String> {
        let data = tempfile::tempdir().map_err(|e| e.to_string())?;
        let loaded = Arc::new(AtomicBool::new(false));
        let loaded_callback = loaded.clone();
        let mut context = tauri::generate_context!();
        eprintln!(
            "caption_visual_probe embedded frontend config: {:?}",
            context.config().build.frontend_dist
        );
        if !matches!(
            context.config().build.frontend_dist,
            Some(tauri::utils::config::FrontendDist::Directory(_))
        ) {
            return Err("probe requires an embedded frontend directory; Windows absolute drive paths can parse as URLs".into());
        }
        context.config_mut().identifier =
            format!("io.tabbyrs.tests.caption-visual.{}", std::process::id());
        context.config_mut().app.windows.clear();
        let directory = data.path().to_path_buf();
        let (ready_send, ready_receive) = sync_channel(1);
        let (done_send, done_receive) = sync_channel(1);
        let app = tauri::Builder::default()
            .setup(move |app| {
                let callback = loaded_callback.clone();
                let window = tauri::WebviewWindowBuilder::new(
                    app,
                    "captionProbe",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .title("Owned caption overlay routing experiment")
                .visible(false)
                .decorations(false)
                .inner_size(640.0, 480.0)
                .position(32.0, 32.0)
                .incognito(true)
                .data_directory(directory)
                .on_page_load(move |_, payload| {
                    if payload.event() == tauri::webview::PageLoadEvent::Finished {
                        callback.store(true, Ordering::Release);
                    }
                })
                .build()?;
                ready_send.send(window).map_err(std::io::Error::other)?;
                Ok(())
            })
            .build(context)
            .map_err(|e| e.to_string())?;
        let handle = app.handle().clone();
        let worker = std::thread::spawn(move || {
            let result = ready_receive
                .recv_timeout(Duration::from_secs(60))
                .map_err(|e| e.to_string())
                .and_then(|window| {
                    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
                    let tested = checks(&window, &loaded);
                    if tested.is_err() {
                        if let Err(error) = capture_owned(&window, "failure") {
                            eprintln!("caption_visual_probe failure capture failed: {error}");
                        }
                    }
                    window.destroy().map_err(|e| e.to_string())?;
                    wait("owned parent destroyed", || {
                        Ok(unsafe { IsWindow(hwnd as HWND) } == 0)
                    })?;
                    tested
                });
            let code = if result.is_ok() { 0 } else { 1 };
            let _ = done_send.send(result);
            handle.exit(code);
        });
        let exit_code = app.run_return(|_, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
        wait("worker returned its owned window handles", || {
            Ok(worker.is_finished())
        })?;
        worker.join().map_err(|_| "worker panicked".to_string())?;
        let tested = done_receive.recv_timeout(WAIT).map_err(|e| e.to_string())?;
        if let Err(error) = &tested {
            eprintln!("caption_visual_probe observed failure before cleanup: {error}");
        }
        cleanup_data(data)?;
        if exit_code != 0 {
            return Err(format!("event-loop returned failure code {exit_code}"));
        }
        tested
    }
}
