//! CLI do weryfikacji i pomiaru skanera: `cargo run --release --example scan -- <ścieżka> [opcje]`.
//! Wypisuje sumę zajętą w KiB (porównywalną z `du -sk`), liczniki i czas.
//!
//! Opcje: `--threads N` (liczba wątków), `--repeat N` (powtórzenia, min/mediana),
//! `--no-bulk` (wymusza `read_dir` + `lstat` zamiast `getattrlistbulk`),
//! `--queries` (czas zapytań widoku). Z `--features scan-stats` dochodzi podział czasu.

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.iter().find(|a| !a.starts_with("--") && a.parse::<usize>().is_err()).cloned().unwrap_or_else(|| ".".into());
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
        let t = scan_path(path.as_ref(), &opts).expect("skan nieudany");
        times.push(started.elapsed().as_secs_f64());
        tree = Some(t);
    }
    let tree = tree.expect("co najmniej jeden skan");
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

    if cfg!(feature = "scan-stats") {
        // Sumy po wątkach z ostatniego powtórzenia, w sekundach wątku.
        let s = |c: &std::sync::atomic::AtomicU64| c.load(Relaxed) as f64 / 1e9;
        println!("listing   {:.2} thread-s", s(&stats::LISTING));
        println!("metadata  {:.2} thread-s", s(&stats::METADATA));
        println!("lock_wait {:.2} thread-s", s(&stats::LOCK_WAIT));
        println!("lock_hold {:.2} s (pod blokadą, sekwencyjnie)", s(&stats::LOCK_HOLD));
    }

    // Czas zapytań widoku na całym drzewie (`--queries`).
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
