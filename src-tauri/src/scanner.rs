//! Parallel file system scanner.
//!
//! Each `rayon` task handles one folder: it reads the entries with their metadata (`listing`),
//! inserts the children into the tree in one operation under the write lock (totals go up right
//! away, so results are visible during the scan) and hands the subfolders to further tasks.

use crate::category;
use crate::listing::{self, Backend, FileKind, RawEntry};
use crate::platform;
use crate::model::{extension_of, flags, Kind, NewEntry, Skipped, Tree};
use parking_lot::{Mutex, RwLock};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// Scan time counters (ns, summed over threads), collected only with the `scan-stats` feature.
pub mod stats {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    /// The whole folder read (open, entries, metadata).
    pub static LISTING: AtomicU64 = AtomicU64::new(0);
    /// Single `lstat` calls (standard and fallback path).
    pub static METADATA: AtomicU64 = AtomicU64::new(0);
    pub static LOCK_WAIT: AtomicU64 = AtomicU64::new(0);
    pub static LOCK_HOLD: AtomicU64 = AtomicU64::new(0);

    pub struct Timer {
        #[cfg(feature = "scan-stats")]
        t: std::time::Instant,
    }

    impl Timer {
        #[inline]
        pub fn start() -> Self {
            Timer {
                #[cfg(feature = "scan-stats")]
                t: std::time::Instant::now(),
            }
        }

        #[inline]
        pub fn stop(self, _counter: &AtomicU64) {
            #[cfg(feature = "scan-stats")]
            _counter.fetch_add(self.t.elapsed().as_nanos() as u64, Relaxed);
        }
    }

    pub fn reset() {
        for c in [&LISTING, &METADATA, &LOCK_WAIT, &LOCK_HOLD] {
            c.store(0, Relaxed);
        }
    }
}

/// Extensions of folders that macOS shows as a single file.
const PACKAGE_EXTS: &[&str] = &[
    "app", "photoslibrary", "bundle", "framework", "plugin", "kext", "appex", "xpc", "pkg",
    "mpkg", "rtfd", "sparsebundle", "logicx", "fcpbundle", "imovielibrary", "musiclibrary",
    "tvlibrary", "xcarchive", "xcodeproj", "xcworkspace", "playground", "band", "pages", "numbers",
    "key", "scptd",
];

#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Whether to descend into mount points of other volumes.
    pub cross_volumes: bool,
    pub threads: usize,
    /// Fast folder reads (`getattrlistbulk` on macOS) where the file system supports them.
    pub bulk: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        // A scan is mostly kernel time (reading APFS metadata). Above roughly cores + 2 threads
        // the kernel starts contending on locks: on 10 cores a scan of "/" took 86 s with 12
        // threads, 93 s with 20 and 141 s with 4 (measured 2026-10-19, `examples/scan --threads`).
        ScanOptions { cross_volumes: false, threads: (cores + 2).clamp(4, 16), bulk: true }
    }
}

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub current: Mutex<String>,
}

#[derive(Default)]
pub struct ScanControl {
    pub cancel: AtomicBool,
    pub done: AtomicBool,
    pub progress: Progress,
}

impl ScanControl {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }
}

fn is_package(name: &str) -> bool {
    PACKAGE_EXTS.contains(&extension_of(name).as_str())
}

fn entry_from_meta(name: String, meta: &Metadata) -> NewEntry {
    let raw = RawEntry::from_meta(OsStr::new(""), meta);
    entry_from_raw(name, &raw)
}

pub fn entry_from_raw(name: String, raw: &RawEntry) -> NewEntry {
    let kind = match raw.kind {
        FileKind::Dir if is_package(&name) => Kind::Package,
        FileKind::Dir => Kind::Dir,
        FileKind::Symlink => Kind::Symlink,
        FileKind::File => Kind::File,
        FileKind::Other => Kind::Other,
    };
    let mut fl = 0u8;
    if name.starts_with('.') {
        fl |= flags::HIDDEN;
    }
    if raw.dataless {
        fl |= flags::DATALESS;
    }
    let cat = category::of(&name, kind);
    NewEntry {
        name,
        kind,
        flags: fl,
        cat,
        alloc: raw.alloc,
        logical: raw.logical,
        mtime: raw.mtime,
        atime: raw.atime,
        created: raw.btime.max(raw.added),
    }
}

/// Creates a tree with only the root. The root must be a folder (a symlink to a folder is resolved).
pub fn new_tree(root: &Path) -> io::Result<Tree> {
    let meta = fs::metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Wybrana ścieżka nie jest folderem"));
    }
    let mut e = entry_from_meta(String::new(), &meta);
    e.kind = Kind::Dir;
    e.cat = category::FOLDER;
    Ok(Tree::new(root.to_path_buf(), e, platform::now_secs()))
}

struct Ctx<'a> {
    tree: &'a RwLock<Tree>,
    opts: &'a ScanOptions,
    ctl: &'a ScanControl,
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
    /// Devices the scan may descend on (`None` = a system without mount detection).
    allowed_devs: Option<Vec<u64>>,
    /// Paths that are not entered (e.g. /System/Volumes/Data when scanning "/").
    skip_paths: Vec<PathBuf>,
    /// How folders are read on each device met so far (usually 1–2 entries).
    backends: Mutex<Vec<(u64, Backend)>>,
}

impl Ctx<'_> {
    fn backend(&self, dev: Option<u64>, path: &Path) -> Backend {
        if !self.opts.bulk {
            return Backend::Std;
        }
        let Some(dev) = dev else { return listing::backend_for(path) };
        if let Some(&(_, b)) = self.backends.lock().iter().find(|(d, _)| *d == dev) {
            return b;
        }
        let b = listing::backend_for(path);
        self.backends.lock().push((dev, b));
        b
    }
}

/// Scans the folder `start` (already in the tree, without children) and its whole subtree.
pub fn scan(tree: &RwLock<Tree>, start: u32, opts: &ScanOptions, ctl: &ScanControl) {
    let _activity = platform::ScanActivity::begin();
    let start_path = tree.read().path(start);
    let start_dev = fs::metadata(&start_path).ok().and_then(|m| platform::device(&m));
    let mut allowed_devs = start_dev.map(|d| vec![d]);
    let mut skip_paths = Vec::new();
    // macOS: "/" is the read-only system volume; user data lives on the Data volume and shows
    // through firmlinks (/Users, /Applications…). We count it through the firmlinks and do not
    // visit /System/Volumes/Data itself, so that nothing is counted twice.
    if cfg!(target_os = "macos") && start_path == Path::new("/") {
        let data = PathBuf::from("/System/Volumes/Data");
        if let (Some(devs), Some(d)) = (allowed_devs.as_mut(), fs::metadata(&data).ok().and_then(|m| platform::device(&m))) {
            devs.push(d);
        }
        skip_paths.push(data);
    }
    let ctx = Ctx {
        tree,
        opts,
        ctl,
        seen_inodes: Mutex::new(HashSet::new()),
        allowed_devs,
        skip_paths,
        backends: Mutex::new(Vec::new()),
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.max(1))
        .thread_name(|i| format!("disk-lens-scan-{i}"))
        .build()
        .expect("failed to create the thread pool");
    let ctx_ref = &ctx;
    pool.scope(move |s| scan_dir(s, ctx_ref, start, start_path, start_dev));
}

/// One directory entry classified for insertion, plus whether the scan should descend into it.
struct Classified {
    entry: NewEntry,
    /// Path and device of a subfolder to scan next.
    descend: Option<(PathBuf, Option<u64>)>,
}

impl Ctx<'_> {
    /// Turns a raw entry of `dir_path` into a tree entry: hard-link duplicates count as 0 bytes,
    /// folders on another volume (or on the skip list) are marked and not entered.
    fn classify(&self, dir_path: &Path, raw: &RawEntry) -> Classified {
        let mut entry = entry_from_raw(raw.name.to_string_lossy().into_owned(), raw);
        if raw.hardlink.is_some_and(|k| !self.seen_inodes.lock().insert(k)) {
            entry.alloc = 0;
            entry.logical = 0;
            entry.flags |= flags::HARDLINK_DUP;
        }
        let mut descend = None;
        if entry.kind.is_dir() {
            let child_path = dir_path.join(raw.name);
            let same_volume = match (&self.allowed_devs, raw.dev) {
                (Some(devs), Some(d)) => devs.contains(&d),
                _ => true,
            };
            let skip = self.skip_paths.iter().any(|p| p == &child_path);
            if skip || (!same_volume && !self.opts.cross_volumes) {
                entry.flags |= flags::OTHER_VOLUME;
            } else {
                descend = Some((child_path, raw.dev));
            }
        }
        Classified { entry, descend }
    }

    /// Marks `dir` as unreadable and records why.
    fn record_unreadable(&self, dir: u32, path: &Path, error: &io::Error) {
        self.ctl.progress.errors.fetch_add(1, Relaxed);
        let mut t = self.tree.write();
        t.nodes[dir as usize].flags |= flags::UNREADABLE;
        t.skipped.push(Skipped { path: path.to_string_lossy().into_owned(), error: error.to_string() });
    }

    /// Inserts the children of `dir` in one operation under the write lock. Returns their ids.
    fn insert_children(&self, dir: u32, entries: Vec<NewEntry>, skipped: Vec<Skipped>) -> Vec<u32> {
        let timer = stats::Timer::start();
        let mut t = self.tree.write();
        timer.stop(&stats::LOCK_WAIT);
        let timer = stats::Timer::start();
        t.skipped.extend(skipped);
        let ids = t.add_children(dir, entries);
        drop(t);
        timer.stop(&stats::LOCK_HOLD);
        ids
    }
}

fn scan_dir<'s>(s: &rayon::Scope<'s>, ctx: &'s Ctx<'s>, dir: u32, path: PathBuf, dev: Option<u64>) {
    if ctx.ctl.cancelled() {
        return;
    }
    let progress = &ctx.ctl.progress;
    if let Some(mut cur) = progress.current.try_lock() {
        cur.clear();
        cur.push_str(&path.to_string_lossy());
    }

    let mut entries = Vec::new();
    let mut subdirs: Vec<(usize, PathBuf, Option<u64>)> = Vec::new();
    let mut skipped = Vec::new();
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut n = 0usize;

    let timer = stats::Timer::start();
    let listed = listing::read_dir(&path, ctx.backend(dev, &path), |item| {
        n += 1;
        if n.is_multiple_of(2048) && ctx.ctl.cancelled() {
            return false;
        }
        let raw = match item {
            Ok(raw) => raw,
            Err(err) => {
                let p = err.name.map_or_else(|| path.clone(), |name| path.join(name));
                skipped.push(Skipped { path: p.to_string_lossy().into_owned(), error: err.error.to_string() });
                return true;
            }
        };
        let Classified { entry, descend } = ctx.classify(&path, &raw);
        if let Some((child_path, child_dev)) = descend {
            subdirs.push((entries.len(), child_path, child_dev));
        }
        if !entry.kind.is_dir() {
            files += 1;
        }
        bytes += entry.alloc;
        entries.push(entry);
        true
    });
    timer.stop(&stats::LISTING);

    if let Err(e) = listed {
        ctx.record_unreadable(dir, &path, &e);
        return;
    }

    progress.files.fetch_add(files, Relaxed);
    progress.dirs.fetch_add(1, Relaxed);
    progress.bytes.fetch_add(bytes, Relaxed);
    if !skipped.is_empty() {
        progress.errors.fetch_add(skipped.len() as u64, Relaxed);
    }

    let ids = ctx.insert_children(dir, entries, skipped);
    for (idx, child_path, child_dev) in subdirs {
        let id = ids[idx];
        s.spawn(move |s| scan_dir(s, ctx, id, child_path, child_dev));
    }
}

/// Helper: a full scan of a folder without the UI (tests, the CLI example).
pub fn scan_path(root: &Path, opts: &ScanOptions) -> io::Result<Tree> {
    let tree = RwLock::new(new_tree(root)?);
    let ctl = ScanControl::default();
    scan(&tree, crate::model::ROOT, opts, &ctl);
    Ok(tree.into_inner())
}

/// Rescans the subtree of `id` in an existing tree.
pub fn rescan(tree: &RwLock<Tree>, id: u32, opts: &ScanOptions, ctl: &ScanControl) -> io::Result<()> {
    let path = tree.read().path(id);
    let meta = fs::symlink_metadata(&path)?;
    {
        let mut t = tree.write();
        t.clear_children(id);
        t.set_own(id, &entry_from_meta(String::new(), &meta));
        let prefix = path.to_string_lossy().into_owned();
        let prefix_dir = format!("{}/", prefix.trim_end_matches('/'));
        t.skipped.retain(|s| s.path != prefix && !s.path.starts_with(&prefix_dir));
    }
    if meta.is_dir() {
        scan(tree, id, opts, ctl);
    }
    Ok(())
}
