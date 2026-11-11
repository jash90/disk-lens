//! Actions on a single node: reveal, terminal, clipboard, Trash.

use super::state::check_id;
use super::{AppState, Res};
use crate::actions;
use crate::model::{Tree, ROOT};
use serde::Serialize;
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

/// Why `id` must not be moved to the Trash (scan root, system path…), if it must not.
fn trash_block_reason(tree: &Tree, id: u32) -> Option<String> {
    actions::protection_reason(&tree.path(id), &tree.root_path, actions::home_dir().as_deref(), actions::RULES)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashCheck {
    path: String,
    alloc: u64,
    files: u32,
    blocked: Option<String>,
}

/// Data for the confirmation dialog (path, size, the reason it is blocked, if any).
#[tauri::command]
pub async fn trash_check(state: State<'_, AppState>, id: u32) -> Res<TrashCheck> {
    state.with_node(id, |t| {
        let n = t.node(id);
        Ok(TrashCheck {
            path: t.path(id).to_string_lossy().into_owned(),
            alloc: n.alloc,
            files: n.files + u32::from(!n.kind.is_dir()),
            blocked: trash_block_reason(t, id),
        })
    })
}

/// Moves the item to the Trash (never deletes permanently) and subtracts it from the tree.
/// Returns the freed bytes.
#[tauri::command]
pub async fn move_to_trash(state: State<'_, AppState>, id: u32) -> Res<u64> {
    state.ensure_idle()?;
    let (_, shared) = state.current()?;
    let (path, alloc) = {
        let t = shared.read();
        check_id(&t, id)?;
        if id == ROOT {
            return Err("Nie można przenieść do Kosza korzenia skanu.".into());
        }
        if let Some(reason) = trash_block_reason(&t, id) {
            return Err(format!("Zablokowane: {reason}"));
        }
        (t.path(id), t.node(id).alloc)
    };
    actions::move_to_trash(&path).map_err(|e| format!("Nie udało się przenieść do Kosza: {e}"))?;
    shared.write().detach(id);
    Ok(alloc)
}
