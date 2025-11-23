#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod state;

use state::AppState;

fn main() {
    tauri::Builder::default()
        .manage(AppState::new())
        .setup(|_app| Ok(()))
        .invoke_handler(tauri::generate_handler![
            commands::list_entries,
            commands::stat_entry,
            commands::read_file,
            commands::write_file,
            commands::delete_path,
            commands::list_sources,
            commands::add_source,
            commands::remove_source,
            commands::update_source,
            commands::replace_sources,
            commands::upload_dropped_files,
            commands::list_storage_schemas,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
