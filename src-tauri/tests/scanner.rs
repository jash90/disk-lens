//! Testy skanera na drzewie fikstur tworzonym w katalogu tymczasowym (Unix: bloki, i-węzły, chmod).
#![cfg(unix)]

use disk_lens_lib::model::{flags, Kind, Tree, ROOT};
use disk_lens_lib::scanner::{new_tree, rescan, scan, scan_path, ScanControl, ScanOptions};
use parking_lot::RwLock;
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
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

type Snapshot = BTreeMap<String, (Kind, u8, u64, u64, i64, u32, u32)>;

/// Drzewo spłaszczone do ścieżka → (rodzaj, flagi, alloc, logical, mtime, pliki, foldery).
/// Który z twardych dowiązań dostanie HARDLINK_DUP, zależy od kolejności wątków, więc dla plików
/// z kilkoma dowiązaniami porównujemy tylko rodzaj – sumy folderów i tak muszą się zgadzać.
fn snapshot(t: &Tree, root: &Path) -> Snapshot {
    let mut out = BTreeMap::new();
    let mut stack = vec![(ROOT, String::new())];
    while let Some((id, p)) = stack.pop() {
        let n = t.node(id);
        let linked = fs::symlink_metadata(root.join(p.trim_start_matches('/'))).is_ok_and(|m| !m.is_dir() && m.nlink() > 1);
        let (alloc, logical) = if linked { (0, 0) } else { (n.alloc, n.logical) };
        out.insert(p.clone(), (n.kind, n.flags & !flags::HARDLINK_DUP, alloc, logical, n.mtime, n.files, n.dirs));
        for c in t.children(id) {
            stack.push((c, format!("{p}/{}", t.name(c))));
        }
    }
    out
}

fn scan_both(r: &Path) -> (Tree, Tree) {
    let fast = scan_path(r, &ScanOptions { bulk: true, ..opts() }).unwrap();
    let slow = scan_path(r, &ScanOptions { bulk: false, ..opts() }).unwrap();
    (fast, slow)
}

#[test]
fn bulk_and_std_listing_build_identical_trees() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("a/b/c/deep.bin"), 123_456);
    write(&r.join("a/x.txt"), 10);
    write(&r.join(".hidden"), 5);
    write(&r.join("zażółć gęślą jaźń.txt"), 777);
    fs::create_dir(r.join("empty")).unwrap();
    write(&r.join("Foo.app/Contents/MacOS/foo"), 40_000);
    write(&r.join("links/orig.bin"), 90_000);
    fs::hard_link(r.join("links/orig.bin"), r.join("links/second.bin")).unwrap();
    std::os::unix::fs::symlink(r.join("a"), r.join("sym-dir")).unwrap();
    std::os::unix::fs::symlink("/nonexistent/target", r.join("sym-broken")).unwrap();
    let fifo = std::ffi::CString::new(r.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    File::create(r.join("sparse.img")).unwrap().set_len(64 * 1024 * 1024).unwrap();
    // Tyle wpisów, żeby folder nie zmieścił się w jednej paczce getattrlistbulk.
    for i in 0..5000 {
        write(&r.join(format!("many/file-with-a-longish-name-{i:05}.dat")), i % 9000);
    }

    let (fast, slow) = scan_both(r);
    assert_eq!(fast.node(ROOT).alloc, slow.node(ROOT).alloc);
    assert_eq!(fast.node(ROOT).logical, slow.node(ROOT).logical);
    assert_eq!(snapshot(&fast, r), snapshot(&slow, r));
    assert!(fast.skipped.is_empty() && slow.skipped.is_empty());
    let kind = |t: &Tree, name: &str| t.node(find(t, ROOT, name)).kind;
    assert_eq!(kind(&fast, "pipe"), Kind::Other);
    assert_eq!(kind(&fast, "sym-broken"), Kind::Symlink);
    assert_eq!(fast.node(find(&fast, ROOT, "many")).files, 5000);
}

#[test]
fn bulk_and_std_report_unreadable_dirs_alike() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("pomijam: root czyta wszystko");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("locked/secret.bin"), 1000);
    write(&r.join("ok/visible.bin"), 1000);
    fs::set_permissions(r.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let (fast, slow) = scan_both(r);
    fs::set_permissions(r.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(snapshot(&fast, r), snapshot(&slow, r));
    let paths = |t: &Tree| t.skipped.iter().map(|s| s.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&fast), paths(&slow));
}

#[derive(Debug, Clone)]
enum Item {
    File(u32),
    Dir,
    Symlink,
}

fn item() -> impl Strategy<Value = (Vec<u8>, Item)> {
    (
        prop::collection::vec(0u8..4, 1..4),
        prop_oneof![
            6 => (0u32..300_000).prop_map(Item::File),
            2 => Just(Item::Dir),
            1 => Just(Item::Symlink),
        ],
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn bulk_and_std_agree_on_random_trees(items in prop::collection::vec(item(), 1..40)) {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        let mut created = 0;
        for (parts, it) in &items {
            let rel: PathBuf = parts.iter().map(|p| format!("n{p}")).collect();
            let p = r.join(&rel);
            // Kolizje (plik tam, gdzie już jest folder itp.) po prostu pomijamy. Sprawdzamy tylko
            // przodków wewnątrz `r` – na macOS sam katalog tymczasowy leży pod symlinkiem /var.
            let blocked = rel.ancestors().skip(1).filter(|a| !a.as_os_str().is_empty()).any(|a| {
                let a = r.join(a);
                a.is_file() || a.is_symlink()
            });
            if blocked || p.symlink_metadata().is_ok() {
                continue;
            }
            created += 1;
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            match it {
                Item::File(n) => write(&p, *n as usize),
                Item::Dir => fs::create_dir(&p).unwrap(),
                Item::Symlink => std::os::unix::fs::symlink(r, &p).unwrap(),
            }
        }
        prop_assert!(created > 0);
        let (fast, slow) = scan_both(r);
        prop_assert!(fast.len() > created / 2, "drzewo ma {} węzłów przy {} utworzonych", fast.len(), created);
        prop_assert_eq!(snapshot(&fast, r), snapshot(&slow, r));
    }
}

