//! The last few scans, kept in the app's cache folder. A scan of a folder that has one starts
//! from it and only looks at what changed since (see `refresh`), and the newest one is shown
//! when the app starts.

use crate::model::Tree;
use crate::persist;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Scans of this many folders are kept; the oldest file goes first.
const KEEP: usize = 3;
/// Smaller trees are scanned from scratch faster than a file is worth keeping.
pub const MIN_NODES: usize = 50_000;
const EXTENSION: &str = "disklens";

/// File of the scan of `root`: the name is a stable hash (FNV-1a) of the path.
fn path_for(dir: &Path, root: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.as_os_str().as_encoded_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    dir.join(format!("{hash:016x}.{EXTENSION}"))
}

fn files_newest_first(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == EXTENSION))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|(_, path)| path).collect()
}

/// Writes the tree next to the older scans. The file appears only when it is complete.
pub fn store(dir: &Path, tree: &Tree) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let target = path_for(dir, &tree.root_path);
    let partial = target.with_extension("partial");
    let written = persist::save(tree, &partial).and_then(|()| fs::rename(&partial, &target).map_err(|e| e.to_string()));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    for old in files_newest_first(dir).into_iter().skip(KEEP) {
        let _ = fs::remove_file(old);
    }
    written
}

/// The kept scan of `root`, if there is a readable one.
pub fn load(dir: &Path, root: &Path) -> Option<Tree> {
    let file = path_for(dir, root);
    match persist::load(&file) {
        Ok(tree) if tree.root_path == root => Some(tree),
        Ok(_) => None,
        Err(_) => {
            // Unreadable (damaged, or written by a newer version): it would fail every time.
            let _ = fs::remove_file(file);
            None
        }
    }
}

/// The most recently written scan.
pub fn latest(dir: &Path) -> Option<Tree> {
    files_newest_first(dir).into_iter().find_map(|file| persist::load(&file).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testutil::sample_tree;
    use crate::model::ROOT;

    fn tree_of(root: &str) -> Tree {
        let mut t = sample_tree();
        t.root_path = root.into();
        t
    }

    #[test]
    fn keeps_one_file_per_folder_and_finds_it_again() {
        let dir = tempfile::tempdir().unwrap();
        let scans = dir.path().join("scans");
        assert!(load(&scans, Path::new("/a")).is_none() && latest(&scans).is_none());

        let mut a = tree_of("/a");
        a.state.event_id = 42;
        store(&scans, &a).unwrap();
        store(&scans, &tree_of("/b")).unwrap();
        store(&scans, &a).unwrap();
        assert_eq!(files_newest_first(&scans).len(), 2);
        let loaded = load(&scans, Path::new("/a")).unwrap();
        assert_eq!((loaded.state.event_id, loaded.node(ROOT).alloc), (42, a.node(ROOT).alloc));
        assert!(load(&scans, Path::new("/c")).is_none());
    }

    #[test]
    fn drops_the_oldest_scans_and_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        for (i, root) in ["/a", "/b", "/c", "/d", "/e"].into_iter().enumerate() {
            store(dir.path(), &tree_of(root)).unwrap();
            let file = path_for(dir.path(), Path::new(root));
            let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64);
            fs::File::options().write(true).open(file).unwrap().set_modified(time).unwrap();
        }
        store(dir.path(), &tree_of("/f")).unwrap();
        assert_eq!(files_newest_first(dir.path()).len(), KEEP);
        assert!(load(dir.path(), Path::new("/a")).is_none());
        assert_eq!(latest(dir.path()).unwrap().root_path, PathBuf::from("/f"));

        fs::write(path_for(dir.path(), Path::new("/f")), b"garbage").unwrap();
        assert!(load(dir.path(), Path::new("/f")).is_none());
        assert!(!path_for(dir.path(), Path::new("/f")).exists(), "a damaged file is removed");
        assert_eq!(latest(dir.path()).unwrap().root_path, PathBuf::from("/e"));
    }
}
