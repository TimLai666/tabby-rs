use crate::{desktop::VibrancyOptions, error::AppError};

mod frame;
pub use frame::{set_window_frame, window_caption_controls};

#[cfg(target_os = "windows")]
mod window_size;
#[cfg(target_os = "windows")]
pub use window_size::set_window_outer_size;

#[cfg(target_os = "macos")]
pub(crate) mod macos_quit;

#[cfg(all(test, target_os = "macos"))]
mod material_tests;

pub fn set_window_opacity(window: &tauri::WebviewWindow, opacity: f64) -> Result<(), AppError> {
    validate_opacity(opacity)?;

    #[cfg(target_os = "macos")]
    return set_macos_window_opacity(window, opacity);

    #[cfg(target_os = "windows")]
    return set_windows_window_opacity(window, opacity);

    #[cfg(target_os = "linux")]
    {
        let _ = window;
        Err(AppError::Unsupported(
            "window opacity is unavailable on Linux".into(),
        ))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = window;
        Err(AppError::Unsupported(
            "window opacity is unavailable on this platform".into(),
        ))
    }
}

fn validate_opacity(opacity: f64) -> Result<(), AppError> {
    if !(0.0..=1.0).contains(&opacity) {
        return Err(AppError::InvalidArgument(
            "window opacity must be between 0 and 1".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_macos_window_opacity(window: &tauri::WebviewWindow, opacity: f64) -> Result<(), AppError> {
    use objc2_app_kit::NSWindow;
    use std::sync::mpsc::sync_channel;

    let (sender, receiver) = sync_channel(1);
    let window = window.clone();
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let native_window = main_window
                    .ns_window()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                let native_window: &NSWindow = unsafe { &*native_window.cast() };
                native_window.setOpaque(false);
                native_window.setAlphaValue(opacity);
                Ok(())
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;

    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
}

#[cfg(target_os = "windows")]
fn set_windows_window_opacity(window: &tauri::WebviewWindow, opacity: f64) -> Result<(), AppError> {
    use std::sync::mpsc::sync_channel;
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED,
    };

    let (sender, receiver) = sync_channel(1);
    let window = window.clone();
    let main_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let hwnd = main_window
                    .hwnd()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                unsafe { SetLastError(0) };
                let style = unsafe { GetWindowLongPtrW(hwnd.0, GWL_EXSTYLE) } as u32;
                if style == 0 && unsafe { GetLastError() } != 0 {
                    return Err(AppError::Io(std::io::Error::last_os_error().to_string()));
                }
                let layered_style = style | WS_EX_LAYERED;
                let added_layered_style = layered_style != style;
                if layered_style != style {
                    unsafe { SetLastError(0) };
                    let previous =
                        unsafe { SetWindowLongPtrW(hwnd.0, GWL_EXSTYLE, layered_style as isize) };
                    if previous == 0 && unsafe { GetLastError() } != 0 {
                        return Err(AppError::Io(std::io::Error::last_os_error().to_string()));
                    }
                }

                let alpha = (opacity * 255.0).round() as u8;
                if unsafe { SetLayeredWindowAttributes(hwnd.0, 0, alpha, LWA_ALPHA) } == 0 {
                    let error = std::io::Error::last_os_error();
                    if added_layered_style {
                        unsafe {
                            SetLastError(0);
                            let _ = SetWindowLongPtrW(hwnd.0, GWL_EXSTYLE, style as isize);
                        }
                    }
                    return Err(AppError::Io(error.to_string()));
                }
                Ok(())
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| AppError::Io(error.to_string()))?;

    receiver
        .recv()
        .map_err(|error| AppError::Io(error.to_string()))?
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

    #[cfg(target_os = "macos")]
    {
        use std::sync::mpsc::sync_channel;

        let (sender, receiver) = sync_channel(1);
        let window = window.clone();
        let main_window = window.clone();
        let enabled = options.enabled;
        window
            .run_on_main_thread(move || {
                let result = (|| {
                    while window_vibrancy::clear_vibrancy(&main_window)
                        .map_err(|error| AppError::Io(error.to_string()))?
                    {}

                    if enabled {
                        let material =
                            macos_vibrancy_material(sysinfo::System::kernel_version().as_deref());
                        window_vibrancy::apply_vibrancy(
                            &main_window,
                            material,
                            Some(window_vibrancy::NSVisualEffectState::Active),
                            None,
                        )
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    }
                    Ok(())
                })();
                let _ = sender.send(result);
            })
            .map_err(|error| AppError::Io(error.to_string()))?;

        return receiver
            .recv()
            .map_err(|error| AppError::Io(error.to_string()))?;
    }

    #[cfg(target_os = "windows")]
    {
        use tauri::window::EffectsBuilder;

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

#[cfg(target_os = "windows")]
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
fn macos_vibrancy_material(version: Option<&str>) -> window_vibrancy::NSVisualEffectMaterial {
    // The fixed upstream 14e2d60 macos-release table recognizes Darwin 18..=25
    // as macOS 10.14+. Unrecognized releases keep its Dark fallback.
    let major = version
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.parse::<u32>().ok());

    match major {
        Some(18..=25) => window_vibrancy::NSVisualEffectMaterial::FullScreenUI,
        _ => window_vibrancy::NSVisualEffectMaterial::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::validate_opacity;

    #[test]
    fn validates_window_opacity_bounds() {
        assert!(validate_opacity(0.0).is_ok());
        assert!(validate_opacity(1.0).is_ok());
        assert!(validate_opacity(-0.01).is_err());
        assert!(validate_opacity(1.01).is_err());
        assert!(validate_opacity(f64::NAN).is_err());
    }
}
