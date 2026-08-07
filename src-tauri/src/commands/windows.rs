use tauri::AppHandle;

use crate::windows_integration::{self, WindowsIntegrationStatus};

#[tauri::command]
pub fn windows_integration_status(app: AppHandle) -> WindowsIntegrationStatus {
    windows_integration::status(&app)
}
