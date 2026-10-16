//! Bounded actual Win32 checks for the production caption adapter.
//! This is not full Tauri/WebView/desktop parity acceptance.
#[cfg(target_os = "windows")]
mod caption {
    include!("../src/platform/frame_windows_buttons.rs");

    // Simulate an OS theme changing after the last cached snapshot, without
    // changing the user's registry or adding a production test API.
    pub(super) fn stale_theme_cache(root: windows_sys::Win32::Foundation::HWND) {
        let state = OWNED
            .with(|map| map.borrow().get(&(root as isize)).cloned())
            .unwrap();
        let actual = state.hover_color.borrow().clone();
        state.hover_color.replace(if actual == "#ffffff" {
            "#000000".into()
        } else {
            "#ffffff".into()
        });
    }
}
#[path = "../src/window_caption.rs"]
mod desktop;

#[cfg(target_os = "windows")]
fn run_checks() -> Result<usize, String> {
    use std::{
        mem::zeroed,
        ptr,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::COLOR_WINDOW,
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::Input::KeyboardAndMouse::GetCapture,
        UI::WindowsAndMessaging::*,
    };

    const CLASS: &[u16] = &[87, 67, 97, 112, 78, 97, 116, 105, 118, 101, 0];
    struct RootGuard(HWND);
    impl Drop for RootGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.0.is_null() && IsWindow(self.0) != 0 {
                    let _ = caption::remove(self.0);
                    DestroyWindow(self.0);
                }
            }
        }
    }
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_CLOSE => {
                let prior = GetPropW(hwnd, CLOSE_PROP.as_ptr()) as usize;
                SetPropW(hwnd, CLOSE_PROP.as_ptr(), (prior + 1) as _);
                0
            }
            WM_NCDESTROY => {
                RemovePropW(hwnd, CLOSE_PROP.as_ptr());
                DefWindowProcW(hwnd, msg, w, l)
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
    static CLOSE_PROP: &[u16] = &[67, 108, 111, 115, 101, 82, 101, 113, 117, 101, 115, 116, 0];
    unsafe fn make_root() -> Result<(HWND, HWND), String> {
        let instance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: (COLOR_WINDOW + 1) as _,
            lpszMenuName: ptr::null(),
            lpszClassName: CLASS.as_ptr(),
        };
        RegisterClassW(&wc);
        let root = CreateWindowExW(
            0,
            CLASS.as_ptr(),
            [78, 97, 116, 105, 118, 101, 0].as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            80,
            80,
            640,
            420,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if root.is_null() {
            return Err("CreateWindowExW root failed".into());
        }
        let child = CreateWindowExW(
            0,
            [83, 84, 65, 84, 73, 67, 0].as_ptr(),
            [
                87, 101, 98, 86, 105, 101, 119, 32, 98, 111, 117, 110, 100, 97, 114, 121, 0,
            ]
            .as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            640,
            390,
            root,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if child.is_null() {
            DestroyWindow(root);
            return Err("CreateWindowExW STATIC child failed".into());
        }
        Ok((root, child))
    }
    unsafe fn pump_until(mut pred: impl FnMut() -> bool) -> Result<(), String> {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(350) {
            let mut msg: MSG = zeroed();
            while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if pred() {
                return Ok(());
            }
            std::thread::yield_now();
        }
        if pred() {
            Ok(())
        } else {
            Err("bounded message pump deadline exceeded".into())
        }
    }
    unsafe fn children(root: HWND) -> Vec<HWND> {
        let mut out = Vec::new();
        let mut child = GetWindow(root, GW_CHILD);
        while !child.is_null() {
            out.push(child);
            child = GetWindow(child, GW_HWNDNEXT);
        }
        out
    }
    unsafe fn label(hwnd: HWND) -> String {
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
    unsafe fn button(root: HWND, expected: &str) -> Result<HWND, String> {
        children(root)
            .into_iter()
            .find(|h| label(*h) == expected)
            .ok_or_else(|| format!("native child button {expected:?} missing"))
    }
    unsafe fn button_count(root: HWND) -> usize {
        children(root)
            .into_iter()
            .filter(|h| {
                let mut class = [0u16; 16];
                let n = GetClassNameW(*h, class.as_mut_ptr(), class.len() as i32);
                String::from_utf16_lossy(&class[..n.max(0) as usize]) == "Button"
            })
            .count()
    }
    unsafe fn click(root: HWND, expected: &str) -> Result<(), String> {
        let b = button(root, expected)?;
        SendMessageW(b, BM_CLICK, 0, 0);
        Ok(())
    }
    let mut checks = 0usize;
    unsafe {
        let (root, web) = make_root()?;
        let _root_guard = RootGuard(root);
        let outer_before = {
            let mut r = zeroed();
            GetWindowRect(root, &mut r);
            r
        };
        let child_ok = GetParent(web) == root;
        if !child_ok {
            DestroyWindow(root);
            return Err("STATIC child is not owned by root".into());
        }
        checks += 1;
        let invalid = caption::snapshot(ptr::null_mut());
        if invalid.is_ok() {
            DestroyWindow(root);
            return Err("null HWND was accepted".into());
        }
        checks += 1;
        let root_bits = root as isize;
        let wrong_thread = std::thread::spawn(move || caption::snapshot(root_bits as HWND))
            .join()
            .map_err(|_| "worker panic")?;
        if wrong_thread.is_ok() {
            DestroyWindow(root);
            return Err("foreign-thread HWND was accepted".into());
        }
        checks += 1;

        let events = Arc::new(AtomicUsize::new(0));
        let event_counter = events.clone();
        caption::install(
            root,
            Box::new(move |_| {
                event_counter.fetch_add(1, Ordering::SeqCst);
            }),
        )?;
        let initial = caption::snapshot(root)?.ok_or("fresh install returned no state")?;
        let mut client = zeroed();
        GetClientRect(root, &mut client);
        let outer_width = outer_before.right - outer_before.left;
        let outer_height = outer_before.bottom - outer_before.top;
        if initial.scale_factor != 1.0
            || client.left != 0
            || client.top != 0
            || client.right != outer_width
            || client.bottom != outer_height
            || initial.buttons.len() != 3
            || initial
                .buttons
                .iter()
                .map(|b| (b.width, b.height))
                .collect::<Vec<_>>()
                != [(45, 31), (46, 31), (46, 31)]
            || initial.buttons[0].x != client.right - 138
            || initial.buttons[1].x != client.right - 93
            || initial.buttons[2].x != client.right - 47
            || initial.buttons.iter().any(|b| b.y != 1)
            || initial.buttons[2].x + initial.buttons[2].width as i32 != client.right - 1
        {
            caption::remove(root)?;
            DestroyWindow(root);
            return Err(format!("unexpected fresh geometry: {initial:?}"));
        }
        let mut after = zeroed();
        GetWindowRect(root, &mut after);
        if outer_before.left != after.left
            || outer_before.top != after.top
            || outer_before.right != after.right
            || outer_before.bottom != after.bottom
        {
            caption::remove(root)?;
            DestroyWindow(root);
            return Err("install changed outer bounds".into());
        }
        checks += 1;
        let prior_events = events.load(Ordering::SeqCst);
        let prior_button_count = button_count(root);
        caption::install(root, Box::new(|_| {}))?;
        if events.load(Ordering::SeqCst) != prior_events
            || button_count(root) != prior_button_count
            || prior_button_count != 3
        {
            caption::remove(root)?;
            DestroyWindow(root);
            return Err("reapply emitted a duplicate install event".into());
        }
        checks += 1;
        let min = button(root, "Minimize")?;
        let max = button(root, "Maximize")?;
        let close = button(root, "Close")?;
        for h in [min, max, close] {
            let mut class = [0u16; 16];
            let n = GetClassNameW(h, class.as_mut_ptr(), class.len() as i32);
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            if String::from_utf16_lossy(&class[..n.max(0) as usize]) != "Button"
                || style & WS_TABSTOP != 0
                || GetParent(h) != root
            {
                caption::remove(root)?;
                DestroyWindow(root);
                return Err("caption child is not an owned non-tabstop native BUTTON".into());
            }
        }
        checks += 1;

        // Operations target only the HWND whose native child was activated.
        let (other, _) = make_root()?;
        let _other_guard = RootGuard(other);
        caption::install(other, Box::new(|_| {}))?;
        click(root, "Maximize")?;
        pump_until(|| IsZoomed(root) != 0)?;
        if IsZoomed(other) != 0 {
            caption::remove(root)?;
            DestroyWindow(other);
            DestroyWindow(root);
            return Err("maximize changed independent window".into());
        }
        let maxed = caption::snapshot(root)?.ok_or("maximized snapshot absent")?;
        if !maxed.maximized || label(button(root, "Restore")?) != "Restore" {
            return Err("BM_CLICK did not expose actual maximized/Restore state".into());
        }
        checks += 1;
        click(root, "Restore")?;
        pump_until(|| IsZoomed(root) == 0)?;
        if IsZoomed(other) != 0 {
            return Err("restore changed independent window".into());
        }
        checks += 1;
        click(root, "Minimize")?;
        pump_until(|| IsIconic(root) != 0)?;
        if IsIconic(other) != 0 {
            return Err("minimize changed independent window".into());
        }
        ShowWindow(root, SW_RESTORE);
        pump_until(|| IsIconic(root) == 0)?;
        checks += 1;

        // Exercise the standard BUTTON's captured press path, cancellation,
        // release outside, and re-entry using HWND messages (not physical input).
        let b = button(root, "Maximize")?;
        let (bw, bh) = {
            let mut r = zeroed();
            GetClientRect(b, &mut r);
            (r.right, r.bottom)
        };
        let inside = (((bh / 2) as u16 as isize) << 16) | (bw / 2) as u16 as isize;
        let outside = (((bh + 8) as u16 as isize) << 16) | (bw as u16 as isize);
        SendMessageW(b, WM_LBUTTONDOWN, 1, inside);
        if GetCapture() != b || caption::snapshot(root)?.unwrap().buttons[1].pressed != true {
            return Err(
                "native BUTTON did not own capture and pressed state after client down".into(),
            );
        }
        SendMessageW(b, WM_MOUSEMOVE, 1, outside);
        if caption::snapshot(root)?.unwrap().buttons[1].pressed {
            return Err("moving outside retained pressed state".into());
        }
        SendMessageW(b, WM_LBUTTONUP, 0, outside);
        if IsZoomed(root) != 0 || GetCapture() == b {
            return Err("release outside activated maximize".into());
        }
        SendMessageW(b, WM_LBUTTONDOWN, 1, inside);
        if GetCapture() != b {
            return Err("button did not capture before cancellation".into());
        }
        SendMessageW(b, WM_CANCELMODE, 0, 0);
        SendMessageW(b, WM_LBUTTONUP, 0, inside);
        if IsZoomed(root) != 0
            || GetCapture() == b
            || caption::snapshot(root)?.unwrap().buttons[1].pressed
            || GetPropW(root, CLOSE_PROP.as_ptr()) as usize != 0
        {
            return Err("cancelled press activated or emitted state".into());
        }
        SendMessageW(b, WM_LBUTTONDOWN, 1, inside);
        if GetCapture() != b {
            return Err("re-entry path did not start with native capture".into());
        }
        SendMessageW(b, WM_MOUSEMOVE, 1, outside);
        SendMessageW(b, WM_MOUSEMOVE, 1, inside);
        SendMessageW(b, WM_LBUTTONUP, 0, inside);
        pump_until(|| IsZoomed(root) != 0)?;
        if caption::snapshot(root)?.is_none() || IsZoomed(other) != 0 {
            return Err("re-entry activation affected wrong native state".into());
        }
        ShowWindow(root, SW_RESTORE);
        pump_until(|| IsZoomed(root) == 0)?;
        checks += 1;

        // Close activation posts one WM_CLOSE; the parent intercepts it and
        // keeps the root alive for the existing close-confirmation path.
        click(root, "Close")?;
        pump_until(|| GetPropW(root, CLOSE_PROP.as_ptr()) as usize != 0)?;
        if GetPropW(root, CLOSE_PROP.as_ptr()) as usize != 1
            || IsWindow(root) == 0
            || IsWindow(other) == 0
        {
            return Err(
                "close button did not post exactly one WM_CLOSE and retain both roots".into(),
            );
        }
        click(other, "Maximize")?;
        pump_until(|| IsZoomed(other) != 0)?;
        if caption::snapshot(root)?.is_none() || IsWindow(root) == 0 {
            return Err("second window became unusable after first received WM_CLOSE".into());
        }
        checks += 1;

        // An externally destroyed owned caption child invalidates that
        // adapter instance. Cleanup must stay scoped to its own hooks/buttons.
        let detached_ref = Arc::new(());
        let detached_capture = detached_ref.clone();
        let detached_none_events = Arc::new(AtomicUsize::new(0));
        let detached_none_capture = detached_none_events.clone();
        let detached_all_capture = events.clone();
        caption::remove(root)?;
        caption::install(
            root,
            Box::new(move |state| {
                let _keep_alive = &detached_capture;
                detached_all_capture.fetch_add(1, Ordering::SeqCst);
                if state.is_none() {
                    detached_none_capture.fetch_add(1, Ordering::SeqCst);
                }
            }),
        )?;
        if Arc::strong_count(&detached_ref) != 2 {
            return Err("detach observer capture was not retained".into());
        }
        let native_min = button(root, "Minimize")?;
        DestroyWindow(native_min);
        if detached_none_events.load(Ordering::SeqCst) != 1 {
            return Err("destroyed own caption child did not emit exactly one None state".into());
        }
        if caption::snapshot(root)?.is_some() {
            return Err(
                "snapshot remained available after external caption-button destruction".into(),
            );
        }
        if Arc::strong_count(&detached_ref) != 1 {
            return Err("external button destruction leaked the observer capture".into());
        }
        if IsWindow(root) == 0
            || IsWindow(web) == 0
            || GetParent(web) != root
            || button_count(root) != 0
        {
            return Err(
                "external button destruction removed or invalidated a non-owned child/root".into(),
            );
        }
        let extra = CreateWindowExW(
            0,
            [83, 84, 65, 84, 73, 67, 0].as_ptr(),
            [
                70, 111, 114, 101, 105, 103, 110, 32, 99, 104, 105, 108, 100, 0,
            ]
            .as_ptr(),
            WS_CHILD | WS_VISIBLE,
            7,
            9,
            81,
            27,
            root,
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        );
        if extra.is_null() {
            return Err("could not create unrelated STATIC child".into());
        }
        let mut extra_before: windows_sys::Win32::Foundation::RECT = zeroed();
        GetWindowRect(extra, &mut extra_before);
        if GetParent(extra) != root {
            return Err("unrelated STATIC child lost its root parent".into());
        }
        let mut extra_after: windows_sys::Win32::Foundation::RECT = zeroed();
        caption::remove(root)?;
        let detached_snapshot = caption::snapshot(root)?;
        GetWindowRect(extra, &mut extra_after);
        if extra_before.left != extra_after.left
            || extra_before.top != extra_after.top
            || extra_before.right != extra_after.right
            || extra_before.bottom != extra_after.bottom
        {
            return Err("adapter cleanup moved the unrelated child".into());
        }
        if detached_snapshot.is_some()
            || IsWindow(extra) == 0
            || Arc::strong_count(&detached_ref) != 1
        {
            return Err(
                "post-detach remove/snapshot touched foreign child or leaked observer capture"
                    .into(),
            );
        }
        // The independently installed second root remains operable throughout.
        click(other, "Restore")?;
        pump_until(|| IsZoomed(other) == 0)?;
        if caption::snapshot(other)?.is_none() || IsWindow(other) == 0 {
            return Err("unexpected-child cleanup affected the other caption adapter".into());
        }
        DestroyWindow(extra);
        checks += 1;

        caption::install(root, {
            let events = events.clone();
            Box::new(move |_| {
                events.fetch_add(1, Ordering::SeqCst);
            })
        })?;
        let before_remove = events.load(Ordering::SeqCst);
        caption::remove(root)?;
        if events.load(Ordering::SeqCst) != before_remove + 1 {
            DestroyWindow(root);
            return Err("remove did not emit one None observer notification".into());
        }
        if caption::snapshot(root)?.is_some() {
            DestroyWindow(root);
            return Err("snapshot remained after remove".into());
        }
        if button_count(root) != 0 {
            DestroyWindow(root);
            return Err("native BUTTON children remained after remove".into());
        }
        let mut restored = zeroed();
        GetWindowRect(root, &mut restored);
        if outer_before.left != restored.left
            || outer_before.top != restored.top
            || outer_before.right != restored.right
            || outer_before.bottom != restored.bottom
        {
            DestroyWindow(root);
            return Err("remove did not restore native outer bounds".into());
        }
        checks += 1;

        // A panicking observer must be contained at the FFI boundary and must
        // not disable the default BUTTON -> BN_CLICKED action chain.
        caption::install(root, Box::new(|_| panic!("observer sentinel panic")))?;
        click(root, "Minimize")?;
        pump_until(|| IsIconic(root) != 0)?;
        ShowWindow(root, SW_RESTORE);
        pump_until(|| IsIconic(root) == 0)?;
        caption::remove(root)?;
        checks += 1;

        // The observer closure's captured Arc is released on explicit remove
        // and on root WM_NCDESTROY, covering both registry cleanup paths.
        let sentinel = Arc::new(());
        let captured = sentinel.clone();
        caption::install(
            root,
            Box::new(move |_| {
                let _keep_alive = &captured;
            }),
        )?;
        if Arc::strong_count(&sentinel) != 2 {
            return Err("observer capture was not retained exactly once".into());
        }
        caption::remove(root)?;
        if Arc::strong_count(&sentinel) != 1 {
            return Err("remove leaked captured observer reference".into());
        }
        checks += 1;
        let (destroy_root, _) = make_root()?;
        let _destroy_guard = RootGuard(destroy_root);
        let destroy_sentinel = Arc::new(());
        let destroy_capture = destroy_sentinel.clone();
        caption::install(
            destroy_root,
            Box::new(move |_| {
                let _keep_alive = &destroy_capture;
            }),
        )?;
        if Arc::strong_count(&destroy_sentinel) != 2 {
            DestroyWindow(destroy_root);
            return Err("destroy observer capture was not retained".into());
        }
        DestroyWindow(destroy_root);
        if Arc::strong_count(&destroy_sentinel) != 1 {
            DestroyWindow(root);
            return Err("root destruction leaked captured observer reference".into());
        }
        checks += 1;

        let theme_events = Arc::new(AtomicUsize::new(0));
        let theme_counter = theme_events.clone();
        caption::install(
            root,
            Box::new(move |_| {
                theme_counter.fetch_add(1, Ordering::SeqCst);
            }),
        )?;
        let actual_theme = caption::snapshot(root)?.unwrap().hover_color;
        for message in [WM_THEMECHANGED, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE] {
            caption::stale_theme_cache(root);
            theme_events.store(0, Ordering::SeqCst);
            SendMessageW(root, message, 0, 0);
            if theme_events.load(Ordering::SeqCst) != 1
                || caption::snapshot(root)?.unwrap().hover_color != actual_theme
            {
                return Err(format!("theme message {message:#x} did not publish the changed hover color exactly once"));
            }
            theme_events.store(0, Ordering::SeqCst);
            SendMessageW(root, message, 0, 0);
            if theme_events.load(Ordering::SeqCst) != 0 {
                return Err(format!(
                    "unchanged theme message {message:#x} published a duplicate state"
                ));
            }
        }
        checks += 1;
        caption::remove(root)?;
        DestroyWindow(root);
        let _ = GetCurrentThreadId();
    }
    Ok(checks)
}

fn main() {
    if std::env::args().any(|arg| arg == "--help") {
        println!("Usage: cargo run --locked --manifest-path src-tauri/Cargo.toml --example windows_caption_native\nRuns bounded actual Win32 caption-adapter checks on owned windows. Does not accept full-app or desktop parity. Non-Windows hosts report an explicit skip.");
        return;
    }
    #[cfg(not(target_os = "windows"))]
    eprintln!("windows_caption_native: skipped; actual Win32 checks require Windows");
    #[cfg(target_os = "windows")]
    {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(45));
            eprintln!("windows_caption_native: watchdog timeout");
            std::process::exit(1);
        });
        match run_checks() {
            Ok(count) => println!("windows_caption_native: {count} checks passed"),
            Err(error) => {
                eprintln!("windows_caption_native: {error}");
                std::process::exit(1);
            }
        }
    }
}
