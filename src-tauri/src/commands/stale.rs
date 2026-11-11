//! The "stale files" tab: lists of things not used for a long time and the age histogram.

use super::state::ViewArgs;
use super::{AppState, Res};
use crate::platform::now_secs;
use crate::stale::{self, AgeHistogram, StaleArgs, StaleRow};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StalePage {
    total: usize,
    total_size: u64,
    rows: Vec<StaleRow>,
}

#[tauri::command]
pub async fn stale_list(
    state: State<'_, AppState>,
    root: u32,
    view: ViewArgs,
    args: StaleArgs,
    offset: usize,
    limit: usize,
) -> Res<StalePage> {
    state.with_stale(&view, root, &args, |v, found| {
        Ok(StalePage {
            total: found.matches.len(),
            total_size: found.total_size,
            rows: found.matches.iter().skip(offset).take(limit).map(|m| stale::row(v.tree, root, m)).collect(),
        })
    })
}

#[tauri::command]
pub async fn age_histogram(state: State<'_, AppState>, root: u32, view: ViewArgs) -> Res<AgeHistogram> {
    state.with_view(&view, root, |v| Ok(stale::age_histogram(v, root, now_secs())))
}
