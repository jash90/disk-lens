//! Scan lifecycle: start, cancel, progress events, rescan and the summary of the current tree.

use super::state::check_id;
use super::{AppState, Res};
use crate::model::ROOT;
use crate::scanner::{self, ScanControl, ScanOptions};
use crate::volume::{self, Coverage, VolumeInfo};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    scan_id: u64,
    files: u64,
    dirs: u64,
    bytes: u64,
    current: String,
    elapsed_ms: u64,
    done: bool,
    cancelled: bool,
}

fn progress_event(scan_id: u64, ctl: &ScanControl, started: Instant, done: bool) -> ProgressEvent {
    let p = &ctl.progress;
    ProgressEvent {
        scan_id,
        files: p.files.load(Relaxed),
        dirs: p.dirs.load(Relaxed),
        bytes: p.bytes.load(Relaxed),
        current: p.current.lock().clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        done,
        cancelled: ctl.cancelled(),
    }
}

/// Folder given on the command line (`Disk Lens <folder>`), if it exists.
#[tauri::command]
pub async fn launch_path() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-') && PathBuf::from(a).is_dir())
}

#[tauri::command]
pub async fn list_volumes() -> Vec<volume::Volume> {
    volume::list_volumes()
}

#[tauri::command]
pub async fn scan_start(app: AppHandle, state: State<'_, AppState>, path: String, cross_volumes: bool) -> Res<u64> {
    state.cancel_running_scan();
    let root = PathBuf::from(&path);
    let tree = scanner::new_tree(&root).map_err(|e| format!("Nie można otworzyć {path}: {e}"))?;
    let (seq, shared) = state.set_tree(tree);
    let ctl = Arc::new(ScanControl::default());
    *state.scan.lock() = Some(ctl.clone());
    let opts = ScanOptions { cross_volumes, ..Default::default() };

    std::thread::Builder::new()
        .name("disk-lens-scan".into())
        .spawn(move || {
            let started = Instant::now();
            let ticker = {
                let (ctl, app) = (ctl.clone(), app.clone());
                std::thread::spawn(move || {
                    while !ctl.done.load(Relaxed) {
                        std::thread::sleep(Duration::from_millis(250));
                        if ctl.done.load(Relaxed) {
                            break;
                        }
                        let _ = app.emit("scan://progress", progress_event(seq, &ctl, started, false));
                    }
                })
            };
            scanner::scan(&shared, ROOT, &opts, &ctl);
            ctl.done.store(true, Relaxed);
            let _ = ticker.join();
            let _ = app.emit("scan://progress", progress_event(seq, &ctl, started, true));
        })
        .map_err(|e| e.to_string())?;
    Ok(seq)
}

#[tauri::command]
pub async fn scan_cancel(state: State<'_, AppState>) -> Res<()> {
    if let Some(c) = state.scan.lock().as_ref() {
        c.cancel.store(true, Relaxed);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeInfo {
    scan_id: u64,
    root_path: String,
    scanning: bool,
    alloc: u64,
    logical: u64,
    files: u32,
    dirs: u32,
    skipped: usize,
    scanned_at: i64,
    /// Volume of the scan root (`None` when the path is gone, e.g. a loaded scan from another disk).
    volume: Option<VolumeInfo>,
    coverage: Option<Coverage>,
}

#[tauri::command]
pub async fn tree_info(state: State<'_, AppState>) -> Res<Option<TreeInfo>> {
    let Some((seq, shared)) = state.tree.read().clone() else { return Ok(None) };
    let scanning = state.scanning();
    let root_path = shared.read().root_path.clone();
    let vol = {
        let mut cached = state.volume.lock();
        match cached.as_ref() {
            Some((p, v)) if scanning && *p == root_path => v.clone(),
            _ => {
                let v = volume::volume_for(&root_path);
                *cached = Some((root_path, v.clone()));
                v
            }
        }
    };
    let t = shared.read();
    let root = t.node(ROOT);
    Ok(Some(TreeInfo {
        scan_id: seq,
        root_path: t.root_path.to_string_lossy().into_owned(),
        scanning,
        alloc: root.alloc,
        logical: root.logical,
        files: root.files,
        dirs: root.dirs,
        skipped: t.skipped.len(),
        scanned_at: t.scanned_at,
        coverage: vol.as_ref().map(|v| volume::coverage(v, root.alloc)),
        volume: vol,
    }))
}

#[tauri::command]
pub async fn rescan_node(state: State<'_, AppState>, id: u32) -> Res<()> {
    state.ensure_idle()?;
    let (_, shared) = state.current()?;
    check_id(&shared.read(), id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let ctl = ScanControl::default();
        scanner::rescan(&shared, id, &ScanOptions::default(), &ctl).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
