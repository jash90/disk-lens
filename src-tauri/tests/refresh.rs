//! Incremental refresh against a real folder: after any mix of changes, refreshing the changed
//! folders must give the same tree as scanning everything again.
#![cfg(unix)]

use disk_lens_lib::model::{flags, Kind, Tree, ROOT};
use disk_lens_lib::refresh::{refresh, Change, RefreshStats};
use disk_lens_lib::scanner::{scan_path, ScanControl, ScanOptions};
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn opts() -> ScanOptions {
    ScanOptions { threads: 4, ..Default::default() }
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![3u8; bytes]).unwrap();
}

type Snapshot = BTreeMap<String, (Kind, u8, u64, u64, i64, u32, u32, u32)>;

/// path → (kind, flags, alloc, logical, mtime, created, files, folders) of every live node.
/// Which hard link carries the size differs between scans, so links compare by kind only.
fn snapshot(t: &Tree) -> Snapshot {
    let mut out = Snapshot::new();
    let mut stack = vec![ROOT];
    while let Some(id) = stack.pop() {
        let n = t.node(id);
        let linked = t.state.links.iter().any(|l| l.id == id);
        let row = if linked {
            (n.kind, 0, 0, 0, 0, 0, 0, 0)
        } else {
            (n.kind, n.flags, n.alloc, n.logical, n.mtime, n.created, n.files, n.dirs)
        };
        out.insert(t.rel_path(ROOT, id), row);
        stack.extend(t.children(id));
    }
    out
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    tree: RwLock<Tree>,
}

impl Fixture {
    fn new(build: impl FnOnce(&Path)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        fs::create_dir_all(&root).unwrap();
        build(&root);
        let tree = RwLock::new(scan_path(&root, &opts()).unwrap());
        Fixture { _dir: dir, root, tree }
    }

    fn refresh(&self, changed: &[&str]) -> RefreshStats {
        self.refresh_with(changed, false)
    }

    fn refresh_with(&self, changed: &[&str], recursive: bool) -> RefreshStats {
        let changes: Vec<Change> = changed.iter().map(|rel| Change { path: self.root.join(rel), recursive }).collect();
        refresh(&self.tree, &changes, &opts(), &ScanControl::default())
    }

    /// The refreshed tree against a scan made from scratch right now.
    fn assert_matches_fresh_scan(&self) {
        let fresh = scan_path(&self.root, &opts()).unwrap();
        let t = self.tree.read();
        let (ours, theirs) = (snapshot(&t), snapshot(&fresh));
        for (path, row) in &theirs {
            assert_eq!(ours.get(path), Some(row), "node {path:?} differs from a fresh scan");
        }
        assert_eq!(ours.len(), theirs.len(), "extra nodes: {:?}", ours.keys().filter(|k| !theirs.contains_key(*k)).collect::<Vec<_>>());
        let (a, b) = (t.node(ROOT), fresh.node(ROOT));
        assert_eq!((a.alloc, a.logical, a.files, a.dirs), (b.alloc, b.logical, b.files, b.dirs), "root totals");
        let mut skipped: Vec<_> = t.skipped.iter().map(|s| &s.path).collect();
        let mut fresh_skipped: Vec<_> = fresh.skipped.iter().map(|s| &s.path).collect();
        skipped.sort();
        fresh_skipped.sort();
        assert_eq!(skipped, fresh_skipped, "skipped paths");
    }
}

fn base(root: &Path) {
    write(&root.join("a/one.txt"), 1000);
    write(&root.join("a/two.bin"), 9000);
    write(&root.join("a/deep/three.txt"), 300);
    write(&root.join("b/four.txt"), 5000);
    write(&root.join("b/sub/five.txt"), 70_000);
    write(&root.join("top.txt"), 10);
}

#[test]
fn nothing_changed_changes_nothing() {
    let f = Fixture::new(base);
    let before = snapshot(&f.tree.read());
    let stats = f.refresh(&["a", "b/sub", ""]);
    assert_eq!((stats.listed, stats.rescanned), (3, 0));
    assert_eq!(snapshot(&f.tree.read()), before);
    f.assert_matches_fresh_scan();
}

#[test]
fn added_removed_and_grown_files() {
    let f = Fixture::new(base);
    write(&f.root.join("a/new.txt"), 4000);
    fs::remove_file(f.root.join("a/one.txt")).unwrap();
    write(&f.root.join("b/four.txt"), 123_456);
    f.refresh(&["a", "b"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn file_grown_in_place_without_touching_its_folder() {
    let f = Fixture::new(base);
    let folder_mtime = fs::metadata(f.root.join("b/sub")).unwrap().modified().unwrap();
    write(&f.root.join("b/sub/five.txt"), 500_000);
    fs::File::open(f.root.join("b/sub")).unwrap().set_modified(folder_mtime).unwrap();
    f.refresh(&["b/sub"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn recently_modified_file_is_checked_without_any_reported_change() {
    // A program that keeps a file open writes to it without the event log saying so.
    let f = Fixture::new(base);
    write(&f.root.join("b/sub/five.txt"), 900_000);
    let stats = f.refresh(&[]);
    assert_eq!((stats.listed, stats.rescanned, stats.rechecked), (0, 0, 6));
    f.assert_matches_fresh_scan();
}

#[test]
fn file_that_was_old_at_scan_time_is_left_to_the_event_log() {
    let f = Fixture::new(|root| {
        base(root);
        let old = SystemTime::now() - Duration::from_secs(30 * 24 * 3600);
        fs::File::options().write(true).open(root.join("top.txt")).unwrap().set_modified(old).unwrap();
    });
    assert_eq!(f.refresh(&[]).rechecked, 5);
}

#[test]
fn new_folder_tree_is_scanned_in_depth() {
    let f = Fixture::new(base);
    write(&f.root.join("a/pkg/x/y/z.bin"), 20_000);
    write(&f.root.join("a/pkg/readme"), 5);
    // The event log reports the new folders themselves; none of them is in the tree yet.
    f.refresh(&["a/pkg/x/y", "a/pkg", "a"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn removed_and_moved_folders() {
    let f = Fixture::new(base);
    fs::rename(f.root.join("a/deep"), f.root.join("b/moved")).unwrap();
    fs::remove_dir_all(f.root.join("b/sub")).unwrap();
    f.refresh(&["a", "b", "b/sub"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn file_replaced_by_folder_and_back() {
    let f = Fixture::new(base);
    fs::remove_file(f.root.join("top.txt")).unwrap();
    write(&f.root.join("top.txt/inside"), 800);
    fs::remove_dir_all(f.root.join("a/deep")).unwrap();
    write(&f.root.join("a/deep"), 77);
    f.refresh(&["", "a", "top.txt", "a/deep"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn renamed_file_and_changed_times() {
    let f = Fixture::new(base);
    fs::rename(f.root.join("a/one.txt"), f.root.join("a/uno.txt")).unwrap();
    let later = SystemTime::now() + Duration::from_secs(3600);
    fs::File::options().write(true).open(f.root.join("a/two.bin")).unwrap().set_modified(later).unwrap();
    f.refresh(&["a"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn deep_change_rescans_the_whole_folder() {
    let f = Fixture::new(base);
    write(&f.root.join("a/deep/more/files.bin"), 9999);
    fs::remove_file(f.root.join("a/one.txt")).unwrap();
    let stats = f.refresh_with(&["a", "a/deep"], true);
    assert_eq!((stats.listed, stats.rescanned), (0, 1), "the folder below is covered by the one above");
    f.assert_matches_fresh_scan();
}

#[test]
fn hard_links_keep_the_total_right() {
    let f = Fixture::new(|root| {
        base(root);
        write(&root.join("a/big.bin"), 200_000);
        fs::hard_link(root.join("a/big.bin"), root.join("b/big-link.bin")).unwrap();
    });
    // A third link appears, and the link that may be carrying the size goes away.
    fs::hard_link(f.root.join("a/big.bin"), f.root.join("b/sub/third.bin")).unwrap();
    fs::remove_file(f.root.join("a/big.bin")).unwrap();
    // A file that had one name gets a second one.
    fs::hard_link(f.root.join("top.txt"), f.root.join("a/top-link.txt")).unwrap();
    f.refresh(&["a", "b/sub", ""]);
    f.assert_matches_fresh_scan();
    let t = f.tree.read();
    let carrying = |name: &str| {
        let ids: Vec<u32> = (0..t.len() as u32).filter(|&i| t.is_attached(i) && t.name(i).contains(name)).collect();
        ids.iter().filter(|&&i| t.node(i).flags & flags::HARDLINK_DUP == 0).count()
    };
    assert_eq!((carrying("big"), carrying("third") + carrying("big")), (1, 1), "exactly one link of the big file carries its size");
    assert_eq!(carrying("top"), 1);

    drop(t);
    fs::remove_file(f.root.join("b/big-link.bin")).unwrap();
    fs::remove_file(f.root.join("b/sub/third.bin")).unwrap();
    f.refresh(&["b", "b/sub"]);
    f.assert_matches_fresh_scan();
}

#[test]
fn folder_that_became_unreadable_and_readable_again() {
    let f = Fixture::new(base);
    let closed = f.root.join("b/sub");
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    f.refresh(&["b/sub"]);
    f.assert_matches_fresh_scan();
    assert_eq!(f.tree.read().skipped.len(), 1);

    fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).unwrap();
    f.refresh(&["b/sub"]);
    f.assert_matches_fresh_scan();
    assert!(f.tree.read().skipped.is_empty());
}

#[test]
fn paths_outside_the_tree_and_vanished_paths_are_harmless() {
    let f = Fixture::new(base);
    let before = snapshot(&f.tree.read());
    let outside = Change { path: PathBuf::from("/definitely/not/here"), recursive: true };
    refresh(&f.tree, &[outside], &opts(), &ScanControl::default());
    f.refresh(&["no/such/folder", "a/one.txt"]);
    assert_eq!(snapshot(&f.tree.read()), before);
    f.assert_matches_fresh_scan();
}

#[test]
fn many_random_changes_match_a_fresh_scan() {
    let f = Fixture::new(|root| {
        for d in 0..12 {
            for i in 0..15 {
                write(&root.join(format!("d{d}/s{}/f{i}.dat", i % 3)), 100 * (i + 1) + d);
            }
        }
    });
    let mut changed = Vec::new();
    for d in 0..12usize {
        let dir = format!("d{d}/s{}", d % 3);
        match d % 4 {
            0 => write(&f.root.join(&dir).join("added.dat"), 4242),
            1 => fs::remove_file(f.root.join(&dir).join(format!("f{}.dat", d % 3))).unwrap(),
            2 => write(&f.root.join(&dir).join(format!("f{}.dat", d % 3)), 77_777),
            _ => {
                fs::remove_dir_all(f.root.join(&dir)).unwrap();
                changed.push(format!("d{d}"));
            }
        }
        changed.push(dir);
    }
    let refs: Vec<&str> = changed.iter().map(String::as_str).collect();
    f.refresh(&refs);
    f.assert_matches_fresh_scan();
}
