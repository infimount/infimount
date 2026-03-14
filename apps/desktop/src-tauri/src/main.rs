#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod state;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

fn main() {
    let app_state = state::AppState::new().expect("failed to initialize desktop state");

    tauri::Builder::default()
        .manage(app_state)
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
                    if let Err(error) = app_state.ensure_runtime_from_settings().await {
                        eprintln!("failed to initialize MCP runtime: {}", error.message);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_entries,
            commands::stat_entry,
            commands::read_file,
            commands::write_file,
            commands::create_directory,
            commands::delete_path,
            commands::list_storages,
            commands::add_storage,
            commands::remove_storage,
            commands::update_storage,
            commands::verify_storage,
            commands::import_storage_config,
            commands::export_storage_config,
            commands::upload_dropped_files,
            commands::transfer_entries,
            commands::list_storage_schemas,
            commands::get_mcp_settings,
            commands::list_mcp_tools,
            commands::update_mcp_settings,
            commands::get_mcp_status,
            commands::start_mcp_http,
            commands::stop_mcp_http,
            commands::get_mcp_client_snippets,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
