#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod state;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
#[cfg(target_os = "macos")]
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(state::AppState::new())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use objc2::runtime::AnyObject;
                use objc2::{class, msg_send};

                if let Some(main_window) = app.get_webview_window("main") {
                    // Ensure the native window background stays transparent so rounded webview
                    // edges don't show white patches in macOS corners.
                    main_window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))?;

                    // Get the NSWindow and apply corner radius masking
                    if let Ok(ns_window) = main_window.ns_window() {
                        unsafe {
                            let ns_win = ns_window as *mut AnyObject;
                            
                            // Set the window's background color to clear
                            let clear_color: *mut AnyObject = msg_send![class!(NSColor), clearColor];
                            let _: () = msg_send![ns_win, setBackgroundColor: clear_color];
                            
                            // Get the content view and apply corner radius
                            let content_view: *mut AnyObject = msg_send![ns_win, contentView];
                            if !content_view.is_null() {
                                // Ensure the view has a layer
                                let _: () = msg_send![content_view, setWantsLayer: true];
                                
                                let layer: *mut AnyObject = msg_send![content_view, layer];
                                if !layer.is_null() {
                                    let _: () = msg_send![layer, setCornerRadius: 12.0_f64];
                                    let _: () = msg_send![layer, setMasksToBounds: true];
                                }
                            }
                        }
                    }
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
            commands::delete_path,
            commands::list_sources,
            commands::add_source,
            commands::remove_source,
            commands::update_source,
            commands::replace_sources,
            commands::upload_dropped_files,
            commands::transfer_entries,
            commands::list_storage_schemas,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
