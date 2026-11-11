//! CLI for checking and measuring the scanner: `cargo run --release --example scan -- <path> [options]`.
//! Prints the allocated total in KiB (comparable with `du -sk`), the counters and the time.
//!
//! Options: `--threads N`, `--repeat N` (min/median), `--no-bulk` (forces `read_dir` + `lstat`
//! instead of `getattrlistbulk`), `--queries` (time of the view queries), `--save FILE` (writes
//! the scan with its position in the event log), `--refresh FILE` (instead of scanning: loads
//! FILE, reads only what changed since and prints the time of each step), `--compare FILE`
//! (counts entries whose size differs from the scan in FILE; `--refresh-other` refreshes that
//! scan first). With `--features scan-stats` the
//! time is broken down.

use disk_lens_lib::fsevents;
use disk_lens_lib::persist;
use disk_lens_lib::refresh;
use disk_lens_lib::scanner::ScanControl;
use parking_lot::RwLock;
use std::path::Path;
use std::time::Duration;
use disk_lens_lib::model::{SizeMode, ROOT};
use disk_lens_lib::query::{self, compute_filtered, Filter, SortKey, View};
use disk_lens_lib::scanner::{scan_path, stats, ScanOptions};
use disk_lens_lib::treemap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Instant;

fn flag(args: &[String], name: &str) -> Option<usize> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1)?.parse().ok()
}

fn text_flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

/// Loads a saved scan and brings it up to date from the event log, timing each step.
fn refresh_from(file: &str, root: &Path, opts: &ScanOptions) -> disk_lens_lib::model::Tree {
    let step = Instant::now();
    let now = fsevents::position(root).expect("no event log for this volume");
    let tree = persist::load(file.as_ref()).expect("cannot load the saved scan");
    println!("load      {:.2}s ({} nodes)", step.elapsed().as_secs_f64(), tree.len());
    let since = fsevents::Position { event_id: tree.state.event_id, uuid: tree.state.event_uuid.clone() };
    println!("event ids {} since the saved scan", now.event_id.saturating_sub(since.event_id));
    let step = Instant::now();
    let changes = fsevents::changes_since(root, &since, Duration::from_secs(120), &|| false).expect("event log unusable");
    println!("event log {:.2}s ({} changed folders, {} in depth)", step.elapsed().as_secs_f64(), changes.len(), changes.iter().filter(|c| c.recursive).count());
    let shared = RwLock::new(tree);
    let step = Instant::now();
    let done = refresh::refresh(&shared, &changes, opts, &ScanControl::default());
    println!("refresh   {:.2}s ({} listed, {} rescanned)", step.elapsed().as_secs_f64(), done.listed, done.rescanned);
    let mut tree = shared.into_inner();
    (tree.state.event_id, tree.state.event_uuid) = (now.event_id, now.uuid);
    tree
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let takes_value = ["--threads", "--repeat", "--save", "--refresh", "--compare"];
    let path = args
        .iter()
        .enumerate()
        .find(|(i, a)| !(a.starts_with("--") || *i > 0 && takes_value.contains(&args[i - 1].as_str())))
        .map(|(_, a)| a.clone())
        .unwrap_or_else(|| ".".into());
    let mut opts = ScanOptions::default();
    if let Some(n) = flag(&args, "--threads") {
        opts.threads = n;
    }
    opts.bulk = !args.iter().any(|a| a == "--no-bulk");
    let repeat = flag(&args, "--repeat").unwrap_or(1).max(1);

    let mut times = Vec::new();
    let mut tree = None;
    for _ in 0..repeat {
        stats::reset();
        let started = Instant::now();
        let t = match text_flag(&args, "--refresh") {
            Some(file) => refresh_from(&file, path.as_ref(), &opts),
            None => {
                let position = fsevents::position(path.as_ref()).unwrap_or_default();
                let mut t = scan_path(path.as_ref(), &opts).expect("scan failed");
                (t.state.event_id, t.state.event_uuid) = (position.event_id, position.uuid);
                t
            }
        };
        times.push(started.elapsed().as_secs_f64());
        tree = Some(t);
    }
    let tree = tree.expect("at least one scan");
    times.sort_by(f64::total_cmp);
    let (min, median) = (times[0], times[times.len() / 2]);

    let root = tree.node(ROOT);
    println!("path      {path}");
    println!("threads   {} (bulk: {})", opts.threads, opts.bulk);
    println!("alloc_kib {}", root.alloc / 1024);
    println!("logical_b {}", root.logical);
    println!("files     {}", root.files);
    println!("dirs      {}", root.dirs);
    println!("nodes     {}", tree.len());
    println!("skipped   {}", tree.skipped.len());
    println!("elapsed   {median:.2}s (min {min:.2}s, n={repeat})");
    println!("entries/s {:.0}", tree.len() as f64 / median);

    if let Some(file) = text_flag(&args, "--compare") {
        // With `--refresh-other` the other scan is brought up to date first, so that both
        // describe the disk at (nearly) the same moment.
        let other = if args.iter().any(|a| a == "--refresh-other") {
            refresh_from(&file, path.as_ref(), &opts)
        } else {
            persist::load(file.as_ref()).expect("cannot load the scan to compare with")
        };
        let diff = persist::diff(&other, &tree, SizeMode::Alloc, usize::MAX);
        let o = other.node(ROOT);
        println!("compare   {} entries differ; other: alloc_kib {} files {} dirs {} skipped {}", diff.entries.len(), o.alloc / 1024, o.files, o.dirs, other.skipped.len());
        for e in diff.entries.iter().filter(|e| !e.is_dir).take(40) {
            println!("  {:<8} {:>12} {}", e.status, e.delta, e.path);
        }
    }
    if let Some(file) = text_flag(&args, "--save") {
        let step = Instant::now();
        persist::save(&tree, file.as_ref()).expect("cannot save");
        println!("save      {:.2}s", step.elapsed().as_secs_f64());
    }

    if cfg!(feature = "scan-stats") {
        // Sums over threads from the last repetition, in thread-seconds.
        let s = |c: &std::sync::atomic::AtomicU64| c.load(Relaxed) as f64 / 1e9;
        println!("listing   {:.2} thread-s", s(&stats::LISTING));
        println!("metadata  {:.2} thread-s", s(&stats::METADATA));
        println!("lock_wait {:.2} thread-s", s(&stats::LOCK_WAIT));
        println!("lock_hold {:.2} s (under the lock, sequential)", s(&stats::LOCK_HOLD));
    }

    // Time of the view queries on the whole tree (`--queries`).
    if args.iter().any(|a| a == "--queries") {
        let time = |label: &str, f: &mut dyn FnMut()| {
            let t = Instant::now();
            f();
            println!("{label:<16}{:>8.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        };
        let v = View::new(&tree, SizeMode::Alloc, None);
        time("children", &mut || drop(query::children(&v, ROOT, SortKey::Size, true, 0, 500)));
        time("top_files", &mut || drop(query::top_files(&v, ROOT, 100)));
        time("type_breakdown", &mut || drop(query::type_breakdown(&v, ROOT, 40)));
        let opts = treemap::Options { width: 1300.0, height: 800.0, min_area: 1.0, max_tiles: 50_000, padding: 1.0, now: 0 };
        time("treemap", &mut || drop(treemap::layout(&v, ROOT, &opts)));
        let f = Filter { excludes: vec!["node_modules".into()], ..Default::default() };
        let mut sums = None;
        time("filter", &mut || sums = Some(Arc::new(compute_filtered(&tree, &f, SizeMode::Alloc))));
        let fv = View::new(&tree, SizeMode::Alloc, sums);
        time("treemap+filter", &mut || drop(treemap::layout(&fv, ROOT, &opts)));
    }
}
