mod commands;
mod error;
mod identity;
mod launch;
mod pty;
mod security;
mod shell;
mod state;
mod storage;

use std::sync::Arc;

use commands::{
    app::{app_bootstrap, app_quit, app_runtime_info},
    backup::{backup_create, backup_list, backup_restore},
    config::{config_read, config_write},
    identity::{identity_alias_status, identity_get, identity_set_alias},
    keychain::{keychain_delete, keychain_get, keychain_put},
    launch::app_initial_launch,
    migration::{migration_detect, migration_execute},
    pty::{
        pty_ack, pty_attach, pty_detach, pty_exists, pty_get_children, pty_get_cwd, pty_get_pid,
        pty_get_true_pid, pty_kill, pty_resize, pty_spawn, pty_write,
    },
    secrets::{secret_import_execute, secret_import_plan},
    shell::{shell_detect, shell_prepare_spawn},
    vault::{
        vault_get_file, vault_get_secret, vault_lock, vault_put_file, vault_put_secret,
        vault_remove_secret, vault_replace, vault_set_config, vault_set_enabled, vault_snapshot,
        vault_status, vault_summary, vault_unlock, vault_update_secret,
    },
};
use launch::{parse_launch_context, LaunchContext};
use pty::PtyManager;
use security::{CredentialState, SecretState};
use state::AppState;
use storage::{
    paths::StoragePaths,
    state_file::{load_state, save_state},
};
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
            let storage_paths = StoragePaths::from_app_paths(&paths);
            storage_paths.ensure_layout()?;
            let state_file_existed = std::fs::symlink_metadata(storage_paths.state_file()).is_ok();
            let persisted_state = load_state(storage_paths.state_file())?;
            if !state_file_existed {
                save_state(storage_paths.state_file(), &persisted_state)?;
            }
            app.manage(AppState::new(paths, initial_launch));
            app.manage(Arc::new(SecretState::default()));
            app.manage(Arc::new(PtyManager::default()));
            app.manage(CredentialState::default());

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
            backup_create,
            backup_list,
            backup_restore,
            config_read,
            config_write,
            identity_get,
            identity_alias_status,
            identity_set_alias,
            keychain_get,
            keychain_put,
            keychain_delete,
            migration_detect,
            migration_execute,
            pty_spawn,
            pty_exists,
            pty_attach,
            pty_detach,
            pty_write,
            pty_resize,
            pty_kill,
            pty_ack,
            pty_get_pid,
            pty_get_true_pid,
            pty_get_children,
            pty_get_cwd,
            secret_import_plan,
            secret_import_execute,
            shell_detect,
            shell_prepare_spawn,
            vault_status,
            vault_unlock,
            vault_replace,
            vault_lock,
            vault_set_enabled,
            vault_summary,
            vault_snapshot,
            vault_get_secret,
            vault_put_secret,
            vault_update_secret,
            vault_remove_secret,
            vault_set_config,
            vault_put_file,
            vault_get_file,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Tabby RS");
}
