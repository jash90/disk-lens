//! Testy skanera na drzewie fikstur tworzonym w katalogu tymczasowym (Unix: bloki, i-węzły, chmod).
#![cfg(unix)]

use disk_lens_lib::model::{flags, Kind, Tree, ROOT};
use disk_lens_lib::scanner::{new_tree, rescan, scan, scan_path, ScanControl, ScanOptions};
use parking_lot::RwLock;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

fn write(path: &Path, bytes: usize) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    let mut f = File::create(path).unwrap();
    f.write_all(&vec![7u8; bytes]).unwrap();
}

fn alloc(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

fn find(tree: &Tree, parent: u32, name: &str) -> u32 {
    tree.children(parent).find(|&c| tree.name(c) == name).unwrap_or_else(|| panic!("brak {name}"))
}

fn opts() -> ScanOptions {
    ScanOptions { threads: 4, ..Default::default() }
}

#[test]
fn folder_sums_match_metadata() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("a/x.bin"), 10_000);
    write(&r.join("a/b/y.bin"), 50_000);
    write(&r.join("z.txt"), 3);
    fs::create_dir(r.join("empty")).unwrap();

    let t = scan_path(r, &opts()).unwrap();
    let a = find(&t, ROOT, "a");
    let b = find(&t, a, "b");
    assert_eq!(t.node(b).alloc, alloc(&r.join("a/b")) + alloc(&r.join("a/b/y.bin")));
    assert_eq!(
        t.node(a).alloc,
        alloc(&r.join("a")) + alloc(&r.join("a/x.bin")) + t.node(b).alloc
    );
    assert_eq!(t.node(a).logical, fs::metadata(r.join("a")).unwrap().len() + 10_000 + t.node(b).logical);
    assert_eq!(t.node(ROOT).files, 3);
    assert_eq!(t.node(ROOT).dirs, 3);
    assert_eq!(t.node(a).files, 2);
    assert!(t.skipped.is_empty());
}

#[test]
fn hardlinks_counted_once() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("one/orig.bin"), 200_000);
    fs::create_dir(r.join("two")).unwrap();
    fs::hard_link(r.join("one/orig.bin"), r.join("two/link.bin")).unwrap();

    let t = scan_path(r, &opts()).unwrap();
    let one = find(&t, ROOT, "one");
    let two = find(&t, ROOT, "two");
    let files_alloc = t.node(one).alloc + t.node(two).alloc - alloc(&r.join("one")) - alloc(&r.join("two"));
    assert_eq!(files_alloc, alloc(&r.join("one/orig.bin")), "dowiązanie liczone tylko raz");
    let dups = [find(&t, one, "orig.bin"), find(&t, two, "link.bin")]
        .iter()
        .filter(|&&id| t.node(id).flags & flags::HARDLINK_DUP != 0)
        .count();
    assert_eq!(dups, 1);
}

#[test]
fn symlinks_are_not_followed() {
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("big.bin"), 1_000_000);
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::os::unix::fs::symlink(outside.path(), r.join("link-to-dir")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("big.bin"), r.join("link-to-file")).unwrap();

    let t = scan_path(r, &opts()).unwrap();
    let l = find(&t, ROOT, "link-to-dir");
    assert_eq!(t.node(l).kind, Kind::Symlink);
    assert_eq!(t.children(l).count(), 0);
    assert!(t.node(ROOT).alloc < 100_000, "rozmiar celu symlinka nie może być doliczony");
    assert!(t.node(ROOT).logical < 10_000);
}

#[test]
fn unreadable_dir_is_skipped_and_scan_continues() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("pomijam: root czyta wszystko");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("locked/secret.bin"), 1000);
    write(&r.join("open/visible.bin"), 1000);
    fs::set_permissions(r.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();

    let t = scan_path(r, &opts());
    fs::set_permissions(r.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    let t = t.unwrap();

    let locked = find(&t, ROOT, "locked");
    assert!(t.node(locked).flags & flags::UNREADABLE != 0);
    assert_eq!(t.skipped.len(), 1);
    assert!(t.skipped[0].path.ends_with("locked"));
    let open = find(&t, ROOT, "open");
    assert_eq!(t.node(open).files, 1);
}

#[test]
fn sparse_file_logical_bigger_than_allocated() {
    let d = tempfile::tempdir().unwrap();
    let f = File::create(d.path().join("sparse.img")).unwrap();
    f.set_len(512 * 1024 * 1024).unwrap();
    drop(f);
    let t = scan_path(d.path(), &opts()).unwrap();
    let id = find(&t, ROOT, "sparse.img");
    assert_eq!(t.node(id).logical, 512 * 1024 * 1024);
    assert!(t.node(id).alloc < 1024 * 1024, "plik rzadki zajmuje {} B", t.node(id).alloc);
}

#[test]
fn packages_are_single_nodes_but_scanned() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join("Foo.app/Contents/MacOS/foo"), 40_000);
    let t = scan_path(d.path(), &opts()).unwrap();
    let app = find(&t, ROOT, "Foo.app");
    assert_eq!(t.node(app).kind, Kind::Package);
    assert_eq!(t.node(app).files, 1);
    assert!(t.node(app).alloc >= 40_000);
}

#[test]
fn cancel_stops_quickly() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..300 {
        for j in 0..10 {
            fs::create_dir_all(d.path().join(format!("d{i}/e{j}"))).unwrap();
        }
    }
    // Anulowany przed startem: nic poza korzeniem.
    let tree = RwLock::new(new_tree(d.path()).unwrap());
    let ctl = ScanControl::default();
    ctl.cancel.store(true, Relaxed);
    scan(&tree, ROOT, &opts(), &ctl);
    assert_eq!(tree.read().len(), 1);

    // Anulowany w trakcie: kończy się szybko i drzewo jest spójne.
    let tree = RwLock::new(new_tree(d.path()).unwrap());
    let ctl = ScanControl::default();
    let started = Instant::now();
    std::thread::scope(|s| {
        s.spawn(|| {
            std::thread::sleep(Duration::from_millis(2));
            ctl.cancel.store(true, Relaxed);
        });
        scan(&tree, ROOT, &ScanOptions { threads: 1, ..Default::default() }, &ctl);
    });
    assert!(started.elapsed() < Duration::from_secs(1));
    let t = tree.read();
    let counted = t.nodes.iter().skip(1).filter(|n| n.kind == Kind::Dir).count() as u32;
    assert_eq!(t.node(ROOT).dirs, counted);
}

#[test]
fn rescan_updates_totals() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("sub/a.bin"), 10_000);
    write(&r.join("keep.bin"), 5_000);
    let tree = RwLock::new(new_tree(r).unwrap());
    scan(&tree, ROOT, &opts(), &ScanControl::default());
    let sub = find(&tree.read(), ROOT, "sub");

    write(&r.join("sub/b.bin"), 300_000);
    rescan(&tree, sub, &opts(), &ScanControl::default()).unwrap();
    let t = tree.read();
    let expected = alloc(r) + alloc(&r.join("keep.bin")) + alloc(&r.join("sub")) + alloc(&r.join("sub/a.bin")) + alloc(&r.join("sub/b.bin"));
    assert_eq!(t.node(ROOT).alloc, expected);
    assert_eq!(t.node(ROOT).files, 3);
    assert_eq!(t.node(sub).files, 2);
}

#[test]
fn matches_du() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    for i in 0..40 {
        write(&r.join(format!("d{}/f{i}.bin", i % 7)), 1000 * (i + 1) * 37);
    }
    fs::hard_link(r.join("d0/f0.bin"), r.join("d1/hl.bin")).unwrap();
    let t = scan_path(r, &opts()).unwrap();
    let out = std::process::Command::new("du").arg("-sk").arg(r).output().unwrap();
    let du_kib: u64 = String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(t.node(ROOT).alloc / 1024, du_kib);
}

#[test]
fn trash_moves_file_without_finder() {
    // Prawdziwy Kosz użytkownika – plik o unikalnej nazwie, sprzątamy go po teście.
    let d = tempfile::tempdir().unwrap();
    let name = format!("disk-lens-test-{}.bin", std::process::id());
    let p = d.path().join(&name);
    write(&p, 1000);
    disk_lens_lib::actions::move_to_trash(&p).unwrap();
    assert!(!p.exists());
    let trashed = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".Trash").join(&name);
    if trashed.exists() {
        fs::remove_file(trashed).unwrap();
    }
}
