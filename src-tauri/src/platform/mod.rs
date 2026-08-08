use crate::{desktop::VibrancyOptions, error::AppError};

pub fn set_window_opacity(
    _window: &tauri::WebviewWindow,
    opacity: f64,
) -> Result<(), AppError> {
    if !(0.0..=1.0).contains(&opacity) {
        return Err(AppError::InvalidArgument(
            "window opacity must be between 0 and 1".into(),
        ));
    }
    if (opacity >= 0.999) {
        return Ok(());
    }
    Err(AppError::Unsupported(
        "native window opacity adapter is unavailable on this platform build".into(),
    ))
}

pub fn set_window_vibrancy(
    window: &tauri::WebviewWindow,
    options: &VibrancyOptions,
) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    {
        let _ = window;
        if options.enabled {
            return Err(AppError::Unsupported(
                "window vibrancy is unavailable on Linux".into(),
            ));
        }
        return Ok(());
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        use tauri::window::{Effect, EffectsBuilder};

        if !options.enabled {
            return window
                .set_effects(None)
                .map_err(|error| AppError::Io(error.to_string()));
        }
        let effect = options
            .effect
            .as_deref()
            .and_then(parse_effect)
            .unwrap_or(default_effect());
        return window
            .set_effects(EffectsBuilder::new().effect(effect).build())
            .map_err(|error| AppError::Io(error.to_string()));
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn parse_effect(value: &str) -> Option<tauri::window::Effect> {
    use tauri::window::Effect;
    match value.to_ascii_lowercase().as_str() {
        "acrylic" => Some(Effect::Acrylic),
        "blur" => Some(Effect::Blur),
        "mica" => Some(Effect::Mica),
        "micadark" | "mica-dark" => Some(Effect::MicaDark),
        "micalight" | "mica-light" => Some(Effect::MicaLight),
        "tabbed" => Some(Effect::Tabbed),
        "sidebar" => Some(Effect::Sidebar),
        "popover" => Some(Effect::Popover),
        "hudwindow" | "hud-window" => Some(Effect::HudWindow),
        "underwindowbackground" | "under-window-background" => Some(Effect::UnderWindowBackground),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn default_effect() -> tauri::window::Effect {
    tauri::window::Effect::Acrylic
}

#[cfg(target_os = "macos")]
fn default_effect() -> tauri::window::Effect {
    tauri::window::Effect::UnderWindowBackground
}
