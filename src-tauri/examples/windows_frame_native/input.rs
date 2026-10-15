//! Physical input checks against the owned caption controller.
use super::{check, Check};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{Manager, WebviewUrl, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP, MOUSEINPUT, VK_LBUTTON,
        },
        WindowsAndMessaging::{
            GetAncestor, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
            SetCursorPos, SetForegroundWindow, WindowFromPoint, GA_ROOT, HTCLOSE, HTMAXBUTTON,
            HTMINBUTTON, WM_NCHITTEST,
        },
    },
};

const TIMEOUT: Duration = Duration::from_secs(5);

struct Fixture {
    window: WebviewWindow,
    hwnd: usize,
    allow_close: Arc<AtomicBool>,
    close_count: Arc<AtomicUsize>,
}

#[derive(Default)]
struct LeftButtonGuard {
    pressed: bool,
}

impl LeftButtonGuard {
    fn press(&mut self, hwnd: HWND, point: POINT, expected_hit: &[isize]) -> Result<Value, String> {
        let report = send_left(hwnd, point, true, expected_hit)?;
        self.pressed = true;
        Ok(report)
    }

    fn release(
        &mut self,
        hwnd: HWND,
        point: POINT,
        expected_hit: &[isize],
    ) -> Result<Value, String> {
        let report = send_left(hwnd, point, false, expected_hit)?;
        self.pressed = false;
        Ok(report)
    }

    fn cleanup(&mut self) -> Result<Value, String> {
        let owned_press = self.pressed;
        let sent = if owned_press {
            send_mouse_button(false)
        } else {
            0
        };
        if owned_press && sent != 1 {
            return Err(format!("cleanup SendInput release returned {sent}/1"));
        }
        poll(
            || Ok(unsafe { GetAsyncKeyState(VK_LBUTTON as i32) >= 0 }),
            |released| *released,
        )?;
        self.pressed = false;
        Ok(json!({"released":owned_press,"sendInputCount":sent,"buttonReadbackUp":true}))
    }
}

impl Drop for LeftButtonGuard {
    fn drop(&mut self) {
        if self.pressed {
            let _ = send_mouse_button(false);
            self.pressed = false;
        }
    }
}

fn fail(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn poll<T>(
    mut read: impl FnMut() -> Result<T, String>,
    mut ready: impl FnMut(&T) -> bool,
) -> Result<T, String> {
    let deadline = Instant::now() + TIMEOUT;
    let mut last_error = None;
    loop {
        match read() {
            Ok(value) if ready(&value) => return Ok(value),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return Err(
                last_error.unwrap_or_else(|| "timed out waiting for native readback".into())
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn snapshot(window: &WebviewWindow) -> Result<Value, String> {
    let value = super::platform::caption_state(window)
        .map_err(|error| format!("caption_state: {error:?}"))?
        .ok_or_else(|| "caption_state returned no snapshot".to_owned())?;
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn create_fixture(owner: &WebviewWindow, data: &Path) -> Result<Fixture, String> {
    let allow_close = Arc::new(AtomicBool::new(false));
    let close_count = Arc::new(AtomicUsize::new(0));
    let event_allow = allow_close.clone();
    let event_count = close_count.clone();
    let window = tauri::WebviewWindowBuilder::new(
        owner.app_handle(),
        format!("captionInputFixture{}", std::process::id()),
        WebviewUrl::External("about:blank".parse().map_err(fail)?),
    )
    .title("Tabby controlled caption input fixture")
    .inner_size(800.0, 580.0)
    .decorations(false)
    .transparent(true)
    .visible(true)
    .always_on_top(true)
    .incognito(true)
    .data_directory(data.to_owned())
    .on_navigation(|url| url.as_str() == "about:blank")
    .build()
    .map_err(fail)?;
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            event_count.fetch_add(1, Ordering::SeqCst);
            if !event_allow.load(Ordering::SeqCst) {
                api.prevent_close();
            }
        }
    });
    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd.0,
        Err(error) => {
            let _ = window.destroy();
            return Err(fail(error));
        }
    };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if hwnd.is_null() || pid != std::process::id() {
        let _ = window.destroy();
        return Err("fixture HWND does not belong to this process".into());
    }
    Ok(Fixture {
        window,
        hwnd: hwnd as usize,
        allow_close,
        close_count,
    })
}

fn restore_fixture(fixture: &Fixture) -> Result<(), String> {
    if unsafe { IsWindow(fixture.hwnd as HWND) } != 0 {
        super::native::destroy(&fixture.window)?;
    }
    poll(
        || Ok(unsafe { IsWindow(fixture.hwnd as HWND) == 0 }),
        |destroyed| *destroyed,
    )?;
    Ok(())
}

fn wait_maximized(window: &WebviewWindow, expected: bool) -> Result<Value, String> {
    poll(
        || {
            let actual = window.is_maximized().map_err(fail)?;
            let mut state = snapshot(window)?;
            state["actualIsMaximized"] = json!(actual);
            Ok(state)
        },
        |value| {
            value["actualIsMaximized"].as_bool() == Some(expected)
                && value["maximized"].as_bool() == Some(expected)
        },
    )
}

fn button_point(state: &Value, name: &str) -> Result<POINT, String> {
    let layout = &state["layout"];
    let clip = &layout["clip"];
    let cx = clip["x"].as_f64().ok_or("layout.clip.x missing")?;
    let cy = clip["y"].as_f64().ok_or("layout.clip.y missing")?;
    let cr = cx + clip["width"].as_f64().ok_or("layout.clip.width missing")?;
    let cb = cy
        + clip["height"]
            .as_f64()
            .ok_or("layout.clip.height missing")?;
    let buttons = layout["buttons"]
        .as_array()
        .ok_or("layout.buttons missing")?;
    for button in buttons {
        if button["button"] != name {
            continue;
        }
        let bounds = &button["bounds"];
        let left = bounds["x"]
            .as_f64()
            .ok_or("button.bounds.x missing")?
            .max(cx);
        let top = bounds["y"]
            .as_f64()
            .ok_or("button.bounds.y missing")?
            .max(cy);
        let right = (bounds["x"].as_f64().ok_or("button.bounds.x missing")?
            + bounds["width"]
                .as_f64()
                .ok_or("button.bounds.width missing")?)
        .min(cr);
        let bottom = (bounds["y"].as_f64().ok_or("button.bounds.y missing")?
            + bounds["height"]
                .as_f64()
                .ok_or("button.bounds.height missing")?)
        .min(cb);
        if right <= left || bottom <= top {
            return Err(format!("{name} has no visible area within layout.clip"));
        }
        return Ok(POINT {
            x: ((left + right) / 2.0).floor() as i32,
            y: ((top + bottom) / 2.0).floor() as i32,
        });
    }
    Err(format!("layout has no {name} button"))
}

fn foreground_and_hit(hwnd: HWND, point: POINT, expected_hit: &[isize]) -> Result<Value, String> {
    if unsafe { GetForegroundWindow() } != hwnd {
        unsafe { SetForegroundWindow(hwnd) };
        poll(
            || Ok(unsafe { GetForegroundWindow() } == hwnd),
            |foreground| *foreground,
        )?;
    }
    let foreground = unsafe { GetForegroundWindow() };
    let mut actual_cursor = POINT { x: 0, y: 0 };
    let cursor_read = unsafe { GetCursorPos(&mut actual_cursor) } != 0;
    let target = unsafe { WindowFromPoint(actual_cursor) };
    let root = unsafe { GetAncestor(target, GA_ROOT) };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(root, &mut pid) };
    let packed = ((actual_cursor.y as u16 as u32) << 16 | actual_cursor.x as u16 as u32) as isize;
    let hit = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(target, WM_NCHITTEST, 0, packed)
    };
    let report = json!({
        "foreground": foreground as usize,
        "pointTarget": target as usize,
        "cursorRead": cursor_read,
        "actualCursor": [actual_cursor.x, actual_cursor.y],
        "expectedCursor": [point.x, point.y],
        "pointRoot": root as usize,
        "pointRootPid": pid,
        "expectedHwnd": hwnd as usize,
        "hitTest": hit
    });
    if foreground != hwnd
        || !cursor_read
        || actual_cursor.x != point.x
        || actual_cursor.y != point.y
        || target.is_null()
        || root != hwnd
        || pid != std::process::id()
    {
        return Err(format!(
            "foreground or point ownership check failed: {report}"
        ));
    }
    if !expected_hit.contains(&(hit as isize)) {
        return Err(format!("unexpected WM_NCHITTEST value: {report}"));
    }
    Ok(report)
}

fn send_left(
    hwnd: HWND,
    point: POINT,
    down: bool,
    expected_hit: &[isize],
) -> Result<Value, String> {
    let ownership = foreground_and_hit(hwnd, point, expected_hit)?;
    let sent = send_mouse_button(down);
    if sent != 1 {
        return Err(format!(
            "SendInput returned {sent}/1; ownership: {ownership}"
        ));
    }
    Ok(json!({"sendInputCount": sent, "ownership": ownership}))
}

fn send_mouse_button(down: bool) -> u32 {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_MOUSE;
    input.Anonymous = INPUT_0 {
        mi: MOUSEINPUT {
            dx: 0,
            dy: 0,
            mouseData: 0,
            dwFlags: if down {
                MOUSEEVENTF_LEFTDOWN
            } else {
                MOUSEEVENTF_LEFTUP
            },
            time: 0,
            dwExtraInfo: 0,
        },
    };
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) }
}

fn move_to(hwnd: HWND, point: POINT) -> Result<(), String> {
    if unsafe { SetCursorPos(point.x, point.y) } == 0 {
        return Err(format!(
            "SetCursorPos failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let target = unsafe { WindowFromPoint(point) };
    if target.is_null() || unsafe { GetAncestor(target, GA_ROOT) } != hwnd {
        return Err("cursor move did not land on the owned fixture client".into());
    }
    Ok(())
}

fn click(
    fixture: &Fixture,
    button: &mut LeftButtonGuard,
    name: &str,
) -> Result<(Value, Value), String> {
    let state = snapshot(&fixture.window)?;
    let point = client_to_screen(fixture.hwnd as HWND, button_point(&state, name)?)?;
    move_to(fixture.hwnd as HWND, point)?;
    let hovered = poll(
        || snapshot(&fixture.window),
        |value| value["pointer"]["hovered"] == name,
    )?;
    let hittest = match name {
        "minimize" => &[HTMINBUTTON as isize][..],
        "maximize" => &[HTMAXBUTTON as isize][..],
        "close" => &[HTCLOSE as isize][..],
        _ => return Err(format!("unsupported button {name}")),
    };
    let before = hovered;
    if !before["pointer"]["pressed"].is_null() {
        return Err(format!("caption already has a pressed button: {before}"));
    }
    let down = button.press(fixture.hwnd as HWND, point, hittest)?;
    let pressed = poll(
        || snapshot(&fixture.window),
        |value| value["pointer"]["pressed"] == name,
    )?;
    let up = button.release(fixture.hwnd as HWND, point, hittest)?;
    let released = poll(
        || snapshot(&fixture.window),
        |value| value["pointer"]["pressed"].is_null(),
    )?;
    Ok((
        json!({"before":before,"pressed":pressed,"released":released,"down":down,"up":up}),
        json!({"screenPoint":[point.x,point.y]}),
    ))
}

fn client_to_screen(hwnd: HWND, mut point: POINT) -> Result<POINT, String> {
    if unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut point) } == 0 {
        return Err(format!(
            "ClientToScreen failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(point)
}

fn run_checks(
    owner: &WebviewWindow,
    other: &WebviewWindow,
    data: &Path,
    output: &Path,
) -> Vec<Check> {
    let mut checks = Vec::new();
    let initial_owner = super::native::snapshot(owner);
    let initial_other = super::native::snapshot(other);
    let mut saved_cursor = POINT { x: 0, y: 0 };
    let cursor_saved = unsafe { GetCursorPos(&mut saved_cursor) } != 0;
    if !cursor_saved {
        checks.push(check(
            "caption-physical-input-preconditions",
            false,
            json!({"error":format!("GetCursorPos failed: {}",std::io::Error::last_os_error())}),
        ));
        return checks;
    }
    if unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0 {
        checks.push(check(
            "caption-physical-input-preconditions",
            false,
            json!({"error":"left mouse button was already pressed before the fixture input"}),
        ));
        return checks;
    }
    let fixture = match create_fixture(owner, data) {
        Ok(fixture) => fixture,
        Err(error) => {
            checks.push(check(
                "caption-fixture-created",
                false,
                json!({"error":error}),
            ));
            return checks;
        }
    };
    let mut armed = false;
    let mut left_button = LeftButtonGuard::default();
    let work = (|| -> Result<(), String> {
        super::renderer::wait(&fixture.window)?;
        let before =
            super::native::capture(&fixture.window, &output.join("caption-input-before.png"))
                .map_err(fail)?;
        let prepared = super::platform::prepare_caption_controls(&fixture.window)
            .map_err(|error| format!("prepare_caption_controls: {error:?}"));
        if let Err(error) = prepared {
            checks.push(check(
                "caption-preparation-and-visible-layout",
                false,
                json!({"error":error}),
            ));
            return Err(error);
        }
        super::platform::set_caption_frame(&fixture.window, "thin")
            .map_err(|error| format!("set_caption_frame(thin): {error:?}"))?;
        let after_prepare = super::native::capture(
            &fixture.window,
            &output.join("caption-input-after-prepare.png"),
        )
        .map_err(fail)?;
        let pixels_unchanged = before.width == after_prepare.width
            && before.height == after_prepare.height
            && before.rgba == after_prepare.rgba;
        if !pixels_unchanged {
            return Err("native capture changed across caption preparation".into());
        }
        let visible = poll(
            || snapshot(&fixture.window),
            |value| value["visible"] == true,
        )?;
        checks.push(check("caption-preparation-and-visible-layout", true, json!({"visibleSnapshot":visible,"width":before.width,"height":before.height,"rgbaBytes":before.rgba.len(),"captureUnchangedAfterPreparation":pixels_unchanged})));
        let max_result = click(&fixture, &mut left_button, "maximize")?;
        let maximized = wait_maximized(&fixture.window, true)?;
        if maximized["maximized"] != true
            || maximized["revision"].as_u64() <= visible["revision"].as_u64()
        {
            return Err(format!(
                "maximize did not advance state/revision: {maximized}"
            ));
        }
        let restore_result = click(&fixture, &mut left_button, "maximize")?;
        let restored = wait_maximized(&fixture.window, false)?;
        if restored["revision"].as_u64() <= maximized["revision"].as_u64() {
            return Err(format!("restore did not advance revision: {restored}"));
        }
        checks.push(check("sendinput-maximize-restore-readback", true, json!({"maximize":max_result,"maximized":maximized,"restore":restore_result,"restored":restored})));

        let minimize_result = click(&fixture, &mut left_button, "minimize")?;
        let minimized = poll(
            || fixture.window.is_minimized().map_err(fail),
            |value| *value,
        )?;
        fixture.window.unminimize().map_err(fail)?;
        fixture.window.set_focus().map_err(fail)?;
        let unminimized = poll(
            || fixture.window.is_minimized().map_err(fail),
            |value| !*value,
        )?;
        let refocused = poll(
            || snapshot(&fixture.window),
            |value| value["visible"] == true && value["active"] == true,
        )?;
        checks.push(check("sendinput-minimize-and-api-restore", true, json!({"minimize":minimize_result,"minimized":minimized,"unminimized":unminimized,"refocused":refocused})));

        let close_state = snapshot(&fixture.window)?;
        let close = client_to_screen(fixture.hwnd as HWND, button_point(&close_state, "close")?)?;
        move_to(fixture.hwnd as HWND, close)?;
        let body = client_to_screen(fixture.hwnd as HWND, POINT { x: 40, y: 120 })?;
        let down = left_button.press(fixture.hwnd as HWND, close, &[HTCLOSE as isize])?;
        let pressed = poll(
            || snapshot(&fixture.window),
            |value| value["pointer"]["pressed"] == "close",
        )?;
        move_to(fixture.hwnd as HWND, body)?;
        let up = left_button.release(fixture.hwnd as HWND, body, &[1])?;
        let cancelled = poll(
            || snapshot(&fixture.window),
            |value| value["pointer"]["pressed"].is_null(),
        )?;
        super::native::on_main(&fixture.window, |_| Ok(()))?;
        let quiet_deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < quiet_deadline {
            if fixture.close_count.load(Ordering::SeqCst) != 0 {
                return Err("dragging out of Close generated CloseRequested".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let no_close = fixture.close_count.load(Ordering::SeqCst);
        checks.push(check("sendinput-close-drag-cancels", no_close == 0, json!({"down":down,"pressed":pressed,"up":up,"afterDrag":cancelled,"closeRequestedCount":no_close,"quietPeriodMs":250,"nativeBarrier":true})));

        let first_close = click(&fixture, &mut left_button, "close")?;
        let one_close = poll(
            || Ok(fixture.close_count.load(Ordering::SeqCst)),
            |count| *count == 1,
        )?;
        let survives = unsafe { IsWindow(fixture.hwnd as HWND) != 0 };
        if !survives {
            return Err("prevent_close fixture was destroyed after first close".into());
        }
        checks.push(check("sendinput-first-close-prevented", true, json!({"firstClick":first_close,"firstCloseRequestedCount":one_close,"survivedPreventClose":survives})));
        fixture.allow_close.store(true, Ordering::SeqCst);
        let last = snapshot(&fixture.window)?;
        let last_point = client_to_screen(fixture.hwnd as HWND, button_point(&last, "close")?)?;
        move_to(fixture.hwnd as HWND, last_point)?;
        if !last["pointer"]["pressed"].is_null() {
            return Err("caption still reports pressed button before final close".into());
        }
        let down = left_button.press(fixture.hwnd as HWND, last_point, &[HTCLOSE as isize])?;
        let pressed = poll(
            || snapshot(&fixture.window),
            |value| value["pointer"]["pressed"] == "close",
        )?;
        let up = left_button.release(fixture.hwnd as HWND, last_point, &[HTCLOSE as isize])?;
        let destroyed = poll(
            || Ok(unsafe { IsWindow(fixture.hwnd as HWND) == 0 }),
            |value| *value,
        )?;
        let two_close = poll(
            || Ok(fixture.close_count.load(Ordering::SeqCst)),
            |count| *count == 2,
        )?;
        checks.push(check("sendinput-second-close-allowed", destroyed && two_close == 2, json!({"down":down,"pressed":pressed,"up":up,"destroyed":destroyed,"closeRequestedCount":two_close})));
        armed = true;
        Ok(())
    })();
    if let Err(error) = work {
        checks.push(check(
            "caption-physical-input-sequence",
            false,
            json!({"error":error,"sequenceStopped":true}),
        ));
    }
    let button_cleanup = left_button.cleanup();
    let cursor_restore = if cursor_saved {
        unsafe { SetCursorPos(saved_cursor.x, saved_cursor.y) != 0 }
    } else {
        false
    };
    let cursor_readback = if cursor_restore {
        let mut now = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut now) } != 0)
            && now.x == saved_cursor.x
            && now.y == saved_cursor.y
    } else {
        false
    };
    let destroyed = restore_fixture(&fixture).is_ok();
    let released = button_cleanup.is_ok();
    checks.push(check("fixture-cleanup-and-cursor-restore", destroyed && cursor_restore && cursor_readback && released, json!({"destroyed":destroyed,"leftButtonCleanup":button_cleanup,"cursorSaved":cursor_saved,"cursorRestored":cursor_restore,"cursorReadbackMatches":cursor_readback,"sequenceReachedFinalClose":armed})));
    let final_owner = super::native::snapshot(owner);
    let final_other = super::native::snapshot(other);
    checks.push(check("other-window-and-owner-isolation", matches!((&initial_owner,&final_owner), (Ok(before),Ok(after)) if before == after) && matches!((&initial_other,&final_other), (Ok(before),Ok(after)) if before == after), json!({"initialOwner":initial_owner,"finalOwner":final_owner,"initialOther":initial_other,"finalOther":final_other})));
    checks
}

pub fn run(
    owner: &tauri::WebviewWindow,
    other: &tauri::WebviewWindow,
    data: &Path,
    output: &Path,
) -> Vec<Check> {
    run_checks(owner, other, data, output)
}
