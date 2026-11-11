//! Scan lifecycle: start, cancel, progress events, rescan and the summary of the current tree.

use super::state::{check_id, SharedTree};
use super::{AppState, Res};
use crate::model::ROOT;
use crate::refresh::{self, RefreshStats};
use crate::scanner::{self, ScanControl, ScanOptions};
use crate::volume::{self, Coverage, VolumeInfo};
use crate::{cache, fsevents};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

/// Reading the event log is given this long before a full scan is the better deal.
const EVENT_LOG_TIMEOUT: Duration = Duration::from_secs(20);
/// After a refresh the kept scan is written again when it is older than this…
const REWRITE_AFTER: Duration = Duration::from_secs(30 * 60);
/// …or when the refresh looked at this many changed folders.
const REWRITE_AFTER_FOLDERS: u64 = 20_000;

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
    /// The run only checks what changed since the last scan (counters then cover new items only).
    refresh: bool,
    /// Folders a finished refresh looked at.
    checked: u64,
}

fn progress_event(scan_id: u64, ctl: &ScanControl, started: Instant, done: bool, refresh: bool, checked: u64) -> ProgressEvent {
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
        refresh,
        checked,
    }
}

fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|dir| dir.join("scans"))
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

/// Replaces the content of the shared tree, keeping its generation counter moving forward.
fn swap_tree(shared: &SharedTree, mut tree: crate::model::Tree) {
    let mut current = shared.write();
    tree.generation = current.generation + 1;
    *current = tree;
}

/// Brings the tree up to date from the event log, starting from the scan kept in the cache when
/// the tree is still empty. `None`: not possible, a full scan has to do it.
fn try_refresh(shared: &SharedTree, root: &Path, opts: &ScanOptions, ctl: &ScanControl, have_tree: bool, cache_dir: Option<&Path>) -> Option<RefreshStats> {
    // Taken before anything is read: whatever changes from here on is seen by the next refresh.
    let now = fsevents::position(root)?;
    if !have_tree {
        let cached = cache::load(cache_dir?, root).filter(|t| t.state.event_id != 0)?;
        swap_tree(shared, cached);
    }
    let since = {
        let t = shared.read();
        fsevents::Position { event_id: t.state.event_id, uuid: t.state.event_uuid.clone() }
    };
    let changes = fsevents::changes_since(root, &since, EVENT_LOG_TIMEOUT, &|| ctl.cancelled()).ok()?;
    let stats = refresh::refresh(shared, &changes, opts, ctl);
    if !ctl.cancelled() {
        let mut t = shared.write();
        (t.state.event_id, t.state.event_uuid) = (now.event_id, now.uuid);
    }
    Some(stats)
}

/// Starts a scan of `path`. When the folder was scanned before and the system's event log
/// reaches back that far (macOS), only the folders that changed since are read again; `full`
/// asks for a scan from scratch regardless.
#[tauri::command]
pub async fn scan_start(app: AppHandle, state: State<'_, AppState>, path: String, cross_volumes: bool, full: Option<bool>) -> Res<u64> {
    state.cancel_running_scan();
    let root = PathBuf::from(&path);
    let empty = scanner::new_tree(&root).map_err(|e| format!("Nie można otworzyć {path}: {e}"))?;
    // Other volumes have event logs of their own, so a scan that enters them is always a full one.
    let incremental = !full.unwrap_or(false) && !cross_volumes;
    let in_memory = state.tree.read().clone().map(|(_, shared)| shared).filter(|shared| {
        let t = shared.read();
        incremental && t.root_path == root && t.state.event_id != 0
    });
    let have_tree = in_memory.is_some();
    let (seq, shared) = match in_memory {
        Some(shared) => (state.adopt_tree(shared.clone()), shared),
        None => state.set_tree(empty),
    };
    let ctl = Arc::new(ScanControl::default());
    *state.scan.lock() = Some(ctl.clone());
    let opts = ScanOptions { cross_volumes, ..Default::default() };
    let run_lock = state.run_lock.clone();
    let cache_dir = cache_dir(&app);

    std::thread::Builder::new()
        .name("disk-lens-scan".into())
        .spawn(move || {
            let started = Instant::now();
            let refreshing = Arc::new(AtomicBool::new(incremental));
            let ticker = {
                let (ctl, app, refreshing) = (ctl.clone(), app.clone(), refreshing.clone());
                std::thread::spawn(move || {
                    while !ctl.done.load(Relaxed) {
                        std::thread::sleep(Duration::from_millis(250));
                        if ctl.done.load(Relaxed) {
                            break;
                        }
                        let _ = app.emit("scan://progress", progress_event(seq, &ctl, started, false, refreshing.load(Relaxed), 0));
                    }
                })
            };
            let running = run_lock.lock();
            let mut refreshed = None;
            if incremental && !ctl.cancelled() {
                refreshed = try_refresh(&shared, &root, &opts, &ctl, have_tree, cache_dir.as_deref());
            }
            if refreshed.is_none() && !ctl.cancelled() {
                refreshing.store(false, Relaxed);
                // Whatever the failed attempt left behind is not a base for a scan.
                if let Ok(empty) = scanner::new_tree(&root) {
                    swap_tree(&shared, empty);
                }
                // A scan that entered other volumes cannot be refreshed from one volume's log.
                let position = fsevents::position(&root).filter(|_| !cross_volumes).unwrap_or_default();
                scanner::scan(&shared, ROOT, &opts, &ctl);
                if !ctl.cancelled() {
                    let mut t = shared.write();
                    (t.state.event_id, t.state.event_uuid, t.state.refreshed_at) = (position.event_id, position.uuid, 0);
                }
            }
            ctl.done.store(true, Relaxed);
            drop(running);
            let _ = ticker.join();
            let checked = refreshed.as_ref().map_or(0, |s| (s.listed + s.rescanned) as u64);
            let _ = app.emit("scan://progress", progress_event(seq, &ctl, started, true, refreshed.is_some(), checked));

            // Kept for the next start; the window is usable while this is written. Writing a
            // whole disk takes seconds during which the tree cannot change, so after a refresh
            // the kept scan is replaced only when it has fallen well behind: the event log
            // bridges the gap next time.
            let worth_keeping = match &refreshed {
                None => true,
                Some(_) => checked >= REWRITE_AFTER_FOLDERS || cache_dir.as_deref().and_then(|d| cache::age(d, &root)).is_none_or(|age| age >= REWRITE_AFTER),
            };
            if let Some(dir) = cache_dir.filter(|_| worth_keeping && !ctl.cancelled() && !cross_volumes) {
                let t = shared.read();
                if t.len() >= cache::MIN_NODES {
                    let _ = cache::store(&dir, &t);
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(seq)
}

/// Shows the most recent kept scan when the app starts with nothing to show. Returns its scan
/// number, or `None` when there is no kept scan (or a tree is there already).
#[tauri::command]
pub async fn restore_last_scan(app: AppHandle, state: State<'_, AppState>) -> Res<Option<u64>> {
    let Some(dir) = cache_dir(&app).filter(|_| state.tree.read().is_none()) else { return Ok(None) };
    let tree = tauri::async_runtime::spawn_blocking(move || cache::latest(&dir)).await.map_err(|e| e.to_string())?;
    // A scan may have been started while the file was being read.
    let Some(tree) = tree.filter(|_| state.tree.read().is_none()) else { return Ok(None) };
    let (seq, _) = state.set_tree(tree);
    state.restored.store(seq, Relaxed);
    Ok(Some(seq))
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
    /// When the tree was last brought up to date from the event log; 0 after a full scan.
    refreshed_at: i64,
    /// The tree comes from the app's cache and was not checked against the disk in this session.
    restored: bool,
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
        refreshed_at: t.state.refreshed_at,
        restored: state.restored.load(Relaxed) == seq,
        coverage: vol.as_ref().map(|v| volume::coverage(v, root.alloc)),
        volume: vol,
    }))
}

#[tauri::command]
pub async fn rescan_node(state: State<'_, AppState>, id: u32) -> Res<()> {
    state.ensure_idle()?;
    let (_, shared) = state.current()?;
    check_id(&shared.read(), id)?;
    // Registered like a full scan, so that Trash, save and compare wait for it too.
    let ctl = Arc::new(ScanControl::default());
    *state.scan.lock() = Some(ctl.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let result = scanner::rescan(&shared, id, &ScanOptions::default(), &ctl).map_err(|e| e.to_string());
        ctl.done.store(true, Relaxed);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
