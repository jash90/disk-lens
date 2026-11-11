//! Export of the current view, saved scans and the comparison of two scans.

use super::state::ViewArgs;
use super::{AppState, Res};
use crate::model::SizeMode;
use crate::persist;
use crate::query::{self, SortKey};
use crate::stale::{self, StaleArgs};
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn export_view(
    state: State<'_, AppState>,
    root: u32,
    view: ViewArgs,
    sort: SortKey,
    desc: bool,
    format: String,
    path: String,
    stale: Option<StaleArgs>,
) -> Res<usize> {
    let path = PathBuf::from(&path);
    if let Some(args) = stale {
        return state.with_stale(&view, root, &args, |v, found| {
            let rows: Vec<_> = found.matches.iter().map(|m| stale::row(v.tree, root, m)).collect();
            persist::export_stale(v, root, &rows, &format, &path)
        });
    }
    state.with_view(&view, root, |v| {
        let rows = query::children(v, root, sort, desc, 0, usize::MAX).rows;
        persist::export(v, root, &rows, &format, &path)
    })
}

#[tauri::command]
pub async fn save_scan(state: State<'_, AppState>, path: String) -> Res<()> {
    state.ensure_idle()?;
    state.with_tree(|t| persist::save(t, &PathBuf::from(path)))
}

#[tauri::command]
pub async fn load_scan(state: State<'_, AppState>, path: String) -> Res<u64> {
    state.cancel_running_scan();
    let tree = persist::load(&PathBuf::from(path))?;
    Ok(state.set_tree(tree).0)
}

#[tauri::command]
pub async fn diff_scans(state: State<'_, AppState>, path: String, mode: SizeMode) -> Res<persist::DiffResult> {
    state.ensure_idle()?;
    let before = persist::load(&PathBuf::from(path))?;
    state.with_tree(|after| Ok(persist::diff(&before, after, mode, 1000)))
}
