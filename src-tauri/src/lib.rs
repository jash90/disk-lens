pub mod actions;
pub mod category;
pub mod listing;
mod commands;
pub mod model;
pub mod persist;
pub mod platform;
pub mod query;
pub mod refresh;
pub mod scanner;
pub mod stale;
pub mod trash_batch;
pub mod treemap;
mod updater;
pub mod volume;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // MCP bridge for UI tests: only with the `mcp-bridge` feature (pnpm build:debug) and only on localhost.
    #[cfg(feature = "mcp-bridge")]
    let builder = builder.plugin(tauri_plugin_mcp_bridge::Builder::new().bind_address("127.0.0.1").build());
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(commands::AppState::default())
        .manage(updater::Updates::default())
        .invoke_handler(tauri::generate_handler![
            commands::scan::launch_path,
            commands::scan::list_volumes,
            commands::scan::scan_start,
            commands::scan::scan_cancel,
            commands::scan::tree_info,
            commands::query::get_node,
            commands::query::get_ancestors,
            commands::query::get_children,
            commands::query::treemap_layout,
            commands::query::top_files,
            commands::query::type_breakdown,
            commands::query::skipped_paths,
            commands::actions::reveal,
            commands::actions::open_terminal,
            commands::actions::copy_path,
            commands::actions::open_privacy_settings,
            commands::actions::trash_check_batch,
            commands::actions::move_to_trash_batch,
            commands::stale::stale_list,
            commands::stale::age_histogram,
            commands::scan::rescan_node,
            commands::persist::export_view,
            commands::persist::save_scan,
            commands::persist::load_scan,
            commands::persist::diff_scans,
            updater::check_update,
            updater::known_update,
            updater::install_update,
        ])
        // A single `setup`: another call would replace the previous one.
        .setup(|app| {
            #[cfg(feature = "mcp-bridge")]
            {
                use tauri::Manager;
                app.add_capability(
                    r#"{"identifier":"mcp-bridge","windows":["main"],"permissions":["mcp-bridge:default"]}"#,
                )?;
            }
            updater::check_on_startup(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start the application");
}
