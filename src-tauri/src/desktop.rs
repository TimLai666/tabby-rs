use std::hash::{Hash, Hasher};

use tauri::Monitor;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ColorScheme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VibrancyOptions {
    pub enabled: bool,
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStatePatch {
    pub visible: Option<bool>,
    pub always_on_top: Option<bool>,
    pub fullscreen: Option<bool>,
    pub maximized: Option<bool>,
    pub bounds: Option<WindowBounds>,
    pub opacity: Option<f64>,
    pub progress: Option<Option<f64>>,
    pub vibrancy: Option<VibrancyOptions>,
    pub color_scheme: Option<ColorScheme>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowCapabilities {
    pub absolute_positioning: bool,
    pub docking: bool,
    pub global_hotkey: bool,
    pub opacity: bool,
    pub vibrancy: bool,
    pub progress: bool,
    pub clipboard: bool,
    pub dialogs: bool,
    pub notifications: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStateSnapshot {
    pub visible: bool,
    pub always_on_top: bool,
    pub fullscreen: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub focused: bool,
    pub bounds: WindowBounds,
    pub scale_factor: f64,
    pub capabilities: WindowCapabilities,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInfo {
    pub id: i64,
    pub name: String,
    pub primary: bool,
    pub scale_factor: f64,
    pub bounds: WindowBounds,
    pub work_area: WindowBounds,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockingOptions {
    pub side: String,
    pub screen_id: Option<i64>,
    pub fill: f64,
    pub space: f64,
    pub always_on_top: bool,
    pub min_width: u32,
    pub min_height: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHotkeyRegistration {
    pub id: String,
    pub accelerators: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHotkeyEvent {
    pub id: String,
    pub accelerator: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDialogOptions {
    pub multiple: bool,
    pub directory: bool,
    pub title: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDialogOptions {
    pub title: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopNotification {
    pub title: String,
    pub body: Option<String>,
}

pub fn capabilities() -> WindowCapabilities {
    let wayland = cfg!(target_os = "linux") && is_wayland_session();
    WindowCapabilities {
        absolute_positioning: !wayland,
        docking: !wayland,
        global_hotkey: true,
        opacity: cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux")),
        vibrancy: cfg!(any(target_os = "windows", target_os = "macos")),
        progress: true,
        clipboard: true,
        dialogs: true,
        notifications: true,
    }
}

pub fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|value| value.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

pub fn screen_info(monitors: &[Monitor], primary: Option<&Monitor>) -> Vec<ScreenInfo> {
    let primary_key = primary.map(monitor_key);
    let mut screens = monitors
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            let work_area = monitor.work_area();
            let key = monitor_key(monitor);
            ScreenInfo {
                id: stable_screen_id(&key),
                name: monitor
                    .name()
                    .cloned()
                    .unwrap_or_else(|| "Display".to_owned()),
                primary: primary_key.as_deref() == Some(key.as_str()),
                scale_factor: monitor.scale_factor(),
                bounds: WindowBounds {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                },
                work_area: WindowBounds {
                    x: work_area.position.x,
                    y: work_area.position.y,
                    width: work_area.size.width,
                    height: work_area.size.height,
                },
            }
        })
        .collect::<Vec<_>>();
    screens.sort_by_key(|screen| (screen.bounds.x, screen.bounds.y));
    for (index, screen) in screens.iter_mut().enumerate() {
        if screen.primary {
            screen.name = "Primary Display".to_owned();
        } else if screen.name == "Display" {
            screen.name = format!("Display {}", index + 1);
        }
    }
    screens
}

pub fn docked_bounds(options: &DockingOptions, screen: WindowBounds) -> Result<WindowBounds, AppError> {
    if options.side == "off" {
        return Ok(screen);
    }
    if !matches!(options.side.as_str(), "left" | "right" | "top" | "bottom") {
        return Err(AppError::InvalidArgument("invalid docking side".into()));
    }

    let fill = options.fill.clamp(0.0, 1.0);
    let space = options.space.clamp(0.0, 1.0);
    let mut result = WindowBounds::default();

    if matches!(options.side.as_str(), "left" | "right") {
        result.width = options.min_width.max((f64::from(screen.width) * fill).round() as u32);
        result.width = result.width.min(screen.width);
        result.height = (f64::from(screen.height) * space).round() as u32;
        result.height = result.height.max(1).min(screen.height);
    } else {
        result.width = (f64::from(screen.width) * space).round() as u32;
        result.width = result.width.max(1).min(screen.width);
        result.height = options.min_height.max((f64::from(screen.height) * fill).round() as u32);
        result.height = result.height.min(screen.height);
    }

    result.x = match options.side.as_str() {
        "left" => screen.x,
        "right" => screen.x + screen.width as i32 - result.width as i32,
        _ => screen.x + (screen.width as i32 - result.width as i32) / 2,
    };
    result.y = match options.side.as_str() {
        "top" => screen.y,
        "bottom" => screen.y + screen.height as i32 - result.height as i32,
        _ => screen.y + (screen.height as i32 - result.height as i32) / 2,
    };
    Ok(result)
}

pub fn clamp_bounds(bounds: WindowBounds, screen: WindowBounds) -> WindowBounds {
    let width = bounds.width.max(1).min(screen.width.max(1));
    let height = bounds.height.max(1).min(screen.height.max(1));
    let max_x = screen.x + screen.width as i32 - width as i32;
    let max_y = screen.y + screen.height as i32 - height as i32;
    WindowBounds {
        x: bounds.x.clamp(screen.x, max_x.max(screen.x)),
        y: bounds.y.clamp(screen.y, max_y.max(screen.y)),
        width,
        height,
    }
}

pub fn normalize_accelerator(input: &str) -> Result<String, AppError> {
    let first_chord = input.split_whitespace().next().unwrap_or_default().trim();
    if first_chord.is_empty() || first_chord.len() > 128 || first_chord.chars().any(char::is_control) {
        return Err(AppError::InvalidArgument("invalid global shortcut".into()));
    }

    let normalized = first_chord.replace('⌘', "Super").replace('⌥', "Alt");
    let tokens = normalized
        .replace('-', "+")
        .split('+')
        .filter(|token| !token.trim().is_empty())
        .map(|token| match token.trim().to_ascii_lowercase().as_str() {
            "meta" | "super" | "command" | "cmd" => "Super".to_owned(),
            "control" | "ctrl" => "Control".to_owned(),
            "option" | "alt" => "Alt".to_owned(),
            "shift" => "Shift".to_owned(),
            "commandorcontrol" | "cmdorctrl" => "CommandOrControl".to_owned(),
            _ => token.trim().to_owned(),
        })
        .collect::<Vec<_>>();

    if tokens.len() < 2 {
        return Err(AppError::InvalidArgument("global shortcut requires a modifier".into()));
    }
    Ok(tokens.join("+"))
}

fn monitor_key(monitor: &Monitor) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        monitor.name().cloned().unwrap_or_default(),
        monitor.position().x,
        monitor.position().y,
        monitor.size().width,
        monitor.size().height
    )
}

fn stable_screen_id(value: &str) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() & 0x001f_ffff_ffff_ffff) as i64
}

#[cfg(test)]
mod tests {
    use super::{clamp_bounds, docked_bounds, normalize_accelerator, DockingOptions, WindowBounds};

    #[test]
    fn normalizes_legacy_accelerators() {
        assert_eq!(normalize_accelerator("Meta-Shift-T").unwrap(), "Super+Shift+T");
        assert_eq!(normalize_accelerator("⌘-⌥-Space").unwrap(), "Super+Alt+Space");
        assert_eq!(normalize_accelerator("Ctrl-Alt-T Ctrl-X").unwrap(), "Control+Alt+T");
    }

    #[test]
    fn rejects_unmodified_shortcuts() {
        assert!(normalize_accelerator("F12").is_err());
    }

    #[test]
    fn computes_docking_bounds() {
        let screen = WindowBounds { x: 100, y: 50, width: 1000, height: 800 };
        let options = DockingOptions {
            side: "right".into(),
            screen_id: None,
            fill: 0.5,
            space: 0.75,
            always_on_top: true,
            min_width: 320,
            min_height: 240,
        };
        assert_eq!(
            docked_bounds(&options, screen).unwrap(),
            WindowBounds { x: 600, y: 150, width: 500, height: 600 }
        );
    }

    #[test]
    fn clamps_window_back_onto_visible_area() {
        let screen = WindowBounds { x: 0, y: 0, width: 1920, height: 1080 };
        assert_eq!(
            clamp_bounds(WindowBounds { x: 2500, y: -500, width: 900, height: 700 }, screen),
            WindowBounds { x: 1020, y: 0, width: 900, height: 700 }
        );
    }
}
