//! CLI do weryfikacji skanera: `cargo run --release --example scan -- <ścieżka>`.
//! Wypisuje sumę zajętą w KiB (porównywalną z `du -sk`), liczniki i czas.

use disk_lens_lib::model::{SizeMode, ROOT};
use disk_lens_lib::query::{self, compute_filtered, Filter, SortKey, View};
use disk_lens_lib::treemap;
use std::sync::Arc;
use disk_lens_lib::scanner::{scan_path, ScanOptions};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let started = Instant::now();
    let tree = scan_path(path.as_ref(), &ScanOptions::default()).expect("skan nieudany");
    let root = tree.node(ROOT);
    println!("path      {path}");
    println!("alloc_kib {}", root.alloc / 1024);
    println!("logical_b {}", root.logical);
    println!("files     {}", root.files);
    println!("dirs      {}", root.dirs);
    println!("nodes     {}", tree.len());
    println!("skipped   {}", tree.skipped.len());
    println!("elapsed   {:.2}s", started.elapsed().as_secs_f64());

    // Czas zapytań widoku na całym drzewie (`--queries`).
    if std::env::args().any(|a| a == "--queries") {
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
