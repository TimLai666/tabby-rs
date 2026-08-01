mod commands;
mod error;
mod identity;
mod launch;
mod state;

use commands::{
    app::{app_bootstrap, app_quit, app_runtime_info},
    identity::{identity_alias_status, identity_get, identity_set_alias},
    launch::app_initial_launch,
};
use launch::{parse_launch_context, LaunchContext};
use state::AppState;
use tauri::{Emitter, Manager};

fn initial_launch_context() -> LaunchContext {
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .into_os_string()
        .into_string();
    let argv = std::env::args_os()
        .map(|argument| argument.into_string())
        .collect::<Result<Vec<_>, _>>();

    match (argv, cwd) {
        (Ok(argv), Ok(cwd)) => parse_launch_context(&argv, cwd, false),
        _ => LaunchContext {
            request: Default::default(),
            cwd: ".".into(),
            second_instance: false,
            parse_error: Some("launch arguments or working directory are not valid UTF-8".into()),
        },
    }
}

fn present_and_dispatch(app: &tauri::AppHandle, context: LaunchContext) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Err(error) = app.emit("app.launch", context) {
        eprintln!("failed to emit app.launch: {error}");
    }
}

pub fn run() {
    let initial_launch = initial_launch_context();
    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let context = parse_launch_context(&argv, cwd, true);
            present_and_dispatch(app, context);
        }));
    }

    builder = builder.plugin(tauri_plugin_deep_link::init());

    builder
        .setup(move |app| {
            use tauri_plugin_deep_link::DeepLinkExt;

            let mut initial_launch = initial_launch.clone();

            #[cfg(target_os = "macos")]
            if let Some(urls) = app.deep_link().get_current()? {
                let mut argv = vec![identity::CLI_NAME.to_owned()];
                argv.extend(urls.iter().map(ToString::to_string));
                initial_launch = parse_launch_context(&argv, initial_launch.cwd.clone(), false);
            }

            let paths = identity::AppPaths::detect(app.handle())?;
            app.manage(AppState::new(paths, initial_launch));

            #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
            app.deep_link().register_all()?;

            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    let mut argv = vec![identity::CLI_NAME.to_owned()];
                    argv.extend(event.urls().iter().map(ToString::to_string));
                    let cwd = std::env::current_dir()
                        .unwrap_or_else(|_| std::path::PathBuf::from("."))
                        .to_string_lossy()
                        .into_owned();
                    present_and_dispatch(&handle, parse_launch_context(&argv, cwd, true));
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_bootstrap,
            app_runtime_info,
            app_initial_launch,
            app_quit,
            identity_get,
            identity_alias_status,
            identity_set_alias,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Tabby RS");
}
