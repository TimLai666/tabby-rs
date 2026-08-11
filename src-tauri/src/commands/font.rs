use crate::{error::AppError, font::list_installed_fonts};

#[tauri::command]
pub fn font_list() -> Result<Vec<crate::font::InstalledFont>, AppError> {
    list_installed_fonts()
}

#[tauri::command]
pub fn font_refresh() -> Result<Vec<crate::font::InstalledFont>, AppError> {
    list_installed_fonts()
}
