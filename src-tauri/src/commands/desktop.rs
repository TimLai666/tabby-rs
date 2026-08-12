use std::sync::{Arc, Mutex};
use std::{fs, path::PathBuf};

use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{
    desktop::{
        capabilities, docked_bounds, normalize_accelerator, screen_info, ColorScheme,
        DesktopNotification, DockingOptions, GlobalHotkeyEvent, GlobalHotkeyRegistration,
        OpenDialogOptions, SaveDialogOptions, WindowBounds, WindowStatePatch, WindowStateSnapshot,
    },
    error::AppError,
    state::AppState,
};

#[derive(Debug, Default, serde::Deserialize)]
pub struct NewWindowRequest {
    #[serde(default)]
    pub launch: Option<crate::launch::LaunchContext>,
}

fn io_error(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

fn window_state(window: &tauri::WebviewWindow) -> Result<WindowStateSnapshot, AppError> {
    let position = window.outer_position().map_err(io_error)?;
    let size = window.outer_size().map_err(io_error)?;
    Ok(WindowStateSnapshot {
        visible: window.is_visible().map_err(io_error)?,
        always_on_top: window.is_always_on_top().map_err(io_error)?,
        fullscreen: window.is_fullscreen().map_err(io_error)?,
        maximized: window.is_maximized().map_err(io_error)?,
        minimized: window.is_minimized().map_err(io_error)?,
        focused: window.is_focused().map_err(io_error)?,
        bounds: WindowBounds {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        scale_factor: window.scale_factor().map_err(io_error)?,
        capabilities: capabilities(),
    })
}

#[tauri::command]
pub fn window_new(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    request: NewWindowRequest,
) -> Result<(), AppError> {
    let label = format!("window-{}", state.next_window_id());
    let launch = Arc::new(Mutex::new(request.launch));
    let launch_for_page_load = Arc::clone(&launch);
    let window =
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
            .title("Tabby RS")
            .inner_size(1200.0, 800.0)
            .min_inner_size(640.0, 480.0)
            .on_page_load(move |window, payload| {
                if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                    return;
                }
                let context = launch_for_page_load
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if let Some(context) = context {
                    let _ = window.emit("app:launch", context);
                }
            })
            .build()
            .map_err(io_error)?;
    crate::register_desktop_window_events(&window);
    Ok(())
}

#[tauri::command]
pub fn window_get_state(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<WindowStateSnapshot, AppError> {
    window_state(&window)
}

#[tauri::command]
pub fn window_apply_state(
    window: tauri::WebviewWindow,
    request: WindowStatePatch,
) -> Result<(), AppError> {
    if let Some(title) = request.title.as_deref() {
        window.set_title(title).map_err(io_error)?;
    }
    if let Some(value) = request.always_on_top {
        window.set_always_on_top(value).map_err(io_error)?;
    }
    if let Some(value) = request.fullscreen {
        window.set_fullscreen(value).map_err(io_error)?;
    }
    if let Some(value) = request.maximized {
        if value {
            window.maximize().map_err(io_error)?;
        } else {
            window.unmaximize().map_err(io_error)?;
        }
    }
    if let Some(bounds) = request.bounds {
        window
            .set_size(PhysicalSize::new(bounds.width, bounds.height))
            .map_err(io_error)?;
        if capabilities().absolute_positioning {
            window
                .set_position(PhysicalPosition::new(bounds.x, bounds.y))
                .map_err(io_error)?;
        }
    }
    if let Some(value) = request.progress {
        let state = match value {
            Some(value) => ProgressBarState {
                status: Some(ProgressBarStatus::Normal),
                progress: Some((value.clamp(0.0, 1.0) * 100.0).round() as u64),
            },
            None => ProgressBarState {
                status: Some(ProgressBarStatus::None),
                progress: None,
            },
        };
        window.set_progress_bar(state).map_err(io_error)?;
    }
    if let Some(scheme) = request.color_scheme {
        let theme = match scheme {
            ColorScheme::System => None,
            ColorScheme::Light => Some(tauri::Theme::Light),
            ColorScheme::Dark => Some(tauri::Theme::Dark),
        };
        window.set_theme(theme).map_err(io_error)?;
    }

    if let Some(opacity) = request.opacity {
        crate::platform::set_window_opacity(&window, opacity)?;
    }
    if let Some(vibrancy) = request.vibrancy {
        crate::platform::set_window_vibrancy(&window, &vibrancy)?;
    }
    if let Some(visible) = request.visible {
        if visible {
            window.show().map_err(io_error)?;
        } else {
            window.hide().map_err(io_error)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn window_reload(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    window.reload().map_err(io_error)
}

#[tauri::command]
pub fn window_minimize(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    window.minimize().map_err(io_error)
}

#[tauri::command]
pub fn window_toggle_maximize(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    if window.is_maximized().map_err(io_error)? {
        window.unmaximize().map_err(io_error)
    } else {
        window.maximize().map_err(io_error)
    }
}

#[tauri::command]
pub fn window_close(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    window.close().map_err(io_error)
}

#[tauri::command]
pub fn window_bring_to_front(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    window.show().map_err(io_error)?;
    window.unminimize().map_err(io_error)?;
    window.set_focus().map_err(io_error)
}

#[tauri::command]
pub fn window_open_devtools(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    #[cfg(debug_assertions)]
    window.open_devtools();
    Ok(())
}

#[tauri::command]
pub fn window_list_screens(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<Vec<crate::desktop::ScreenInfo>, AppError> {
    let monitors = window.available_monitors().map_err(io_error)?;
    let primary = window.primary_monitor().map_err(io_error)?;
    Ok(screen_info(&monitors, primary.as_ref()))
}

#[tauri::command]
pub fn window_set_docking(
    window: tauri::WebviewWindow,
    request: DockingOptions,
) -> Result<WindowStateSnapshot, AppError> {
    if request.side == "off" {
        window.set_always_on_top(false).map_err(io_error)?;
        return window_state(&window);
    }
    if !capabilities().docking {
        return Err(AppError::Unsupported(
            "absolute window docking is unavailable in this display session".into(),
        ));
    }

    let monitors = window.available_monitors().map_err(io_error)?;
    let primary = window.primary_monitor().map_err(io_error)?;
    let screens = screen_info(&monitors, primary.as_ref());
    let selected = request
        .screen_id
        .and_then(|id| screens.iter().find(|screen| screen.id == id))
        .or_else(|| screens.iter().find(|screen| screen.primary))
        .or_else(|| screens.first())
        .ok_or_else(|| AppError::NotFound("display".into()))?;
    let bounds = docked_bounds(&request, selected.work_area)?;

    window
        .set_size(PhysicalSize::new(bounds.width, bounds.height))
        .map_err(io_error)?;
    window
        .set_position(PhysicalPosition::new(bounds.x, bounds.y))
        .map_err(io_error)?;
    window
        .set_always_on_top(request.always_on_top)
        .map_err(io_error)?;
    window_state(&window)
}

#[tauri::command]
pub fn window_toggle_quake(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<bool, AppError> {
    let hide = window.is_visible().map_err(io_error)? && window.is_focused().map_err(io_error)?;
    if hide {
        window.hide().map_err(io_error)?;
        Ok(false)
    } else {
        window.show().map_err(io_error)?;
        window.unminimize().map_err(io_error)?;
        window.set_focus().map_err(io_error)?;
        Ok(true)
    }
}

#[tauri::command]
pub fn hotkey_replace(
    app: tauri::AppHandle,
    request: GlobalHotkeyRegistration,
) -> Result<Vec<String>, AppError> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        let normalized = request
            .accelerators
            .iter()
            .map(|shortcut| normalize_accelerator(shortcut))
            .collect::<Result<Vec<_>, _>>()?;
        let shortcuts = normalized
            .iter()
            .map(|shortcut| {
                shortcut.parse::<Shortcut>().map_err(|error| {
                    AppError::InvalidArgument(format!(
                        "invalid global shortcut {shortcut}: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        app.global_shortcut().unregister_all().map_err(io_error)?;
        if shortcuts.is_empty() {
            return Ok(normalized);
        }

        let id = request.id.clone();
        app.global_shortcut()
            .on_shortcuts(shortcuts, move |app, shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                let accelerator = shortcut.to_string();
                if id == "toggle-window" {
                    if let Some(window) = app.get_webview_window("main") {
                        let hide = window.is_visible().unwrap_or(false)
                            && window.is_focused().unwrap_or(false);
                        if hide {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                }
                let _ = app.emit(
                    "desktop:hotkey",
                    GlobalHotkeyEvent {
                        id: id.clone(),
                        accelerator,
                    },
                );
            })
            .map_err(io_error)?;
        return Ok(normalized);
    }

    #[allow(unreachable_code)]
    Err(AppError::Unsupported(
        "global shortcuts are unavailable".into(),
    ))
}

#[tauri::command]
pub fn clipboard_read_text(
    app: tauri::AppHandle,
    _request: serde_json::Value,
) -> Result<String, AppError> {
    app.clipboard().read_text().map_err(io_error)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardWriteRequest {
    pub text: String,
}

#[tauri::command]
pub fn clipboard_write_text(
    app: tauri::AppHandle,
    request: ClipboardWriteRequest,
) -> Result<(), AppError> {
    app.clipboard().write_text(request.text).map_err(io_error)
}

#[tauri::command]
pub async fn dialog_open(
    app: tauri::AppHandle,
    request: OpenDialogOptions,
) -> Result<Vec<String>, AppError> {
    let mut builder = app.dialog().file();
    if let Some(title) = request.title.as_deref() {
        builder = builder.set_title(title);
    }
    let result = if request.directory {
        if request.multiple {
            builder
                .blocking_pick_folders()
                .unwrap_or_default()
                .into_iter()
                .map(file_path_to_string)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            builder
                .blocking_pick_folder()
                .map(file_path_to_string)
                .transpose()?
                .into_iter()
                .collect()
        }
    } else if request.multiple {
        builder
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(file_path_to_string)
            .collect::<Result<Vec<_>, _>>()?
    } else {
        builder
            .blocking_pick_file()
            .map(file_path_to_string)
            .transpose()?
            .into_iter()
            .collect()
    };
    Ok(result)
}

#[tauri::command]
pub async fn dialog_save(
    app: tauri::AppHandle,
    request: SaveDialogOptions,
) -> Result<Option<String>, AppError> {
    let mut builder = app.dialog().file();
    if let Some(title) = request.title.as_deref() {
        builder = builder.set_title(title);
    }
    if let Some(file_name) = request.file_name.as_deref() {
        builder = builder.set_file_name(file_name);
    }
    builder
        .blocking_save_file()
        .map(file_path_to_string)
        .transpose()
}

#[tauri::command]
pub fn notification_show(
    app: tauri::AppHandle,
    request: DesktopNotification,
) -> Result<(), AppError> {
    let mut builder = app.notification().builder().title(request.title);
    if let Some(body) = request.body {
        builder = builder.body(body);
    }
    builder.show().map_err(io_error)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenExternalRequest {
    pub url: String,
}

#[tauri::command]
pub fn desktop_open_external(
    app: tauri::AppHandle,
    request: OpenExternalRequest,
) -> Result<(), AppError> {
    let parsed = url::Url::parse(&request.url)
        .map_err(|_| AppError::InvalidArgument("invalid external URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(AppError::InvalidArgument(
            "unsupported external URL scheme".into(),
        ));
    }
    app.opener()
        .open_url(parsed.to_string(), None::<&str>)
        .map_err(io_error)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathRequest {
    pub path: String,
}

#[tauri::command]
pub fn desktop_reveal_path(app: tauri::AppHandle, request: PathRequest) -> Result<(), AppError> {
    app.opener()
        .reveal_item_in_dir(PathBuf::from(request.path))
        .map_err(io_error)
}

#[tauri::command]
pub fn desktop_open_path(app: tauri::AppHandle, request: PathRequest) -> Result<(), AppError> {
    app.opener()
        .open_path(request.path, None::<&str>)
        .map_err(io_error)
}

#[tauri::command]
pub async fn desktop_read_file(
    _app: tauri::AppHandle,
    request: PathRequest,
) -> Result<Vec<u8>, AppError> {
    let path = PathBuf::from(request.path);
    tokio_read(path).await
}

async fn tokio_read(path: PathBuf) -> Result<Vec<u8>, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::read(path))
        .await
        .map_err(|error| AppError::Io(error.to_string()))?
        .map_err(AppError::from)
}

fn file_path_to_string(path: tauri_plugin_dialog::FilePath) -> Result<String, AppError> {
    path.into_path()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(io_error)
}
