//! State shared by all commands: the current tree, the running scan and per-view caches.

use super::Res;
use crate::actions::{self, Protected};
use crate::model::{SizeMode, Tree};
use crate::query::{compute_filtered, Filter, FilteredSums, View};
use crate::scanner::ScanControl;
use crate::stale::{self, StaleArgs, StaleIndex, StaleResult};
use crate::volume::VolumeInfo;
use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use std::hash::{Hash, Hasher};
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

/// Newest-activity index valid for one scan and tree generation.
struct CachedIndex {
    use_access: bool,
    show_apps: bool,
    seq: u64,
    generation: u64,
    index: Arc<StaleIndex>,
}

/// One sorted stale list, so that paging and re-sorting do not walk the tree again.
struct CachedStale {
    key: u64,
    seq: u64,
    generation: u64,
    result: Arc<StaleResult>,
}

/// Stale lists are cached for this long; the cutoff moves with the clock, but only by days.
const STALE_CACHE_SECS: i64 = 600;

#[derive(Default)]
pub struct AppState {
    /// (scan number, tree). Every scan has its own tree, so a cancelled scan cannot damage a new one.
    pub(super) tree: RwLock<Option<(u64, SharedTree)>>,
    pub(super) scan: Mutex<Option<Arc<ScanControl>>>,
    scan_seq: AtomicU64,
    cache: Mutex<Option<CachedSums>>,
    stale_index: Mutex<Option<CachedIndex>>,
    stale_list: Mutex<Option<CachedStale>>,
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
        *self.stale_index.lock() = None;
        *self.stale_list.lock() = None;
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

    fn stale_index(&self, seq: u64, tree: &Tree, use_access: bool, show_apps: bool) -> Arc<StaleIndex> {
        let mut cache = self.stale_index.lock();
        if let Some(c) = cache.as_ref() {
            if c.use_access == use_access && c.show_apps == show_apps && c.seq == seq && c.generation == tree.generation {
                return c.index.clone();
            }
        }
        let index = Arc::new(StaleIndex::compute(tree, use_access, show_apps));
        *cache = Some(CachedIndex { use_access, show_apps, seq, generation: tree.generation, index: index.clone() });
        index
    }

    /// Runs `f` on the stale list for `root` (sorted, cached) and the view it was made from.
    pub(super) fn with_stale<T>(
        &self,
        args: &ViewArgs,
        root: u32,
        stale_args: &StaleArgs,
        f: impl FnOnce(&View, &StaleResult) -> Res<T>,
    ) -> Res<T> {
        let (seq, shared) = self.current()?;
        let tree = shared.read();
        check_id(&tree, root)?;
        let view = View::new(&tree, args.mode, self.sums(seq, &tree, args));
        let now = crate::platform::now_secs();
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (stale_args.key(), args.filter.key(args.mode), root, now / STALE_CACHE_SECS).hash(&mut h);
            h.finish()
        };
        let cached = self.stale_list.lock().as_ref().filter(|c| c.key == key && c.seq == seq && c.generation == tree.generation).map(|c| c.result.clone());
        let result = match cached {
            Some(r) => r,
            None => {
                let index = self.stale_index(seq, &tree, stale_args.use_access, stale_args.show_apps);
                let protected = Protected::resolve(&tree, actions::home_dir().as_deref(), actions::RULES);
                let r = Arc::new(stale::find(&view, root, stale_args, &index, &protected, now));
                *self.stale_list.lock() = Some(CachedStale { key, seq, generation: tree.generation, result: r.clone() });
                r
            }
        };
        f(&view, &result)
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
    if tree.is_attached(id) {
        Ok(())
    } else {
        Err("Element nie istnieje (mógł zostać usunięty lub przeskanowany ponownie).".into())
    }
}
