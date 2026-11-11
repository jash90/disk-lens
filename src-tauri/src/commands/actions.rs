//! Actions on nodes: reveal, terminal, clipboard, Trash.

use super::{AppState, Res};
use crate::actions;
use crate::trash_batch::{self, BatchCheck, BatchOutcome};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

fn path_of(state: &AppState, id: u32) -> Res<PathBuf> {
    state.with_node(id, |t| Ok(t.path(id)))
}

#[tauri::command]
pub async fn reveal(app: AppHandle, state: State<'_, AppState>, id: u32) -> Res<()> {
    let p = path_of(&state, id)?;
    app.opener().reveal_item_in_dir(&p).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_terminal(state: State<'_, AppState>, id: u32) -> Res<()> {
    let p = path_of(&state, id)?;
    let dir = if p.is_dir() { p } else { p.parent().map(PathBuf::from).unwrap_or(p) };
    actions::open_terminal(&dir)
}

#[tauri::command]
pub async fn copy_path(app: AppHandle, state: State<'_, AppState>, id: u32) -> Res<String> {
    let p = path_of(&state, id)?.to_string_lossy().into_owned();
    app.clipboard().write_text(p.clone()).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Opens Settings → Privacy → Full Disk Access.
#[tauri::command]
pub async fn open_privacy_settings() -> Res<()> {
    actions::open_privacy_settings()
}

/// Data for the confirmation dialog: how much would be moved and what is blocked.
#[tauri::command]
pub async fn trash_check_batch(state: State<'_, AppState>, ids: Vec<u32>) -> Res<BatchCheck> {
    check_len(&ids)?;
    state.with_tree(|t| Ok(trash_batch::check(t, &ids, actions::home_dir().as_deref(), actions::RULES)))
}

/// Moves the items to the Trash (never deletes permanently) and subtracts them from the tree.
/// With `verify`, items that changed on disk since the scan are left alone and reported.
#[tauri::command]
pub async fn move_to_trash_batch(state: State<'_, AppState>, ids: Vec<u32>, verify: bool) -> Res<BatchOutcome> {
    check_len(&ids)?;
    state.ensure_idle()?;
    let (_, shared) = state.current()?;
    tauri::async_runtime::spawn_blocking(move || {
        trash_batch::execute(&shared, &ids, verify, actions::home_dir().as_deref(), actions::RULES, actions::move_to_trash)
    })
    .await
    .map_err(|e| e.to_string())
}

fn check_len(ids: &[u32]) -> Res<()> {
    if ids.len() > trash_batch::MAX_ITEMS {
        return Err(format!("Za dużo elementów naraz (najwyżej {}).", trash_batch::MAX_ITEMS));
    }
    Ok(())
}
