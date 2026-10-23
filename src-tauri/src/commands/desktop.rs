use std::{fs, path::PathBuf};

use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio::process::Command;

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

fn set_bounds_size(window: &tauri::WebviewWindow, size: PhysicalSize<u32>) -> Result<(), AppError> {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    return crate::platform::set_window_outer_size(window, size);

    #[cfg(target_os = "linux")]
    return crate::platform::window_completion_linux::set_size(window, size);

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    window.set_size(size).map_err(io_error)
}

fn set_bounds_position(
    window: &tauri::WebviewWindow,
    position: PhysicalPosition<i32>,
) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    return crate::platform::set_window_position(window, position);

    #[cfg(not(target_os = "linux"))]
    window.set_position(position).map_err(io_error)
}

fn window_capabilities(
    window: &tauri::WebviewWindow,
) -> Result<crate::desktop::WindowCapabilities, AppError> {
    #[cfg(target_os = "linux")]
    {
        let mut capabilities = capabilities();
        let positioning = crate::platform::supports_absolute_positioning(window)?;
        capabilities.absolute_positioning = positioning;
        capabilities.docking = positioning;
        Ok(capabilities)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Ok(capabilities())
    }
}

fn window_state(window: &tauri::WebviewWindow) -> Result<WindowStateSnapshot, AppError> {
    #[cfg(target_os = "linux")]
    let (position, size) = crate::platform::window_bounds(window)?;
    #[cfg(not(target_os = "linux"))]
    let (position, size) = (
        window.outer_position().map_err(io_error)?,
        window.outer_size().map_err(io_error)?,
    );
    Ok(WindowStateSnapshot {
        visible: window.is_visible().map_err(io_error)?,
        always_on_top: crate::platform::window_is_always_on_top(window)?,
        fullscreen: window.is_fullscreen().map_err(io_error)?,
        maximized: crate::platform::window_is_maximized(window)?,
        minimized: window.is_minimized().map_err(io_error)?,
        focused: window.is_focused().map_err(io_error)?,
        bounds: WindowBounds {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        scale_factor: window.scale_factor().map_err(io_error)?,
        capabilities: window_capabilities(window)?,
        caption_controls: crate::platform::window_caption_controls(window)?,
    })
}

#[tauri::command]
pub async fn window_new(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    request: NewWindowRequest,
) -> Result<(), AppError> {
    create_window(&app, &state, request).map(|_| ())
}

pub(crate) fn create_window(
    app: &tauri::AppHandle,
    state: &AppState,
    request: NewWindowRequest,
) -> Result<tauri::WebviewWindow, AppError> {
    let label = format!("window-{}", state.next_window_id());
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    let geometry = app.state::<crate::window_geometry::initialization::Initializations>();
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    let pending_geometry = geometry.begin(&label).map_err(AppError::Io)?;
    if let Some(context) = request.launch {
        state.launches().push(&label, context.for_new_window());
    }
    let builder =
        tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App("index.html".into()))
            .title("Tabby RS")
            .inner_size(
                crate::window_geometry::DEFAULT_WINDOW_SIZE.0,
                crate::window_geometry::DEFAULT_WINDOW_SIZE.1,
            )
            .min_inner_size(
                crate::window_geometry::MIN_WINDOW_SIZE.0,
                crate::window_geometry::MIN_WINDOW_SIZE.1,
            )
            .transparent(true)
            .visible(false);
    let builder = if let Some(directory) = crate::webview_storage::data_directory(
        state.paths().portable_root().map(|root| root.as_path()),
    ) {
        builder.data_directory(directory)
    } else {
        builder
    };
    #[cfg(target_os = "linux")]
    let builder = builder.center();
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    let window = builder.build().map_err(|error| {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            pending_geometry.complete(Err(error.to_string()));
            geometry.remove(&label);
        }
        state.launches().remove(&label);
        io_error(error)
    })?;
    crate::register_desktop_window_events(&window);
    #[cfg(target_os = "macos")]
    {
        let result = crate::window_presentation_macos::prepare_window(&window, state);
        if let Err(error) = result {
            pending_geometry.complete(Err(error.to_string()));
            state.launches().set_closing(&label, true);
            return match window.destroy() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io_error(format!(
                    "{error}; failed to destroy the uninitialized window: {cleanup}"
                ))),
            };
        }
        crate::complete_window_restore(&window, pending_geometry);
    }
    #[cfg(target_os = "linux")]
    {
        let result = crate::platform::initialize_window_geometry(&window)
            .and_then(|()| crate::window_persistence_linux::restore_window(&window, state, None));
        match result {
            Ok(constructor) => {
                crate::complete_window_restore_linux(&window, pending_geometry, constructor);
            }
            Err(error) => {
                pending_geometry.complete(Err(error.to_string()));
                state.launches().set_closing(&label, true);
                return match window.destroy() {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(io_error(format!(
                        "{error}; failed to destroy the uninitialized window: {cleanup}"
                    ))),
                };
            }
        }
    }
    #[cfg(windows)]
    {
        let result = crate::platform::initialize_window_geometry(&window);
        pending_geometry.complete(result.as_ref().map(|_| ()).map_err(ToString::to_string));
        if let Err(error) = result {
            state.launches().set_closing(&label, true);
            return match window.destroy() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io_error(format!(
                    "{error}; failed to destroy the uninitialized window: {cleanup}"
                ))),
            };
        }
    }
    Ok(window)
}

#[tauri::command]
pub fn window_get_state(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<WindowStateSnapshot, AppError> {
    window_state(&window)
}

#[tauri::command]
pub async fn window_apply_state(
    window: tauri::WebviewWindow,
    request: WindowStatePatch,
) -> Result<(), AppError> {
    let state = window.state::<AppState>();
    let _operations = state.lock_window_operations().await;
    let bounds = request.bounds;
    let maximized = request.maximized;

    if bounds.is_some() || maximized.is_some() || request.visible == Some(true) {
        crate::platform::wait_for_window_operations(&window).await?;
    }

    if let Some(frame) = request.frame.as_deref() {
        crate::platform::set_window_frame(&window, frame)?;
    }
    if let Some(title) = request.title.as_deref() {
        window.set_title(title).map_err(io_error)?;
    }
    if let Some(value) = request.always_on_top {
        crate::platform::set_window_always_on_top(&window, value)?;
    }
    if let Some(value) = request.fullscreen {
        window.set_fullscreen(value).map_err(io_error)?;
        #[cfg(target_os = "linux")]
        crate::platform::window_completion_linux::wait_for_fullscreen(&window, value).await?;
    }

    if bounds.is_some() && maximized.is_some() {
        window.unmaximize().map_err(io_error)?;
        crate::platform::wait_for_window_operations(&window).await?;
        #[cfg(target_os = "linux")]
        crate::platform::window_completion_linux::wait_for_maximized(&window, false).await?;
    }

    #[cfg(target_os = "linux")]
    let bounds = if bounds.is_some()
        && !crate::platform::window_completion_linux::accepts_bounds(&window)?
    {
        None
    } else {
        bounds
    };

    if let Some(bounds) = bounds {
        let size = PhysicalSize::new(bounds.width, bounds.height);
        #[cfg(target_os = "macos")]
        crate::platform::set_window_outer_size_async(&window, size).await?;
        #[cfg(not(target_os = "macos"))]
        set_bounds_size(&window, size)?;

        if window_capabilities(&window)?.absolute_positioning {
            #[cfg(target_os = "linux")]
            crate::platform::window_completion_linux::wait_for_size(&window, size).await?;
            set_bounds_position(&window, PhysicalPosition::new(bounds.x, bounds.y))?;
        }
        crate::platform::wait_for_window_operations(&window).await?;
        #[cfg(target_os = "linux")]
        crate::platform::window_completion_linux::wait_for_bounds(&window, bounds).await?;
        #[cfg(target_os = "linux")]
        {
            let capture_window = window.clone();
            window
                .run_on_main_thread(move || {
                    let state = capture_window.state::<AppState>();
                    if let Err(error) =
                        crate::window_persistence_linux::capture_window(&capture_window, &state)
                    {
                        eprintln!("failed to record applied window bounds: {error}");
                    }
                })
                .map_err(io_error)?;
            crate::platform::wait_for_window_operations(&window).await?;
        }
    }
    if let Some(maximized) = maximized {
        if maximized {
            window.maximize().map_err(io_error)?;
        } else if request.bounds.is_none() {
            window.unmaximize().map_err(io_error)?;
        }
        crate::platform::wait_for_window_operations(&window).await?;
        #[cfg(target_os = "linux")]
        crate::platform::window_completion_linux::wait_for_maximized(&window, maximized).await?;
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
pub async fn window_toggle_maximize(
    window: tauri::WebviewWindow,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    let state = window.state::<AppState>();
    let _operations = state.lock_window_operations().await;
    crate::platform::wait_for_window_operations(&window).await?;
    let maximize = !crate::platform::window_is_maximized(&window)?;
    if maximize {
        window.maximize().map_err(io_error)?;
    } else {
        window.unmaximize().map_err(io_error)?;
    }
    #[cfg(target_os = "linux")]
    crate::platform::window_completion_linux::wait_for_maximized(&window, maximize).await?;
    crate::platform::wait_for_window_operations(&window).await
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tauri::command]
pub async fn window_close(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    crate::window_close::close_window(&window, &state).await
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[tauri::command]
pub fn window_close(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    _request: serde_json::Value,
) -> Result<(), AppError> {
    // The renderer has already confirmed and closed its tabs. Do not request
    // confirmation again through the native CloseRequested handler.
    state.launches().set_closing(window.label(), true);
    window.destroy().map_err(|error| {
        state.launches().set_closing(window.label(), false);
        let _ = window.emit_to(window.label(), "app:launch", ());
        io_error(error)
    })
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
pub async fn window_set_docking(
    window: tauri::WebviewWindow,
    request: DockingOptions,
) -> Result<WindowStateSnapshot, AppError> {
    if request.side == "off" {
        #[cfg(target_os = "linux")]
        window_apply_state(
            window.clone(),
            WindowStatePatch {
                always_on_top: Some(false),
                ..Default::default()
            },
        )
        .await?;
        #[cfg(not(target_os = "linux"))]
        window.set_always_on_top(false).map_err(io_error)?;
        return window_state(&window);
    }
    if !window_capabilities(&window)?.docking {
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

    #[cfg(target_os = "linux")]
    window_apply_state(
        window.clone(),
        WindowStatePatch {
            bounds: Some(bounds),
            always_on_top: Some(request.always_on_top),
            ..Default::default()
        },
    )
    .await?;
    #[cfg(not(target_os = "linux"))]
    {
        set_bounds_size(&window, PhysicalSize::new(bounds.width, bounds.height))?;
        set_bounds_position(&window, PhysicalPosition::new(bounds.x, bounds.y))?;
        window
            .set_always_on_top(request.always_on_top)
            .map_err(io_error)?;
    }
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
    pub html: Option<String>,
}

#[tauri::command]
pub fn clipboard_write_text(
    app: tauri::AppHandle,
    request: ClipboardWriteRequest,
) -> Result<(), AppError> {
    match request.html {
        Some(html) => app.clipboard().write_html(html, Some(request.text)),
        None => app.clipboard().write_text(request.text),
    }
    .map_err(io_error)
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecRequest {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[tauri::command]
pub async fn desktop_exec(request: ExecRequest) -> Result<(), AppError> {
    if request.executable.trim().is_empty() {
        return Err(AppError::InvalidArgument(
            "executable must not be empty".into(),
        ));
    }

    let status = Command::new(&request.executable)
        .args(&request.args)
        .status()
        .await
        .map_err(io_error)?;
    if !status.success() {
        return Err(AppError::Io(format!("process exited with {status}")));
    }
    Ok(())
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

#[cfg(test)]
mod clipboard_tests {
    use super::ClipboardWriteRequest;

    #[test]
    fn clipboard_accepts_plain_text_and_formatted_requests() {
        let plain: ClipboardWriteRequest =
            serde_json::from_value(serde_json::json!({ "text": "台灣" })).unwrap();
        assert_eq!(plain.text, "台灣");
        assert_eq!(plain.html, None);

        let formatted: ClipboardWriteRequest = serde_json::from_value(serde_json::json!({
            "text": "台灣", "html": "<b>台灣</b>"
        }))
        .unwrap();
        assert_eq!(formatted.text, "台灣");
        assert_eq!(formatted.html.as_deref(), Some("<b>台灣</b>"));
    }
}
