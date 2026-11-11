//! Batch move to the Trash against a real folder. The "Trash" here is a folder of the test.
#![cfg(unix)]

use disk_lens_lib::actions::RULES;
use disk_lens_lib::model::{Tree, ROOT};
use disk_lens_lib::scanner::{scan_path, ScanOptions};
use disk_lens_lib::trash_batch::{check, execute, top_level, BatchOutcome};
use parking_lot::RwLock;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes]).unwrap();
}

fn id_of(tree: &Tree, rel: &str) -> u32 {
    rel.split('/').fold(ROOT, |dir, name| {
        tree.children(dir).find(|&c| tree.name(c) == name).unwrap_or_else(|| panic!("{rel} is not in the tree"))
    })
}

struct Fixture {
    _dir: tempfile::TempDir,
    bin: tempfile::TempDir,
    tree: RwLock<Tree>,
}

impl Fixture {
    /// `data/{old.bin, docs/{a.txt, deep/b.txt}, cache/c.bin}` scanned.
    fn new() -> (Self, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        write(&root.join("old.bin"), 4096);
        write(&root.join("docs/a.txt"), 100);
        write(&root.join("docs/deep/b.txt"), 200);
        write(&root.join("cache/c.bin"), 8192);
        let tree = scan_path(&root, &ScanOptions { threads: 2, ..Default::default() }).unwrap();
        (Fixture { _dir: dir, bin: tempfile::tempdir().unwrap(), tree: RwLock::new(tree) }, root)
    }

    fn id(&self, rel: &str) -> u32 {
        id_of(&self.tree.read(), rel)
    }

    fn run(&self, ids: &[u32], verify: bool) -> BatchOutcome {
        let bin = self.bin.path();
        execute(&self.tree, ids, verify, None, RULES, |p| {
            let n = fs::read_dir(bin).unwrap().count();
            fs::rename(p, bin.join(format!("{n}-{}", p.file_name().unwrap().to_string_lossy()))).map_err(|e| e.to_string())
        })
    }
}

#[test]
fn unchanged_items_are_moved_and_subtracted() {
    let (f, root) = Fixture::new();
    let before = f.tree.read().node(ROOT).alloc;
    let out = f.run(&[f.id("old.bin"), f.id("docs")], true);
    assert_eq!((out.trashed, out.skipped.len(), out.failed.len()), (2, 0, 0), "{out:?}");
    assert!(!root.join("old.bin").exists() && !root.join("docs").exists() && root.join("cache/c.bin").exists());
    assert_eq!(f.tree.read().node(ROOT).alloc, before - out.freed);
    assert_eq!(f.tree.read().node(ROOT).files, 1);
}

#[test]
fn file_modified_after_the_scan_is_skipped() {
    let (f, root) = Fixture::new();
    let later = SystemTime::now() + Duration::from_secs(120);
    fs::File::options().write(true).open(root.join("old.bin")).unwrap().set_modified(later).unwrap();
    let out = f.run(&[f.id("old.bin"), f.id("cache")], true);
    assert_eq!(out.trashed, 1);
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert!(out.skipped[0].path.ends_with("old.bin") && out.skipped[0].reason.contains("zmieniono"));
    assert!(root.join("old.bin").exists());
    // Without verification (a single item picked by hand) the same file is moved.
    assert_eq!(f.run(&[f.id("old.bin")], false).trashed, 1);
}

#[test]
fn folder_that_gained_a_nested_file_is_skipped() {
    let (f, root) = Fixture::new();
    write(&root.join("docs/deep/new.txt"), 10);
    let out = f.run(&[f.id("docs")], true);
    assert_eq!((out.trashed, out.skipped.len()), (0, 1), "{out:?}");
    assert!(out.skipped[0].reason.contains("nowa zawartość"));
    assert!(root.join("docs/deep/b.txt").exists());
}

#[test]
fn folder_with_a_changed_nested_file_is_skipped_but_a_lost_one_is_fine() {
    let (f, root) = Fixture::new();
    fs::remove_file(root.join("docs/a.txt")).unwrap();
    fs::write(root.join("cache/c.bin"), b"rewritten").unwrap();
    let later = SystemTime::now() + Duration::from_secs(120);
    fs::File::options().write(true).open(root.join("cache/c.bin")).unwrap().set_modified(later).unwrap();
    let out = f.run(&[f.id("docs"), f.id("cache")], true);
    assert_eq!((out.trashed, out.skipped.len()), (1, 1), "{out:?}");
    assert!(out.skipped[0].path.ends_with("cache"));
    assert!(!root.join("docs").exists());
}

#[test]
fn item_inside_another_selected_item_is_moved_once() {
    let (f, _) = Fixture::new();
    let ids = [f.id("docs/deep/b.txt"), f.id("docs"), f.id("docs/deep"), f.id("docs")];
    assert_eq!(top_level(&f.tree.read(), &ids), [f.id("docs")]);
    let summary = check(&f.tree.read(), &ids, None, RULES);
    assert_eq!((summary.count, summary.files), (1, 2));
    let before = f.tree.read().node(ROOT).alloc;
    let out = f.run(&ids, true);
    assert_eq!((out.trashed, out.failed.len(), out.skipped.len()), (1, 0, 0), "{out:?}");
    assert_eq!(f.tree.read().node(ROOT).alloc, before - out.freed);
    // Ids from before the move are ignored afterwards instead of being subtracted again.
    let again = f.run(&ids, true);
    assert_eq!((again.trashed, again.freed), (0, 0));
}

#[test]
fn scan_root_is_refused_and_a_failed_move_is_reported() {
    let (f, root) = Fixture::new();
    let summary = check(&f.tree.read(), &[ROOT, f.id("old.bin")], None, RULES);
    assert_eq!((summary.count, summary.blocked.len()), (0, 1), "the root contains the other item, and it is blocked");
    let out = execute(&f.tree, &[f.id("old.bin")], true, None, RULES, |_| Err("no Trash here".into()));
    assert_eq!((out.trashed, out.failed.len()), (0, 1));
    assert!(root.join("old.bin").exists() && f.tree.read().is_attached(f.id("old.bin")));
}
