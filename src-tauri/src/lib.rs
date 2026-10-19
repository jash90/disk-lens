pub mod actions;
pub mod category;
pub mod listing;
mod commands;
pub mod model;
pub mod persist;
pub mod platform;
pub mod query;
pub mod scanner;
pub mod treemap;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Most MCP do testów UI – tylko z funkcją `mcp-bridge` (pnpm build:debug) i tylko na localhost.
    #[cfg(feature = "mcp-bridge")]
    let builder = builder
        .plugin(tauri_plugin_mcp_bridge::Builder::new().bind_address("127.0.0.1").build())
        .setup(|app| {
            use tauri::Manager;
            app.add_capability(
                r#"{"identifier":"mcp-bridge","windows":["main"],"permissions":["mcp-bridge:default"]}"#,
            )?;
            Ok(())
        });
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::platform,
            commands::launch_path,
            commands::list_volumes,
            commands::scan_start,
            commands::scan_cancel,
            commands::tree_info,
            commands::get_node,
            commands::get_ancestors,
            commands::get_children,
            commands::treemap_layout,
            commands::top_files,
            commands::type_breakdown,
            commands::skipped_paths,
            commands::reveal,
            commands::open_terminal,
            commands::copy_path,
            commands::open_privacy_settings,
            commands::trash_check,
            commands::move_to_trash,
            commands::rescan_node,
            commands::export_view,
            commands::save_scan,
            commands::load_scan,
            commands::diff_scans,
        ])
        .run(tauri::generate_context!())
        .expect("błąd uruchamiania aplikacji");
}
