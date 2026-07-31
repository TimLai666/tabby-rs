mod commands;
mod error;
mod state;

use commands::app::{app_bootstrap, app_quit, app_runtime_info};
use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            app_bootstrap,
            app_runtime_info,
            app_quit,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Tabby RS");
}
