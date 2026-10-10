mod commands;
mod context_menu;
mod desktop;
mod diagnostics;
mod error;
mod font;
mod identity;
mod launch;
mod message_box;
mod platform;
mod plugins;
mod pty;
mod security;
mod serial;
mod shell;
mod ssh;
mod state;
mod storage;
mod sudo;
mod telnet;
mod transfer;
pub mod update;
mod windows_integration;
mod winscp;

use context_menu::menu_popup;
use message_box::dialog_message;
use std::sync::Arc;

use commands::{
    app::{
        app_benchmark_frame_report, app_benchmark_ready, app_bootstrap, app_installer_smoke_ready,
        app_quit, app_runtime_info,
    },
    backup::{backup_create, backup_list, backup_restore},
    config::{config_read, config_write},
    desktop::{
        clipboard_read_text, clipboard_write_text, desktop_exec, desktop_open_external,
        desktop_open_path, desktop_read_file, desktop_reveal_path, dialog_open, dialog_save,
        hotkey_replace, notification_show, window_apply_state, window_bring_to_front, window_close,
        window_get_state, window_list_screens, window_minimize, window_new, window_open_devtools,
        window_reload, window_set_docking, window_toggle_maximize, window_toggle_quake,
    },
    diagnostics::{
        diagnostics_append, diagnostics_clear_logs, diagnostics_export, diagnostics_preview,
        diagnostics_status,
    },
    file_edit::{file_edit_prepare, file_edit_ready, file_edit_stop, file_edit_watch},
    font::{font_list, font_refresh},
    identity::{identity_alias_status, identity_get, identity_set_alias},
    keychain::{keychain_delete, keychain_get, keychain_put},
    launch::app_initial_launch,
    migration::{migration_detect, migration_execute},
    plugins::{
        plugins_bootstrap_failed, plugins_bootstrap_plugin_completed,
        plugins_bootstrap_plugin_started, plugins_bootstrap_retry, plugins_bootstrap_succeeded,
        plugins_cancel_operation, plugins_discover, plugins_install, plugins_list_installed,
        plugins_node_status, plugins_prepare_operation, plugins_read_entry, plugins_remove,
        plugins_uninstall, plugins_update,
    },
    pty::{
        pty_ack, pty_attach, pty_detach, pty_exists, pty_get_children, pty_get_cwd, pty_get_pid,
        pty_get_true_pid, pty_is_alive, pty_kill, pty_resize, pty_spawn, pty_write,
    },
    secrets::{secret_import_execute, secret_import_plan},
    serial::{
        serial_close, serial_get_signals, serial_list, serial_open, serial_set_baud_rate,
        serial_set_signals, serial_write,
    },
    sftp::{
        sftp_cancel_transfer, sftp_chmod, sftp_close, sftp_close_transfer, sftp_download,
        sftp_download_open, sftp_list, sftp_mkdir, sftp_open, sftp_read, sftp_readlink,
        sftp_remove, sftp_rename, sftp_stat, sftp_upload, sftp_upload_open, sftp_write,
    },
    shell::{shell_detect, shell_prepare_spawn},
    ssh::{
        ssh_auth_response, ssh_cancel_connect, ssh_close, ssh_connect, ssh_forwarding_list,
        ssh_forwarding_start, ssh_forwarding_stop, ssh_host_key_decision, ssh_import_apply,
        ssh_import_preview, ssh_list_private_keys, ssh_resize, ssh_resolve_agent_socket, ssh_write,
    },
    sudo::sudo_respond,
    telnet::{telnet_close, telnet_connect, telnet_resize, telnet_write},
    transfer::{
        terminal_export, transfer_cancel, transfer_close, transfer_create_directory,
        transfer_list_directory, transfer_open_download, transfer_open_upload, transfer_read,
        transfer_write,
    },
    update::{
        update_cancel, update_check, update_download, update_get_channel, update_install,
        update_set_channel,
    },
    vault::{
        vault_get_file, vault_get_secret, vault_lock, vault_put_file, vault_put_secret,
        vault_remove_secret, vault_replace, vault_set_config, vault_set_enabled, vault_snapshot,
        vault_status, vault_summary, vault_unlock, vault_update_secret,
    },
    windows::windows_integration_status,
};
use launch::{parse_launch_context, LaunchContext};
use pty::PtyManager;
use security::{CredentialState, SecretState};
use serial::SerialManager;
use ssh::SshManager;
use state::AppState;
use storage::{paths::StoragePaths, state_file::save_state};
use tauri::{Emitter, Manager};
use telnet::TelnetManager;
use winscp::launch::winscp_launch;
use winscp::prepare::winscp_convert_key;

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
    if let Err(error) = app
        .state::<std::sync::mpsc::Sender<LaunchContext>>()
        .send(context)
    {
        eprintln!("failed to queue launch request: {error}");
    }
}

fn dispatch_launch(app: &tauri::AppHandle, context: LaunchContext) {
    loop {
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
        let handle = app.clone();
        let request = context.clone();
        // Selection and delivery share the event thread with window destruction.
        if let Err(error) = app.run_on_main_thread(move || {
            let state = handle.state::<AppState>();
            let mut windows = handle.webview_windows().into_values().collect::<Vec<_>>();
            windows.retain(|window| !state.launches().is_closing(window.label()));
            windows.sort_by_key(|window| launch::window_creation_order(window.label()));
            for window in &windows {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            let mut delivered = false;
            for window in windows.iter().rev() {
                if state.launches().push(window.label(), request.clone()) {
                    if let Err(error) = window.emit_to(window.label(), "app:launch", ()) {
                        eprintln!("failed to notify window of launch request: {error}");
                    }
                    delivered = true;
                    break;
                }
            }
            let _ = result_tx.send(delivered);
        }) {
            eprintln!("failed to dispatch launch request: {error}");
            return;
        }
        match result_rx.recv() {
            Ok(true) | Err(_) => return,
            Ok(false) => {}
        }
        // Webview2 creation must stay off the event thread. The single receiver
        // completes this request before attempting the next second invocation.
        if let Err(error) =
            commands::desktop::create_window(app, &app.state::<AppState>(), Default::default())
        {
            eprintln!("failed to open launch window: {error}");
            return;
        }
    }
}

pub(crate) fn register_desktop_window_events(window: &tauri::WebviewWindow) {
    let emitter = window.clone();
    window.clone().on_window_event(move |event| match event {
        tauri::WindowEvent::Focused(focused) => {
            let _ = emitter.emit_to(emitter.label(), "desktop:windowFocused", *focused);
        }
        tauri::WindowEvent::Moved(position) => {
            let _ = emitter.emit_to(
                emitter.label(),
                "desktop:windowMoved",
                serde_json::json!({ "x": position.x, "y": position.y }),
            );
        }
        tauri::WindowEvent::Resized(size) => {
            let _ = emitter.emit_to(
                emitter.label(),
                "desktop:windowResized",
                serde_json::json!({ "width": size.width, "height": size.height }),
            );
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = emitter.emit_to(emitter.label(), "desktop:windowCloseRequested", ());
        }
        tauri::WindowEvent::Destroyed => {
            emitter
                .state::<AppState>()
                .launches()
                .remove(emitter.label());
        }
        tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, position }) => {
            let _ = emitter.emit_to(
                emitter.label(),
                "desktop:fileDrop",
                serde_json::json!({
                    "paths": paths
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect::<Vec<_>>(),
                    "x": position.x,
                    "y": position.y,
                }),
            );
        }
        tauri::WindowEvent::ThemeChanged(theme) => {
            let value = match theme {
                tauri::Theme::Dark => "dark",
                tauri::Theme::Light => "light",
                _ => "system",
            };
            let _ = emitter.emit_to(emitter.label(), "desktop:themeChanged", value);
        }
        tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            let _ = emitter.emit_to(
                emitter.label(),
                "desktop:displayMetricsChanged",
                *scale_factor,
            );
        }
        _ => {}
    });
}

fn release_probe_mode() -> bool {
    [
        "TABBY_RS_BENCHMARK_READY_FILE",
        "TABBY_RS_INSTALLER_SMOKE_READY_FILE",
    ]
    .into_iter()
    .any(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .is_some()
    })
}

pub fn run() {
    if crate::update::rollback::maybe_run_update_rollback_helper() {
        return;
    }
    let initial_launch = initial_launch_context();
    let (launch_sender, launch_receiver) = std::sync::mpsc::channel::<LaunchContext>();
    let mut builder = tauri::Builder::default()
        .manage(launch_sender)
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init());
    builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
        if !release_probe_mode() {
            builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
                let context = parse_launch_context(&argv, cwd, true);
                present_and_dispatch(app, context);
            }));
        }
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
            let logs_dir = paths.logs_dir().clone();
            let known_hosts_path = paths.data_dir().join("known_hosts");
            let storage_paths = StoragePaths::from_app_paths(&paths);
            storage_paths.ensure_layout()?;
            let _ = crate::diagnostics::crash::mark_startup(&logs_dir);
            crate::diagnostics::crash::install_panic_hook(logs_dir);
            let state_file_existed = std::fs::symlink_metadata(storage_paths.state_file()).is_ok();
            let persisted_state = crate::update::rollback::recover_pending_update_from_disk(
                &storage_paths,
                &app.package_info().version.to_string(),
            )?;
            if !state_file_existed {
                save_state(storage_paths.state_file(), &persisted_state)?;
            }
            app.manage(AppState::new(paths, initial_launch, persisted_state));
            app.manage(Arc::new(SecretState::default()));
            app.manage(Arc::new(PtyManager::default()));
            app.manage(Arc::new(SshManager::new(known_hosts_path)));
            app.manage(Arc::new(TelnetManager::default()));
            app.manage(Arc::new(SerialManager::default()));
            SerialManager::start_port_watcher(app.handle().clone());
            app.manage(Arc::new(
                crate::transfer::manager::TransferManager::default(),
            ));
            app.manage(Arc::new(
                crate::transfer::file_edit::FileEditManager::default(),
            ));
            app.manage(CredentialState::default());
            if let Some(window) = app.get_webview_window("main") {
                register_desktop_window_events(&window);
            }
            let launch_app = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                while let Ok(context) = launch_receiver.recv() {
                    dispatch_launch(&launch_app, context);
                }
            });

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
            app_benchmark_ready,
            app_benchmark_frame_report,
            app_installer_smoke_ready,
            app_initial_launch,
            app_quit,
            backup_create,
            backup_list,
            backup_restore,
            clipboard_read_text,
            clipboard_write_text,
            config_read,
            config_write,
            diagnostics_status,
            diagnostics_clear_logs,
            diagnostics_append,
            diagnostics_preview,
            diagnostics_export,
            desktop_open_external,
            desktop_open_path,
            desktop_exec,
            font_list,
            font_refresh,
            desktop_read_file,
            desktop_reveal_path,
            dialog_open,
            dialog_save,
            dialog_message,
            menu_popup,
            hotkey_replace,
            identity_get,
            identity_alias_status,
            identity_set_alias,
            keychain_get,
            keychain_put,
            keychain_delete,
            migration_detect,
            migration_execute,
            plugins_bootstrap_failed,
            plugins_bootstrap_plugin_completed,
            plugins_bootstrap_plugin_started,
            plugins_bootstrap_retry,
            plugins_bootstrap_succeeded,
            plugins_cancel_operation,
            plugins_discover,
            plugins_install,
            plugins_list_installed,
            plugins_node_status,
            plugins_prepare_operation,
            plugins_read_entry,
            plugins_remove,
            plugins_uninstall,
            plugins_update,
            notification_show,
            pty_spawn,
            pty_exists,
            pty_is_alive,
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
            sftp_open,
            sftp_list,
            sftp_stat,
            sftp_readlink,
            sftp_chmod,
            sftp_mkdir,
            sftp_rename,
            sftp_remove,
            sftp_upload_open,
            sftp_upload,
            sftp_download_open,
            sftp_download,
            sftp_read,
            sftp_write,
            sftp_close_transfer,
            sftp_cancel_transfer,
            sftp_close,
            ssh_connect,
            ssh_cancel_connect,
            ssh_host_key_decision,
            ssh_import_apply,
            ssh_import_preview,
            ssh_list_private_keys,
            ssh_resolve_agent_socket,
            ssh_auth_response,
            ssh_write,
            ssh_resize,
            ssh_close,
            ssh_forwarding_start,
            ssh_forwarding_stop,
            ssh_forwarding_list,
            telnet_connect,
            telnet_write,
            telnet_resize,
            telnet_close,
            serial_list,
            serial_open,
            serial_write,
            serial_set_baud_rate,
            serial_set_signals,
            serial_get_signals,
            serial_close,
            sudo_respond,
            file_edit_prepare,
            file_edit_ready,
            file_edit_watch,
            file_edit_stop,
            transfer_open_upload,
            transfer_open_download,
            transfer_read,
            transfer_write,
            transfer_close,
            transfer_cancel,
            transfer_create_directory,
            transfer_list_directory,
            terminal_export,
            update_check,
            update_download,
            update_install,
            update_cancel,
            update_set_channel,
            update_get_channel,
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
            window_apply_state,
            window_bring_to_front,
            window_close,
            window_get_state,
            window_list_screens,
            window_minimize,
            window_open_devtools,
            window_new,
            window_reload,
            window_set_docking,
            window_toggle_maximize,
            window_toggle_quake,
            windows_integration_status,
            winscp_convert_key,
            winscp_launch,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Tabby RS");
}
