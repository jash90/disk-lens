//! Read-only queries on the current tree.

use super::state::ViewArgs;
use super::{AppState, Res};
use crate::model::Skipped;
use crate::query::{self, SortKey};
use crate::{platform, treemap};
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub async fn get_node(state: State<'_, AppState>, id: u32, view: ViewArgs) -> Res<query::NodeInfo> {
    state.with_view(&view, id, |v| Ok(query::node_info(v, id)))
}

#[tauri::command]
pub async fn get_ancestors(state: State<'_, AppState>, id: u32) -> Res<Vec<query::Crumb>> {
    state.with_node(id, |t| Ok(query::crumbs(t, id)))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_children(
    state: State<'_, AppState>,
    id: u32,
    view: ViewArgs,
    sort: SortKey,
    desc: bool,
    offset: usize,
    limit: usize,
) -> Res<query::Page> {
    state.with_view(&view, id, |v| Ok(query::children(v, id, sort, desc, offset, limit.min(5000))))
}

#[tauri::command]
pub async fn treemap_layout(
    state: State<'_, AppState>,
    root: u32,
    view: ViewArgs,
    width: f64,
    height: f64,
) -> Res<tauri::ipc::Response> {
    let opts = treemap::Options {
        width: width.max(1.0),
        height: height.max(1.0),
        min_area: 1.0,
        max_tiles: 50_000,
        padding: 1.0,
        now: platform::now_secs(),
    };
    let bytes = state.with_view(&view, root, |v| Ok(treemap::encode(&treemap::layout(v, root, &opts))))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn top_files(state: State<'_, AppState>, root: u32, view: ViewArgs, limit: usize) -> Res<Vec<query::FileRow>> {
    state.with_view(&view, root, |v| Ok(query::top_files(v, root, limit.min(1000))))
}

#[tauri::command]
pub async fn type_breakdown(state: State<'_, AppState>, root: u32, view: ViewArgs) -> Res<query::Breakdown> {
    state.with_view(&view, root, |v| Ok(query::type_breakdown(v, root, 40)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedList {
    total: usize,
    items: Vec<Skipped>,
}

#[tauri::command]
pub async fn skipped_paths(state: State<'_, AppState>, limit: usize) -> Res<SkippedList> {
    state.with_tree(|t| Ok(SkippedList { total: t.skipped.len(), items: t.skipped.iter().take(limit).cloned().collect() }))
}
