//! Moving many items to the Trash in one go, with the checks that make this safe: nothing
//! protected, nothing twice, and (on request) nothing that changed on disk since it was scanned.

use crate::actions::{protection_reason, Rules};
use crate::model::{flags, Tree, NONE, ROOT};
use parking_lot::RwLock;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// More ids than this in one call is a mistake of the caller, not a selection made by hand.
pub const MAX_ITEMS: usize = 5000;

const SAMPLE_LEN: usize = 5;

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Problem {
    pub path: String,
    pub reason: String,
}

impl Problem {
    fn new(path: &Path, reason: impl Into<String>) -> Self {
        Problem { path: path.to_string_lossy().into_owned(), reason: reason.into() }
    }
}

/// What a batch would move, for the confirmation dialog.
#[derive(Serialize, Debug, Default)]
pub struct BatchCheck {
    /// Items that would be moved (after dropping those inside another selected item).
    pub count: u32,
    pub alloc: u64,
    pub files: u64,
    /// Paths of the first few items that would be moved.
    pub sample: Vec<String>,
    /// Selected items that will be left alone, with the reason.
    pub blocked: Vec<Problem>,
}

#[derive(Serialize, Debug, Default)]
pub struct BatchOutcome {
    pub freed: u64,
    pub trashed: u32,
    /// Left alone on purpose: protected, or changed since the scan.
    pub skipped: Vec<Problem>,
    /// The system refused to move them.
    pub failed: Vec<Problem>,
}

/// The ids that are still in the tree, each once, without those that lie inside another one.
pub fn top_level(tree: &Tree, ids: &[u32]) -> Vec<u32> {
    let set: HashSet<u32> = ids.iter().copied().filter(|&id| tree.is_attached(id)).collect();
    let mut seen = HashSet::new();
    ids.iter()
        .copied()
        .filter(|&id| {
            if !set.contains(&id) || !seen.insert(id) {
                return false;
            }
            let mut cur = tree.node(id).parent;
            while cur != NONE {
                if set.contains(&cur) {
                    return false;
                }
                cur = tree.node(cur).parent;
            }
            true
        })
        .collect()
}

fn block_reason(tree: &Tree, id: u32, home: Option<&Path>, rules: &Rules) -> Option<String> {
    if id == ROOT {
        return Some("Nie można przenieść do Kosza korzenia skanu.".into());
    }
    protection_reason(&tree.path(id), &tree.root_path, home, rules)
}

pub fn check(tree: &Tree, ids: &[u32], home: Option<&Path>, rules: &Rules) -> BatchCheck {
    let mut out = BatchCheck::default();
    for id in top_level(tree, ids) {
        if let Some(reason) = block_reason(tree, id, home, rules) {
            out.blocked.push(Problem::new(&tree.path(id), reason));
            continue;
        }
        let n = tree.node(id);
        if out.sample.len() < SAMPLE_LEN {
            out.sample.push(tree.path(id).to_string_lossy().into_owned());
        }
        out.count += 1;
        out.alloc += n.alloc;
        out.files += u64::from(n.files) + u64::from(!n.kind.is_dir());
    }
    out
}

const CHANGED: &str = "zmieniono po skanie";
const NEW_CONTENT: &str = "po skanie pojawiła się nowa zawartość";
const NOT_SCANNED: &str = "zawiera folder, który nie został przeskanowany";

fn mtime_of(meta: &fs::Metadata) -> i64 {
    crate::platform::mtime(meta)
}

/// Checks that what is on disk at `path` is what the scan recorded for `id`: the same files with
/// the same modification times and nothing new. Entries that disappeared since are fine, as
/// moving fewer things than shown is safe.
pub fn verify_unchanged(tree: &Tree, id: u32, path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "już nie istnieje".to_string())?;
    let n = tree.node(id);
    if n.kind.is_dir() != meta.is_dir() {
        return Err(CHANGED.into());
    }
    if !meta.is_dir() {
        let same_size = n.flags & flags::HARDLINK_DUP != 0 || n.logical == meta.len();
        return if mtime_of(&meta) == n.mtime && same_size { Ok(()) } else { Err(CHANGED.into()) };
    }
    if n.flags & (flags::UNREADABLE | flags::OTHER_VOLUME) != 0 {
        return Err(NOT_SCANNED.into());
    }
    let known: HashMap<&str, u32> = tree.children(id).map(|c| (tree.name(c), c)).collect();
    let entries = fs::read_dir(path).map_err(|_| "nie można odczytać folderu".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "nie można odczytać folderu".to_string())?;
        let name = entry.file_name();
        let Some(&child) = name.to_str().and_then(|s| known.get(s)) else {
            return Err(NEW_CONTENT.into());
        };
        verify_unchanged(tree, child, &entry.path())?;
    }
    Ok(())
}

/// Moves the items to the Trash one at a time (so that one failure does not stop the rest) and
/// removes each moved item from the tree. `trash` does the actual move.
pub fn execute(
    tree: &RwLock<Tree>,
    ids: &[u32],
    verify: bool,
    home: Option<&Path>,
    rules: &Rules,
    trash: impl Fn(&Path) -> Result<(), String>,
) -> BatchOutcome {
    let mut out = BatchOutcome::default();
    let ids = top_level(&tree.read(), ids);
    for id in ids {
        let (path, alloc) = {
            let t = tree.read();
            let path = t.path(id);
            let refusal = block_reason(&t, id, home, rules).or_else(|| if verify { verify_unchanged(&t, id, &path).err() } else { None });
            if let Some(reason) = refusal {
                out.skipped.push(Problem::new(&path, reason));
                continue;
            }
            (path, t.node(id).alloc)
        };
        match trash(&path) {
            Ok(()) => {
                tree.write().detach(id);
                out.freed += alloc;
                out.trashed += 1;
            }
            Err(e) => out.failed.push(Problem::new(&path, e)),
        }
    }
    out
}
