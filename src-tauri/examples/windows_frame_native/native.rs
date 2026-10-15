use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    path::Path,
    ptr::{null, null_mut},
    sync::mpsc::sync_channel,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{Dwm::DwmFlush, Gdi::*},
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

pub fn on_main<T: Send + 'static>(
    window: &tauri::WebviewWindow,
    task: impl FnOnce(tauri::WebviewWindow) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = sync_channel(1);
    let owned = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = tx.send(task(owned));
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|e| format!("native main-thread wait: {e}"))?
}

fn owned_hwnd(window: &tauri::WebviewWindow) -> Result<HWND, String> {
    let handle = window.hwnd().map_err(|e| e.to_string())?.0;
    let mut process = 0;
    unsafe {
        GetWindowThreadProcessId(handle, &mut process);
    }
    if handle.is_null() || process != std::process::id() {
        return Err("native handle is not owned by this probe process".into());
    }
    Ok(handle)
}

pub fn snapshot(window: &tauri::WebviewWindow) -> Result<Value, String> {
    on_main(window, |window| {
        let hwnd = owned_hwnd(&window)?;
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(
            json!({"label":window.label(),"style":unsafe{GetWindowLongPtrW(hwnd,GWL_STYLE)} as u32,"bounds":[rect.left,rect.top,rect.right,rect.bottom],"visible":unsafe{IsWindowVisible(hwnd)}!=0,"maximized":unsafe{IsZoomed(hwnd)}!=0,"dpi":unsafe{GetDpiForWindow(hwnd)}}),
        )
    })
}

pub fn destroy(window: &tauri::WebviewWindow) -> Result<(), String> {
    let handle = on_main(window, |window| Ok(owned_hwnd(&window)? as usize))?;
    window.destroy().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while unsafe { IsWindow(handle as HWND) } != 0 {
        if std::time::Instant::now() >= deadline {
            return Err("owned HWND survived destroy".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]])
    }
}

pub fn capture(window: &tauri::WebviewWindow, path: &Path) -> Result<Raster, String> {
    let hwnd = owned_hwnd(window)?;
    capture_owned(hwnd as usize, std::process::id(), path, false)
}

pub fn capture_owned(
    handle: usize,
    process: u32,
    path: &Path,
    outer: bool,
) -> Result<Raster, String> {
    let hwnd = handle as HWND;
    let mut actual_process = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut actual_process);
    }
    if hwnd.is_null() || actual_process != process {
        return Err("capture handle does not belong to the owned process".into());
    }
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    let mut origin = POINT { x: 0, y: 0 };
    if outer {
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        origin.x = rect.left;
        origin.y = rect.top;
    } else if unsafe { GetClientRect(hwnd, &mut rect) } == 0
        || unsafe { ClientToScreen(hwnd, &mut origin) } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        return Err("invalid or excessive client capture dimensions".into());
    }
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return Err("owned capture window is not visible".into());
    }
    let flush = unsafe { DwmFlush() };
    if flush < 0 {
        return Err(format!("DwmFlush failed: {flush}"));
    }
    let mut bytes = vec![0u8; width as usize * height as usize * 4];
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return Err("GetDC failed".into());
        }
        let memory = CreateCompatibleDC(screen);
        if memory.is_null() {
            ReleaseDC(null_mut(), screen);
            return Err("CreateCompatibleDC failed".into());
        }
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if bitmap.is_null() {
            DeleteDC(memory);
            ReleaseDC(null_mut(), screen);
            return Err("CreateCompatibleBitmap failed".into());
        }
        let previous = SelectObject(memory, bitmap);
        if previous.is_null() || previous as isize == -1 {
            let error = std::io::Error::last_os_error();
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(null_mut(), screen);
            return Err(format!("SelectObject failed: {error}"));
        }
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            screen,
            origin.x,
            origin.y,
            SRCCOPY | CAPTUREBLT,
        );
        let copy_error = std::io::Error::last_os_error();
        SelectObject(memory, previous);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let rows = if copied != 0 {
            GetDIBits(
                screen,
                bitmap,
                0,
                height as u32,
                bytes.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        let read_error = std::io::Error::last_os_error();
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(null_mut(), screen);
        if copied == 0 {
            return Err(format!("BitBlt failed: {copy_error}"));
        }
        if rows != height {
            return Err(format!(
                "GetDIBits returned {rows}/{height} rows: {read_error}"
            ));
        }
    }
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(Raster {
        width: width as u32,
        height: height as u32,
        rgba: bytes,
    })
}

unsafe extern "system" fn no_paint(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTMAXBUTTON as isize,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut paint);
            EndPaint(hwnd, &paint);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

struct Overlay {
    hwnd: usize,
    instance: usize,
    class: Vec<u16>,
    custom: bool,
    x: i32,
    width: i32,
    height: i32,
}

pub fn overlay_experiment(window: &tauri::WebviewWindow, output: &Path) -> Result<Value, String> {
    let baseline = capture(window, &output.join("wry-baseline.png"))?;
    let mut results = Vec::new();
    for mode in ["stock-static", "no-paint", "no-paint-transparent"] {
        let created = on_main(window, move |window| {
            let parent = owned_hwnd(&window)?;
            let mut rect: RECT = unsafe { std::mem::zeroed() };
            if unsafe { GetClientRect(parent, &mut rect) } == 0 {
                return Err("overlay GetClientRect failed".into());
            }
            let dpi = unsafe { GetDpiForWindow(parent) } as f64 / 96.0;
            let width = (46.0 * dpi).round() as i32;
            let height = (32.0 * dpi).round() as i32;
            let x = rect.right - width * 2;
            let custom = mode != "stock-static";
            let class = if custom {
                format!("TabbyRsWryFrameProbe{}", std::process::id())
            } else {
                "STATIC".into()
            }
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
            let instance =
                unsafe { GetWindowLongPtrW(parent, GWLP_HINSTANCE) } as *mut std::ffi::c_void;
            if custom {
                let mut definition: WNDCLASSW = unsafe { std::mem::zeroed() };
                definition.lpfnWndProc = Some(no_paint);
                definition.hInstance = instance;
                definition.lpszClassName = class.as_ptr();
                if unsafe { RegisterClassW(&definition) } == 0 {
                    return Err(format!(
                        "RegisterClassW: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
            let extra = if mode == "no-paint-transparent" {
                WS_EX_TRANSPARENT
            } else {
                0
            };
            let child = unsafe {
                CreateWindowExW(
                    extra,
                    class.as_ptr(),
                    null(),
                    WS_CHILD | WS_VISIBLE,
                    x,
                    0,
                    width,
                    height,
                    parent,
                    null_mut(),
                    instance,
                    null(),
                )
            };
            if child.is_null() {
                let error = std::io::Error::last_os_error();
                if custom {
                    unsafe {
                        UnregisterClassW(class.as_ptr(), instance);
                    }
                }
                return Err(format!("CreateWindowExW: {error}"));
            }
            Ok(Overlay {
                hwnd: child as usize,
                instance: instance as usize,
                class,
                custom,
                x,
                width,
                height,
            })
        });
        let overlay = match created {
            Ok(value) => value,
            Err(error) => {
                results.push(json!({"mode":mode,"supported":false,"error":error}));
                continue;
            }
        };
        std::thread::sleep(Duration::from_millis(100));
        let readback = on_main(window, {
            let child = overlay.hwnd;
            move |_| {
                let child = child as HWND;
                let mut point = POINT { x: 10, y: 10 };
                if unsafe { ClientToScreen(child, &mut point) } == 0 {
                    return Err("child ClientToScreen failed".into());
                }
                let hit = unsafe { WindowFromPoint(point) } == child;
                let packed = ((point.y as u16 as u32) << 16 | point.x as u16 as u32) as isize;
                let native_hit = unsafe { SendMessageW(child, WM_NCHITTEST, 0, packed) };
                Ok(json!({"actualMouseTarget":hit,"nativeHitTest":native_hit}))
            }
        });
        let image = capture(window, &output.join(format!("wry-{mode}.png")));
        let cleanup = on_main(window, {
            let child = overlay.hwnd;
            let instance = overlay.instance;
            let class = overlay.class.clone();
            let custom = overlay.custom;
            move |_| {
                let child = child as HWND;
                let destroyed =
                    unsafe { DestroyWindow(child) } != 0 && unsafe { IsWindow(child) } == 0;
                let unregistered =
                    !custom || unsafe { UnregisterClassW(class.as_ptr(), instance as _) } != 0;
                if !destroyed || !unregistered {
                    return Err(format!(
                        "destroyed={destroyed}, unregistered={unregistered}"
                    ));
                }
                Ok(())
            }
        });
        let mut mismatches = 0;
        let mut valid_pixels = false;
        if let Ok(image) = &image {
            if image.width == baseline.width && image.height == baseline.height {
                valid_pixels = overlay.x >= 0
                    && overlay.x + overlay.width <= image.width as i32
                    && overlay.height <= image.height as i32;
                if valid_pixels {
                    for y in 0..overlay.height as u32 {
                        for x in overlay.x as u32..(overlay.x + overlay.width) as u32 {
                            if image.pixel(x, y) != baseline.pixel(x, y) {
                                mismatches += 1;
                            }
                        }
                    }
                }
            }
        }
        let pixels_unchanged = valid_pixels && mismatches == 0;
        let supported = pixels_unchanged
            && readback.as_ref().is_ok_and(|value| {
                value["actualMouseTarget"] == true && value["nativeHitTest"] == HTMAXBUTTON
            })
            && cleanup.is_ok();
        results.push(json!({"mode":mode,"supported":supported,"pixelsUnchanged":pixels_unchanged,"mismatchedPixels":mismatches,"nativeReadback":readback,"captureError":image.err(),"cleanup":cleanup,"rectangle":{"x":overlay.x,"y":0,"width":overlay.width,"height":overlay.height}}));
    }
    Ok(
        json!({"baselineWidth":baseline.width,"baselineHeight":baseline.height,"baselineBodyPixel":baseline.pixel(20,100),"modes":results}),
    )
}
