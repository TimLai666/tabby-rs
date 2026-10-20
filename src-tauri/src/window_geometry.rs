use tauri::utils::config::Config;

#[cfg(target_os = "macos")]
pub(crate) const DEFAULT_WINDOW_SIZE: (f64, f64) = (800.0, 600.0);
#[cfg(not(target_os = "macos"))]
pub(crate) const DEFAULT_WINDOW_SIZE: (f64, f64) = (1100.0, 720.0);

#[cfg(target_os = "macos")]
pub(crate) const MIN_WINDOW_SIZE: (f64, f64) = (400.0, 300.0);
#[cfg(not(target_os = "macos"))]
pub(crate) const MIN_WINDOW_SIZE: (f64, f64) = (640.0, 480.0);

pub(crate) fn prepare_startup(config: &mut Config) {
    #[cfg(target_os = "macos")]
    if let Some(window) = config
        .app
        .windows
        .iter_mut()
        .find(|window| window.label == "main")
    {
        // Overlay construction uses the whole outer frame as its content area.
        // Replace only the bundled stock geometry; preserve custom declarations.
        if window.width == 1100.0
            && window.height == 720.0
            && window.min_width == Some(640.0)
            && window.min_height == Some(480.0)
            && window.title_bar_style == tauri::TitleBarStyle::Overlay
        {
            window.width = DEFAULT_WINDOW_SIZE.0;
            window.height = DEFAULT_WINDOW_SIZE.1;
            window.min_width = Some(MIN_WINDOW_SIZE.0);
            window.min_height = Some(MIN_WINDOW_SIZE.1);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = config;
}
