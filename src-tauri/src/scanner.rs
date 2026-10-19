//! Równoległy skaner systemu plików.
//!
//! Każde zadanie `rayon` obsługuje jeden folder: czyta wpisy z metadanymi (`listing`), wstawia
//! dzieci do drzewa jedną operacją pod write-lockiem (sumy od razu idą w górę, więc wyniki są
//! widoczne w trakcie skanu) i zleca podfoldery kolejnym zadaniom.

use crate::category;
use crate::listing::{self, Backend, FileKind, RawEntry};
use crate::platform;
use crate::model::{flags, Kind, NewEntry, Skipped, Tree};
use parking_lot::{Mutex, RwLock};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// Liczniki czasu skanu (ns, sumowane po wątkach) – zbierane tylko z funkcją `scan-stats`.
pub mod stats {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    /// Cały odczyt folderu (otwarcie, wpisy, metadane).
    pub static LISTING: AtomicU64 = AtomicU64::new(0);
    /// Pojedyncze `lstat` (ścieżka standardowa i awaryjna).
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

/// Rozszerzenia folderów, które macOS pokazuje jako pojedynczy plik.
const PACKAGE_EXTS: &[&str] = &[
    "app", "photoslibrary", "bundle", "framework", "plugin", "kext", "appex", "xpc", "pkg",
    "mpkg", "rtfd", "sparsebundle", "logicx", "fcpbundle", "imovielibrary", "musiclibrary",
    "tvlibrary", "xcarchive", "xcodeproj", "xcworkspace", "playground", "band", "pages", "numbers",
    "key", "scptd",
];

#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Czy wchodzić do punktów montowania innych wolumenów.
    pub cross_volumes: bool,
    pub threads: usize,
    /// Szybki odczyt folderów (`getattrlistbulk` na macOS), gdzie system plików go obsługuje.
    pub bulk: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        // Skan jest ograniczony przez I/O metadanych – więcej wątków niż rdzeni pomaga.
        ScanOptions { cross_volumes: false, threads: (cores * 2).clamp(4, 32), bulk: true }
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

pub fn is_package(name: &str) -> bool {
    match name.rfind('.') {
        Some(i) if i > 0 => {
            let ext = name[i + 1..].to_ascii_lowercase();
            PACKAGE_EXTS.contains(&ext.as_str())
        }
        _ => false,
    }
}

pub fn entry_from_meta(name: String, meta: &Metadata) -> NewEntry {
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
    }
}

/// Tworzy drzewo z samym korzeniem. Korzeń musi być folderem (symlink do folderu jest rozwijany).
pub fn new_tree(root: &Path) -> io::Result<Tree> {
    let meta = fs::metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Wybrana ścieżka nie jest folderem"));
    }
    let mut e = entry_from_meta(String::new(), &meta);
    e.kind = Kind::Dir;
    e.cat = category::FOLDER;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(Tree::new(root.to_path_buf(), e, now))
}

struct Ctx<'a> {
    tree: &'a RwLock<Tree>,
    opts: &'a ScanOptions,
    ctl: &'a ScanControl,
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
    /// Urządzenia, na których wolno schodzić w głąb (`None` = system bez wykrywania montowań).
    allowed_devs: Option<Vec<u64>>,
    /// Ścieżki, do których nie wchodzimy (np. /System/Volumes/Data przy skanie „/”).
    skip_paths: Vec<PathBuf>,
    /// Sposób odczytu folderów dla każdego napotkanego urządzenia (zwykle 1–2 wpisy).
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

/// Skanuje folder `start` (już obecny w drzewie, bez dzieci) i całe jego poddrzewo.
pub fn scan(tree: &RwLock<Tree>, start: u32, opts: &ScanOptions, ctl: &ScanControl) {
    let start_path = tree.read().path(start);
    let start_dev = fs::metadata(&start_path).ok().and_then(|m| platform::device(&m));
    let mut allowed_devs = start_dev.map(|d| vec![d]);
    let mut skip_paths = Vec::new();
    // macOS: „/” to wolumen systemowy tylko do odczytu; dane użytkownika leżą na wolumenie Data
    // i są widoczne przez firmlinki (/Users, /Applications…). Liczymy je przez firmlinki,
    // a samego /System/Volumes/Data nie odwiedzamy, żeby nie liczyć niczego dwa razy.
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
        .expect("nie udało się utworzyć puli wątków");
    let ctx_ref = &ctx;
    pool.scope(move |s| scan_dir(s, ctx_ref, start, start_path, start_dev));
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
        let mut e = entry_from_raw(raw.name.to_string_lossy().into_owned(), &raw);

        if raw.hardlink.is_some_and(|k| !ctx.seen_inodes.lock().insert(k)) {
            e.alloc = 0;
            e.logical = 0;
            e.flags |= flags::HARDLINK_DUP;
        }

        if e.kind.is_dir() {
            let child_path = path.join(raw.name);
            let same_volume = match (&ctx.allowed_devs, raw.dev) {
                (Some(devs), Some(d)) => devs.contains(&d),
                _ => true,
            };
            let skip = ctx.skip_paths.iter().any(|p| p == &child_path);
            if skip || (!same_volume && !ctx.opts.cross_volumes) {
                e.flags |= flags::OTHER_VOLUME;
            } else {
                subdirs.push((entries.len(), child_path, raw.dev));
            }
        } else {
            files += 1;
        }
        bytes += e.alloc;
        entries.push(e);
        true
    });
    timer.stop(&stats::LISTING);

    if let Err(e) = listed {
        progress.errors.fetch_add(1, Relaxed);
        let mut t = ctx.tree.write();
        t.nodes[dir as usize].flags |= flags::UNREADABLE;
        t.skipped.push(Skipped { path: path.to_string_lossy().into_owned(), error: e.to_string() });
        return;
    }

    progress.files.fetch_add(files, Relaxed);
    progress.dirs.fetch_add(1, Relaxed);
    progress.bytes.fetch_add(bytes, Relaxed);
    if !skipped.is_empty() {
        progress.errors.fetch_add(skipped.len() as u64, Relaxed);
    }

    let ids = {
        let timer = stats::Timer::start();
        let mut t = ctx.tree.write();
        timer.stop(&stats::LOCK_WAIT);
        let timer = stats::Timer::start();
        t.skipped.extend(skipped);
        let ids = t.add_children(dir, entries);
        drop(t);
        timer.stop(&stats::LOCK_HOLD);
        ids
    };

    for (idx, child_path, child_dev) in subdirs {
        let id = ids[idx];
        s.spawn(move |s| scan_dir(s, ctx, id, child_path, child_dev));
    }
}

/// Pomocnicze: pełny skan folderu bez UI (testy, przykład CLI).
pub fn scan_path(root: &Path, opts: &ScanOptions) -> io::Result<Tree> {
    let tree = RwLock::new(new_tree(root)?);
    let ctl = ScanControl::default();
    scan(&tree, crate::model::ROOT, opts, &ctl);
    Ok(tree.into_inner())
}

/// Ponowny skan poddrzewa `id` w istniejącym drzewie.
pub fn rescan(tree: &RwLock<Tree>, id: u32, opts: &ScanOptions, ctl: &ScanControl) -> io::Result<()> {
    let path = tree.read().path(id);
    let meta = fs::symlink_metadata(&path)?;
    {
        let mut t = tree.write();
        t.clear_children(id);
        t.set_own(id, platform::alloc_size(&meta), meta.len(), platform::mtime(&meta));
        let prefix = path.to_string_lossy().into_owned();
        let prefix_dir = format!("{}/", prefix.trim_end_matches('/'));
        t.skipped.retain(|s| s.path != prefix && !s.path.starts_with(&prefix_dir));
    }
    if meta.is_dir() {
        scan(tree, id, opts, ctl);
    }
    Ok(())
}
