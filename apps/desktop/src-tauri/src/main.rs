#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod activation_probe;
mod app_settings;
mod client_integrations;
mod commands;
mod diagnostics;
mod state;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

fn main() {
    let app_state = match state::AppState::new() {
        Ok(s) => s,
        Err(_) => state::AppState::degraded("ERR_STARTUP_RESTORE_RECOVERY"),
    };

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;

                if let Some(main_window) = app.get_webview_window("main") {
                    main_window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))?;
                }
            }

            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let app_state = app_handle.state::<state::AppState>();
                    if !app_state.startup_health().operational {
                        return;
                    }
                    if let Ok(validation) = tauri::async_runtime::spawn_blocking(
                        activation_probe::validate_sidecar_binary,
                    )
                    .await
                    {
                        activation_probe::record_startup_sidecar_event(
                            &app_state.product_events,
                            &validation,
                        );
                    }
                    if let Err(error) = app_state.ensure_runtime_from_settings().await {
                        eprintln!("failed to initialize MCP runtime: {}", error.message);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_entries,
            commands::list_entries_recursive,
            commands::list_entries_page,
            commands::stat_entry,
            commands::read_file,
            commands::read_file_range,
            commands::download_file_to_downloads,
            commands::download_file_version_to_downloads,
            commands::write_file,
            commands::begin_file_upload,
            commands::append_file_upload_chunk,
            commands::finish_file_upload,
            commands::cancel_file_upload,
            commands::create_directory,
            commands::delete_path,
            commands::list_storages,
            commands::create_activation_demo_storage,
            commands::add_storage,
            commands::remove_storage,
            commands::update_storage,
            commands::update_mcp_storage_policy,
            commands::verify_storage,
            commands::export_shareable_config,
            commands::preview_storage_import_cmd,
            commands::apply_storage_import_cmd,
            commands::cancel_storage_import_preview_cmd,
            commands::zeroize_storage_import_previews_cmd,
            commands::upload_dropped_files,
            commands::plan_transfer_entries,
            commands::transfer_entries,
            commands::cancel_transfer_job,
            commands::list_storage_schemas,
            commands::get_storage_capabilities,
            commands::connect_oauth_storage,
            commands::cancel_oauth_storage,
            commands::generate_download_link,
            commands::get_app_settings,
            commands::complete_onboarding,
            commands::skip_onboarding,
            commands::list_mcp_audit_events,
            commands::clear_mcp_audit_events,
            commands::export_mcp_audit_bundle,
            commands::list_pending_mcp_confirmations,
            commands::list_active_mcp_sessions,
            commands::approve_mcp_confirmation,
            commands::deny_mcp_confirmation,
            commands::list_mcp_tools,
            commands::update_mcp_settings_with_auth,
            commands::get_mcp_status,
            commands::start_mcp_http,
            commands::stop_mcp_http,
            commands::get_mcp_client_snippets,
            client_integrations::list_mcp_client_adapters,
            client_integrations::preview_mcp_client_install,
            client_integrations::apply_mcp_client_install,
            client_integrations::rollback_mcp_client_install,
            commands::create_recovery_backup,
            commands::preview_recovery_restore,
            commands::apply_recovery_restore,
            commands::list_versions,
            commands::read_file_version,
            commands::delete_version,
            commands::get_mcp_sidecar_info,
            commands::list_workspaces,
            commands::create_workspace_atomic,
            commands::update_workspace,
            commands::delete_workspace,
            commands::delete_workspace_with_files,
            commands::list_workspace_checkpoints,
            commands::create_workspace_checkpoint,
            commands::restore_workspace_checkpoint,
            commands::save_wizard_state,
            commands::set_telemetry_consent,
            commands::export_diagnostics,
            commands::get_startup_health,
            commands::get_product_events,
            commands::clear_product_events,
            commands::reveal_diagnostics_export,
            commands::get_os_info,
            commands::run_activation_probe,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                commands::zeroize_storage_import_previews_cmd();
            }
        });
}
