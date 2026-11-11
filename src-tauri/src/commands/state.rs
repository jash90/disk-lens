//! State shared by all commands: the current tree, the running scan and per-view caches.

use super::Res;
use crate::model::{SizeMode, Tree};
use crate::query::{compute_filtered, Filter, FilteredSums, View};
use crate::scanner::ScanControl;
use crate::volume::VolumeInfo;
use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

pub(super) type SharedTree = Arc<RwLock<Tree>>;

/// Filtered totals valid for one filter, scan and tree generation.
struct CachedSums {
    key: u64,
    seq: u64,
    generation: u64,
    sums: Arc<FilteredSums>,
}

#[derive(Default)]
pub struct AppState {
    /// (scan number, tree). Every scan has its own tree, so a cancelled scan cannot damage a new one.
    pub(super) tree: RwLock<Option<(u64, SharedTree)>>,
    pub(super) scan: Mutex<Option<Arc<ScanControl>>>,
    scan_seq: AtomicU64,
    cache: Mutex<Option<CachedSums>>,
    /// Last volume data (scan root, data). During a scan `tree_info` is called every 250 ms and
    /// reading purgeable space on macOS can be slow, so this copy is used then.
    pub(super) volume: Mutex<Option<(PathBuf, Option<VolumeInfo>)>>,
}

impl AppState {
    pub(super) fn current(&self) -> Res<(u64, SharedTree)> {
        self.tree.read().clone().ok_or_else(|| "Najpierw wybierz folder do skanowania.".to_string())
    }

    pub(crate) fn scanning(&self) -> bool {
        self.scan.lock().as_ref().is_some_and(|c| !c.done.load(Relaxed))
    }

    pub(super) fn ensure_idle(&self) -> Res<()> {
        if self.scanning() {
            Err("Poczekaj na koniec skanu albo go anuluj.".into())
        } else {
            Ok(())
        }
    }

    /// Asks the running scan (if any) to stop and forgets it.
    pub(super) fn cancel_running_scan(&self) {
        if let Some(old) = self.scan.lock().take() {
            old.cancel.store(true, Relaxed);
        }
    }

    pub(super) fn set_tree(&self, tree: Tree) -> (u64, SharedTree) {
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

    /// Runs `f` on the current tree under its read lock.
    pub(super) fn with_tree<T>(&self, f: impl FnOnce(&Tree) -> Res<T>) -> Res<T> {
        let (_, shared) = self.current()?;
        let tree = shared.read();
        f(&tree)
    }

    /// Like [`Self::with_tree`], after checking that `id` is a live node.
    pub(super) fn with_node<T>(&self, id: u32, f: impl FnOnce(&Tree) -> Res<T>) -> Res<T> {
        self.with_tree(|t| {
            check_id(t, id)?;
            f(t)
        })
    }

    /// Runs `f` on a view of the current tree (under its read lock), after checking that `id`
    /// is a live node.
    pub(super) fn with_view<T>(&self, args: &ViewArgs, id: u32, f: impl FnOnce(&View) -> Res<T>) -> Res<T> {
        let (seq, shared) = self.current()?;
        let tree = shared.read();
        check_id(&tree, id)?;
        let sums = self.sums(seq, &tree, args);
        f(&View::new(&tree, args.mode, sums))
    }
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewArgs {
    pub mode: SizeMode,
    pub filter: Filter,
}

pub(super) fn check_id(tree: &Tree, id: u32) -> Res<()> {
    tree.get(id).map(|_| ()).ok_or_else(|| "Element nie istnieje (mógł zostać usunięty lub przeskanowany ponownie).".into())
}
