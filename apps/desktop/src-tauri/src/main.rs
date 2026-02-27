#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod state;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;

fn main() {
    tauri::Builder::default()
        .manage(state::AppState::new())
        .setup(|app| {
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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_entries,
            commands::stat_entry,
            commands::read_file,
            commands::write_file,
            commands::create_directory,
            commands::delete_path,
            commands::list_sources,
            commands::add_source,
            commands::remove_source,
            commands::update_source,
            commands::verify_source,
            commands::replace_sources,
            commands::upload_dropped_files,
            commands::transfer_entries,
            commands::list_storage_schemas,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
