//! Cienka warstwa komend Tauri nad modułami skanera i zapytań.
//!
//! Komendy są `async`, żeby cięższe zapytania (filtry, treemapa) nie blokowały głównego wątku okna.

use crate::actions;
use crate::model::{SizeMode, Skipped, Tree, ROOT};
use crate::persist;
use crate::query::{self, compute_filtered, Filter, FilteredSums, SortKey, View};
use crate::scanner::{self, ScanControl, ScanOptions};
use crate::treemap;
use crate::volume::{self, Coverage, VolumeInfo};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

type Res<T> = Result<T, String>;
type SharedTree = Arc<RwLock<Tree>>;

/// Przefiltrowane sumy ważne dla danego filtra, skanu i generacji drzewa.
struct CachedSums {
    key: u64,
    seq: u64,
    generation: u64,
    sums: Arc<FilteredSums>,
}

#[derive(Default)]
pub struct AppState {
    /// (numer skanu, drzewo). Każdy skan ma własne drzewo, więc anulowany skan nie psuje nowego.
    tree: RwLock<Option<(u64, SharedTree)>>,
    scan: Mutex<Option<Arc<ScanControl>>>,
    scan_seq: AtomicU64,
    cache: Mutex<Option<CachedSums>>,
    /// Ostatnie dane wolumenu (korzeń skanu, dane) – w trakcie skanu `tree_info` woła się co 250 ms,
    /// a odczyt miejsca do odzyskania na macOS bywa wolny, więc wtedy używamy tej kopii.
    volume: Mutex<Option<(PathBuf, Option<VolumeInfo>)>>,
}

impl AppState {
    fn current(&self) -> Res<(u64, SharedTree)> {
        self.tree.read().clone().ok_or_else(|| "Najpierw wybierz folder do skanowania.".to_string())
    }

    fn scanning(&self) -> bool {
        self.scan.lock().as_ref().is_some_and(|c| !c.done.load(Relaxed))
    }

    fn ensure_idle(&self) -> Res<()> {
        if self.scanning() {
            Err("Poczekaj na koniec skanu albo go anuluj.".into())
        } else {
            Ok(())
        }
    }

    fn set_tree(&self, tree: Tree) -> (u64, SharedTree) {
        let seq = self.scan_seq.fetch_add(1, Relaxed) + 1;
        let shared = Arc::new(RwLock::new(tree));
        *self.tree.write() = Some((seq, shared.clone()));
        *self.cache.lock() = None;
        (seq, shared)
    }

    fn sums(&self, seq: u64, tree: &Tree, args: &ViewArgs) -> Option<Arc<FilteredSums>> {
        if !args.filter.is_active() {
            return None;
        }
        let key = args.filter.key(args.mode);
        let mut cache = self.cache.lock();
        if let Some(c) = cache.as_ref() {
            if c.key == key && c.seq == seq && c.generation == tree.generation {
                return Some(c.sums.clone());
            }
        }
        let sums = Arc::new(compute_filtered(tree, &args.filter, args.mode));
        *cache = Some(CachedSums { key, seq, generation: tree.generation, sums: sums.clone() });
        Some(sums)
    }
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewArgs {
    pub mode: SizeMode,
    pub filter: Filter,
}

/// Wykonuje `f` na widoku bieżącego drzewa (pod read-lockiem).
fn with_view<T>(state: &AppState, args: &ViewArgs, f: impl FnOnce(&View) -> Res<T>) -> Res<T> {
    let (seq, shared) = state.current()?;
    let tree = shared.read();
    let sums = state.sums(seq, &tree, args);
    let view = View::new(&tree, args.mode, sums);
    f(&view)
}

fn check_id(tree: &Tree, id: u32) -> Res<()> {
    tree.get(id).map(|_| ()).ok_or_else(|| "Element nie istnieje (mógł zostać usunięty lub przeskanowany ponownie).".into())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    scan_id: u64,
    files: u64,
    dirs: u64,
    bytes: u64,
    errors: u64,
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
        errors: p.errors.load(Relaxed),
        current: p.current.lock().clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        done,
        cancelled: ctl.cancelled(),
    }
}

/// System operacyjny („macos”, „windows”, „linux”) – UI dobiera etykiety i skróty.
#[tauri::command]
pub fn platform() -> &'static str {
    std::env::consts::OS
}

/// Folder podany w argumentach uruchomienia (`Disk Lens <folder>`), jeśli istnieje.
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
    if let Some(old) = state.scan.lock().take() {
        old.cancel.store(true, Relaxed);
    }
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
    nodes: usize,
    /// Wolumen korzenia skanu (`None`, gdy ścieżki już nie ma, np. wczytany skan z innego dysku).
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
        nodes: t.len(),
        coverage: vol.as_ref().map(|v| volume::coverage(v, root.alloc)),
        volume: vol,
    }))
}

#[tauri::command]
pub async fn get_node(state: State<'_, AppState>, id: u32, view: ViewArgs) -> Res<query::NodeInfo> {
    with_view(&state, &view, |v| {
        check_id(v.tree, id)?;
        Ok(query::node_info(v, id))
    })
}

#[tauri::command]
pub async fn get_ancestors(state: State<'_, AppState>, id: u32) -> Res<Vec<query::Crumb>> {
    let (_, shared) = state.current()?;
    let t = shared.read();
    check_id(&t, id)?;
    Ok(query::crumbs(&t, id))
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
    with_view(&state, &view, |v| {
        check_id(v.tree, id)?;
        Ok(query::children(v, id, sort, desc, offset, limit.min(5000)))
    })
}

#[tauri::command]
pub async fn treemap_layout(
    state: State<'_, AppState>,
    root: u32,
    view: ViewArgs,
    width: f64,
    height: f64,
) -> Res<tauri::ipc::Response> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let bytes = with_view(&state, &view, |v| {
        check_id(v.tree, root)?;
        let opts = treemap::Options {
            width: width.max(1.0),
            height: height.max(1.0),
            min_area: 1.0,
            max_tiles: 50_000,
            padding: 1.0,
            now,
        };
        Ok(treemap::encode(&treemap::layout(v, root, &opts)))
    })?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn top_files(state: State<'_, AppState>, root: u32, view: ViewArgs, limit: usize) -> Res<Vec<query::FileRow>> {
    with_view(&state, &view, |v| {
        check_id(v.tree, root)?;
        Ok(query::top_files(v, root, limit.min(1000)))
    })
}

#[tauri::command]
pub async fn type_breakdown(state: State<'_, AppState>, root: u32, view: ViewArgs) -> Res<query::Breakdown> {
    with_view(&state, &view, |v| {
        check_id(v.tree, root)?;
        Ok(query::type_breakdown(v, root, 40))
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedList {
    total: usize,
    items: Vec<Skipped>,
}

#[tauri::command]
pub async fn skipped_paths(state: State<'_, AppState>, limit: usize) -> Res<SkippedList> {
    let (_, shared) = state.current()?;
    let t = shared.read();
    Ok(SkippedList { total: t.skipped.len(), items: t.skipped.iter().take(limit).cloned().collect() })
}

fn path_of(state: &AppState, id: u32) -> Res<PathBuf> {
    let (_, shared) = state.current()?;
    let t = shared.read();
    check_id(&t, id)?;
    Ok(t.path(id))
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

/// Otwiera Ustawienia → Prywatność → Pełny dostęp do dysku.
#[tauri::command]
pub async fn open_privacy_settings() -> Res<()> {
    actions::open_privacy_settings()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashCheck {
    path: String,
    alloc: u64,
    logical: u64,
    files: u32,
    blocked: Option<String>,
}

/// Dane do okna potwierdzenia (ścieżka, rozmiar, ewentualna blokada).
#[tauri::command]
pub async fn trash_check(state: State<'_, AppState>, id: u32) -> Res<TrashCheck> {
    let (_, shared) = state.current()?;
    let t = shared.read();
    check_id(&t, id)?;
    let p = t.path(id);
    let n = t.node(id);
    let blocked = actions::protection_reason(&p, &t.root_path, actions::home_dir().as_deref(), actions::RULES)
        .or_else(|| (id == ROOT).then(|| "To korzeń bieżącego skanu.".into()));
    Ok(TrashCheck {
        path: p.to_string_lossy().into_owned(),
        alloc: n.alloc,
        logical: n.logical,
        files: n.files + u32::from(!n.kind.is_dir()),
        blocked,
    })
}

/// Przenosi element do Kosza (nigdy nie usuwa trwale) i odejmuje go z drzewa. Zwraca zwolnione bajty.
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
        let p = t.path(id);
        if let Some(reason) = actions::protection_reason(&p, &t.root_path, actions::home_dir().as_deref(), actions::RULES) {
            return Err(format!("Zablokowane: {reason}"));
        }
        (p, t.node(id).alloc)
    };
    actions::move_to_trash(&path).map_err(|e| format!("Nie udało się przenieść do Kosza: {e}"))?;
    shared.write().detach(id);
    Ok(alloc)
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
) -> Res<usize> {
    with_view(&state, &view, |v| {
        check_id(v.tree, root)?;
        let rows = query::children(v, root, sort, desc, 0, usize::MAX).rows;
        persist::export(v, root, &rows, &format, &PathBuf::from(&path))
    })
}

#[tauri::command]
pub async fn save_scan(state: State<'_, AppState>, path: String) -> Res<()> {
    state.ensure_idle()?;
    let (_, shared) = state.current()?;
    let t = shared.read();
    persist::save(&t, &PathBuf::from(path))
}

#[tauri::command]
pub async fn load_scan(state: State<'_, AppState>, path: String) -> Res<u64> {
    if let Some(old) = state.scan.lock().take() {
        old.cancel.store(true, Relaxed);
    }
    let tree = persist::load(&PathBuf::from(path))?;
    Ok(state.set_tree(tree).0)
}

#[tauri::command]
pub async fn diff_scans(state: State<'_, AppState>, path: String, mode: SizeMode) -> Res<persist::DiffResult> {
    state.ensure_idle()?;
    let before = persist::load(&PathBuf::from(path))?;
    let (_, shared) = state.current()?;
    let after = shared.read();
    Ok(persist::diff(&before, &after, mode, 1000))
}
