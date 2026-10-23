//! Fixed-upstream first-load appearance and saved maximization.
use crate::{error::AppError, state::AppState};

/// Read raw startup appearance without creating config.yaml, apply the process
/// frame snapshot before restore/capture, then restore the saved normal bounds.
/// Defer raw-config errors to the existing bootstrap error flow.
pub(crate) fn prepare_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    match crate::commands::app::startup_window_config(state) {
        Ok(config) => {
            crate::platform::set_window_frame(window, state.startup_window_frame(&config))?;
        }
        Err(error) => eprintln!("{error}"),
    }

    crate::window_persistence_macos::restore_window(window, state)
}

/// Called once after the owning page finishes and initialization succeeds.
/// Serialize with geometry, apply vibrancy and raw colorSchemeMode (light/auto/
/// otherwise dark), retain hidden startup, otherwise maximize/show then focus.
/// Drain dispatched native work before returning, and ignore missing/closing labels.
pub(crate) async fn present_window(
    window: &tauri::WebviewWindow,
    state: &AppState,
) -> Result<(), AppError> {
    let _guard = state.lock_window_operations().await;
    let label = window.label();
    if state.startup_window_maximized(label).is_none() || state.launches().is_closing(label) {
        return Ok(());
    }

    let config = crate::commands::app::startup_window_config(state)?;
    crate::platform::set_window_vibrancy(
        window,
        &crate::desktop::VibrancyOptions {
            enabled: true,
            effect: None,
        },
    )?;

    let theme = match config
        .get("appearance")
        .and_then(serde_json::Value::as_object)
        .and_then(|appearance| appearance.get("colorSchemeMode"))
        .and_then(serde_json::Value::as_str)
    {
        Some("light") => Some(tauri::Theme::Light),
        Some("auto") => None,
        _ => Some(tauri::Theme::Dark),
    };
    window
        .set_theme(theme)
        .map_err(|error| AppError::Io(error.to_string()))?;

    crate::platform::wait_for_window_operations(window).await?;
    if state.startup_window_hidden(label)
        || state.launches().is_closing(label)
        || state.startup_window_maximized(label).is_none()
    {
        return Ok(());
    }

    if state.startup_window_maximized(label) == Some(true) {
        window
            .maximize()
            .map_err(|error| AppError::Io(error.to_string()))?;
        crate::platform::wait_for_window_operations(window).await?;
    }

    // Approved close can claim the label while this operation awaits native zoom.
    if state.launches().is_closing(label) || state.startup_window_maximized(label).is_none() {
        return Ok(());
    }
    window
        .show()
        .map_err(|error| AppError::Io(error.to_string()))?;
    window
        .set_focus()
        .map_err(|error| AppError::Io(error.to_string()))?;
    crate::platform::wait_for_window_operations(window).await?;
    Ok(())
}
