//! The scan result tree as an arena: nodes in one `Vec`, linked by `u32` indices.
//!
//! Children always have a larger index than their parent (they are added after it), which lets
//! per-subtree values be computed in one pass from the end of the array.
//!
//! `Tree`, `Node`, `Kind` and `Skipped` are written to `.disklens` files as they are (see
//! `persist`): changing their fields, field order or variant order changes the file format.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const NONE: u32 = u32::MAX;
pub const ROOT: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Kind {
    File,
    Dir,
    /// macOS package (`.app`, `.photoslibrary`…): scanned in full, but presented as one node.
    Package,
    Symlink,
    Other,
}

impl Kind {
    pub fn is_dir(self) -> bool {
        matches!(self, Kind::Dir | Kind::Package)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Dir => "dir",
            Kind::Package => "package",
            Kind::Symlink => "symlink",
            Kind::Other => "other",
        }
    }
}

pub mod flags {
    pub const HIDDEN: u8 = 1 << 0;
    /// iCloud placeholder (SF_DATALESS): the content is not on disk.
    pub const DATALESS: u8 = 1 << 1;
    /// The folder's content could not be read.
    pub const UNREADABLE: u8 = 1 << 2;
    /// Another hard link to an inode already counted (its size counts as 0).
    pub const HARDLINK_DUP: u8 = 1 << 3;
    /// Mount point of another volume: not entered.
    pub const OTHER_VOLUME: u8 = 1 << 4;
    /// Node removed from the tree (Trash / rescan); it stays in the arena, unreachable.
    pub const REMOVED: u8 = 1 << 5;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SizeMode {
    /// Space actually allocated on disk (`st_blocks * 512`).
    #[default]
    Alloc,
    /// Logical size (`st_size`).
    Logical,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub parent: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    pub name_off: u32,
    pub name_len: u16,
    pub kind: Kind,
    pub flags: u8,
    pub cat: u8,
    /// For folders: the subtree total including the folder's own blocks.
    pub alloc: u64,
    pub logical: u64,
    /// Number of files (non-folders) in the subtree, without the node itself.
    pub files: u32,
    /// Number of folders in the subtree, without the node itself.
    pub dirs: u32,
    /// Own modification time, Unix seconds; 0 when unknown.
    pub mtime: i64,
    /// Own last access time, Unix seconds; 0 when unknown.
    pub atime: u32,
    /// When the item appeared on this disk: the newest of its creation time and, where the
    /// system reports it, the time it was added to its folder. Unix seconds; 0 when unknown.
    pub created: u32,
}

impl Node {
    pub fn size(&self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Alloc => self.alloc,
            SizeMode::Logical => self.logical,
        }
    }

    pub fn is_live(&self) -> bool {
        self.flags & flags::REMOVED == 0
    }
}

/// An entry ready to be inserted into the tree (the result of `lstat`).
#[derive(Clone, Debug)]
pub struct NewEntry {
    pub name: String,
    pub kind: Kind,
    pub flags: u8,
    pub cat: u8,
    pub alloc: u64,
    pub logical: u64,
    pub mtime: i64,
    pub atime: u32,
    pub created: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub path: String,
    pub error: String,
}

#[derive(Clone, Copy, Default, Debug)]
struct Totals {
    alloc: u64,
    logical: u64,
    files: u32,
    dirs: u32,
}

/// One name of a file that has several hard links.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Link {
    pub dev: u64,
    pub ino: u64,
    pub id: u32,
}

/// What an incremental refresh needs to know about the scan that built the tree. Saved next to
/// the tree (format version 3), not inside it, so that the tree's own layout stays as it was.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanState {
    /// Every node that is one of several hard links to a file. Entries of removed nodes may linger.
    pub links: Vec<Link>,
    /// Unix time of the last incremental refresh; 0 when the tree comes from a full scan only.
    pub refreshed_at: i64,
    /// File-system event id taken right before the tree was last brought up to date; 0 = none.
    pub event_id: u64,
    /// Identity of the event log `event_id` belongs to; ids of another log mean nothing.
    pub event_uuid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    pub names: String,
    pub skipped: Vec<Skipped>,
    /// Unix timestamp (s) of the start of the scan.
    pub scanned_at: i64,
    /// Grows on every change: invalidates the per-view caches.
    #[serde(skip)]
    pub generation: u64,
    #[serde(skip)]
    pub state: ScanState,
}

impl Tree {
    /// A new tree with only the root. The root's name is the full path.
    pub fn new(root_path: PathBuf, mut root: NewEntry, scanned_at: i64) -> Self {
        root.name = root_path.to_string_lossy().into_owned();
        let mut t = Tree {
            root_path,
            nodes: Vec::new(),
            names: String::new(),
            skipped: Vec::new(),
            scanned_at,
            generation: 0,
            state: ScanState::default(),
        };
        t.push_node(NONE, root);
        t
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn get(&self, id: u32) -> Option<&Node> {
        self.nodes.get(id as usize).filter(|n| n.is_live())
    }

    pub fn name(&self, id: u32) -> &str {
        let n = self.node(id);
        let start = n.name_off as usize;
        &self.names[start..start + n.name_len as usize]
    }

    fn push_node(&mut self, parent: u32, e: NewEntry) -> u32 {
        let id = self.nodes.len() as u32;
        let name_off = self.names.len() as u32;
        // Names on APFS/HFS+ have ≤ 255 UTF-8 characters, so u16 is enough with room to spare.
        let name = truncate_utf8(&e.name, u16::MAX as usize);
        self.names.push_str(name);
        let next_sibling = if parent == NONE {
            NONE
        } else {
            self.nodes[parent as usize].first_child
        };
        self.nodes.push(Node {
            parent,
            first_child: NONE,
            next_sibling,
            name_off,
            name_len: name.len() as u16,
            kind: e.kind,
            flags: e.flags,
            cat: e.cat,
            alloc: e.alloc,
            logical: e.logical,
            files: 0,
            dirs: 0,
            mtime: e.mtime,
            atime: e.atime,
            created: e.created,
        });
        if parent != NONE {
            self.nodes[parent as usize].first_child = id;
        }
        id
    }

    /// Inserts the children of a folder and adds their totals to all ancestors. Returns ids in input order.
    pub fn add_children(&mut self, parent: u32, entries: Vec<NewEntry>) -> Vec<u32> {
        let mut t = Totals::default();
        let mut ids = Vec::with_capacity(entries.len());
        for e in entries {
            t.alloc += e.alloc;
            t.logical += e.logical;
            if e.kind.is_dir() {
                t.dirs += 1;
            } else {
                t.files += 1;
            }
            ids.push(self.push_node(parent, e));
        }
        self.apply(parent, t, true);
        ids
    }

    /// Adds (`add = true`) or subtracts totals on the node `from` and all its ancestors.
    fn apply(&mut self, from: u32, t: Totals, add: bool) {
        let mut cur = from;
        while cur != NONE {
            let n = &mut self.nodes[cur as usize];
            if add {
                n.alloc += t.alloc;
                n.logical += t.logical;
                n.files += t.files;
                n.dirs += t.dirs;
            } else {
                n.alloc = n.alloc.saturating_sub(t.alloc);
                n.logical = n.logical.saturating_sub(t.logical);
                n.files = n.files.saturating_sub(t.files);
                n.dirs = n.dirs.saturating_sub(t.dirs);
            }
            cur = n.parent;
        }
        self.generation += 1;
    }

    pub fn children(&self, id: u32) -> ChildIter<'_> {
        ChildIter {
            tree: self,
            next: self.node(id).first_child,
        }
    }

    /// The chain from the root down to `id`, inclusive.
    pub fn ancestors(&self, id: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = id;
        while cur != NONE {
            out.push(cur);
            cur = self.node(cur).parent;
        }
        out.reverse();
        out
    }

    pub fn path(&self, id: u32) -> PathBuf {
        let chain = self.ancestors(id);
        let mut p = self.root_path.clone();
        for &a in chain.iter().skip(1) {
            p.push(self.name(a));
        }
        p
    }

    /// Path relative to `base` (e.g. the current view root).
    pub fn rel_path(&self, base: u32, id: u32) -> String {
        let chain = self.ancestors(id);
        let start = chain.iter().position(|&a| a == base).map(|i| i + 1).unwrap_or(1);
        chain[start..]
            .iter()
            .map(|&a| self.name(a))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Whether `id` is a node that is still part of the tree: neither it nor any folder above
    /// it was removed. (`detach` flags only the top of the removed subtree.)
    pub fn is_attached(&self, id: u32) -> bool {
        if (id as usize) >= self.nodes.len() {
            return false;
        }
        let mut cur = id;
        while cur != NONE {
            if !self.node(cur).is_live() {
                return false;
            }
            cur = self.node(cur).parent;
        }
        true
    }

    fn subtree_totals(&self, id: u32) -> Totals {
        let n = self.node(id);
        let is_dir = n.kind.is_dir();
        Totals {
            alloc: n.alloc,
            logical: n.logical,
            files: n.files + u32::from(!is_dir),
            dirs: n.dirs + u32::from(is_dir),
        }
    }

    /// Number of nodes reachable from the root; the rest of the arena is left over from removals.
    pub fn live_len(&self) -> usize {
        let mut count = 0;
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            count += 1;
            stack.extend(self.children(id));
        }
        count
    }

    /// A copy without the nodes that were removed. Node ids change; the order of children and
    /// the rule that a child's id is larger than its parent's are kept.
    pub fn compacted(&self) -> Tree {
        let mut out = Tree {
            root_path: self.root_path.clone(),
            nodes: Vec::new(),
            names: String::new(),
            skipped: self.skipped.clone(),
            scanned_at: self.scanned_at,
            generation: self.generation + 1,
            state: ScanState { links: Vec::new(), ..self.state.clone() },
        };
        let mut new_id = vec![NONE; self.nodes.len()];
        let copy = |out: &mut Tree, old: u32, parent: u32| -> u32 {
            let id = out.nodes.len() as u32;
            let name = self.name(old);
            let node = Node { parent, first_child: NONE, next_sibling: NONE, name_off: out.names.len() as u32, ..self.node(old).clone() };
            out.names.push_str(name);
            out.nodes.push(node);
            id
        };
        new_id[ROOT as usize] = copy(&mut out, ROOT, NONE);
        let mut queue = std::collections::VecDeque::from([ROOT]);
        while let Some(old_parent) = queue.pop_front() {
            let parent = new_id[old_parent as usize];
            let mut prev = NONE;
            for child in self.children(old_parent) {
                let id = copy(&mut out, child, parent);
                if prev == NONE {
                    out.nodes[parent as usize].first_child = id;
                } else {
                    out.nodes[prev as usize].next_sibling = id;
                }
                prev = id;
                new_id[child as usize] = id;
                if self.node(child).first_child != NONE {
                    queue.push_back(child);
                }
            }
        }
        out.state.links = self.state.links.iter().filter(|l| new_id[l.id as usize] != NONE).map(|l| Link { id: new_id[l.id as usize], ..*l }).collect();
        out.state.links.sort_unstable();
        out.state.links.dedup();
        out
    }

    /// Detaches a node (e.g. after moving it to the Trash) and subtracts its size from the ancestors.
    pub fn detach(&mut self, id: u32) {
        let parent = self.node(id).parent;
        // A node under an already removed folder was subtracted together with that folder.
        if parent == NONE || !self.is_attached(id) {
            return;
        }
        let t = self.subtree_totals(id);
        // Removal from the singly linked list of siblings.
        let next = self.node(id).next_sibling;
        if self.node(parent).first_child == id {
            self.nodes[parent as usize].first_child = next;
        } else {
            let mut cur = self.node(parent).first_child;
            while cur != NONE {
                if self.node(cur).next_sibling == id {
                    self.nodes[cur as usize].next_sibling = next;
                    break;
                }
                cur = self.node(cur).next_sibling;
            }
        }
        self.nodes[id as usize].flags |= flags::REMOVED;
        self.apply(parent, t, false);
    }

    /// Removes all children of a folder (before a rescan). The folder keeps the size of its own blocks.
    pub fn clear_children(&mut self, id: u32) {
        let mut t = Totals::default();
        let mut cur = self.node(id).first_child;
        while cur != NONE {
            let c = self.subtree_totals(cur);
            t.alloc += c.alloc;
            t.logical += c.logical;
            t.files += c.files;
            t.dirs += c.dirs;
            self.nodes[cur as usize].flags |= flags::REMOVED;
            cur = self.node(cur).next_sibling;
        }
        self.nodes[id as usize].first_child = NONE;
        self.nodes[id as usize].flags &= !flags::UNREADABLE;
        self.apply(id, t, false);
    }

    pub fn find_child(&self, parent: u32, name: &str) -> Option<u32> {
        self.children(parent).find(|&c| self.name(c) == name)
    }

    /// The deepest node on the way to `path` that is in the tree, and whether it is `path` itself.
    /// `None` for a path outside the scan root.
    pub fn resolve(&self, path: &std::path::Path) -> Option<(u32, bool)> {
        let rel = path.strip_prefix(&self.root_path).ok()?;
        let mut cur = ROOT;
        for part in rel.components() {
            let name = part.as_os_str().to_string_lossy();
            match self.find_child(cur, &name) {
                Some(c) => cur = c,
                None => return Some((cur, false)),
            }
        }
        Some((cur, true))
    }

    /// The node's own size: for a folder its total minus the totals of its children.
    fn own_size(&self, id: u32) -> (u64, u64) {
        let n = self.node(id);
        self.children(id).fold((n.alloc, n.logical), |(a, l), c| {
            let c = self.node(c);
            (a.saturating_sub(c.alloc), l.saturating_sub(c.logical))
        })
    }

    /// Brings the node's own sizes, times and the hidden/iCloud flags in line with `own`, keeping
    /// its children, and passes the size difference up. A further hard link stays at size 0.
    /// With `keep_created` the creation time only moves forward (for sources that do not report
    /// when the item was added to its folder). Returns whether anything changed.
    pub fn update_own(&mut self, id: u32, own: &NewEntry, keep_created: bool) -> bool {
        const OWN_FLAGS: u8 = flags::HIDDEN | flags::DATALESS;
        let is_dup = self.node(id).flags & flags::HARDLINK_DUP != 0;
        let (new_alloc, new_logical) = if is_dup { (0, 0) } else { (own.alloc, own.logical) };
        let (old_alloc, old_logical) = self.own_size(id);
        let n = &mut self.nodes[id as usize];
        let created = if keep_created { n.created.max(own.created) } else { own.created };
        let new_flags = (n.flags & !OWN_FLAGS) | (own.flags & OWN_FLAGS);
        let same = (old_alloc, old_logical, n.mtime, n.atime, n.created, n.flags) == (new_alloc, new_logical, own.mtime, own.atime, created, new_flags);
        if same {
            return false;
        }
        (n.mtime, n.atime, n.created, n.flags) = (own.mtime, own.atime, created, new_flags);
        self.apply(id, Totals { alloc: old_alloc, logical: old_logical, files: 0, dirs: 0 }, false);
        self.apply(id, Totals { alloc: new_alloc, logical: new_logical, files: 0, dirs: 0 }, true);
        true
    }

    /// Makes a hard link the one that carries the file's size (`Some(sizes)`) or a further link
    /// that counts as 0 (`None`), and passes the difference up.
    pub fn set_link_primary(&mut self, id: u32, sizes: Option<(u64, u64)>) {
        let n = &mut self.nodes[id as usize];
        let old = Totals { alloc: n.alloc, logical: n.logical, files: 0, dirs: 0 };
        let (alloc, logical) = sizes.unwrap_or((0, 0));
        if sizes.is_some() {
            n.flags &= !flags::HARDLINK_DUP;
        } else {
            n.flags |= flags::HARDLINK_DUP;
        }
        (n.alloc, n.logical) = (alloc, logical);
        let parent = n.parent;
        if parent != NONE {
            self.apply(parent, old, false);
            self.apply(parent, Totals { alloc, logical, files: 0, dirs: 0 }, true);
        }
    }

    /// Overwrites the node's own metadata (after a fresh `lstat` during a rescan). Only the
    /// sizes and times of `own` are used.
    pub fn set_own(&mut self, id: u32, own: &NewEntry) {
        let (alloc, logical) = (own.alloc, own.logical);
        let n = self.node(id);
        let (old_alloc, old_logical) = (n.alloc, n.logical);
        // After clear_children the node's size is its own blocks.
        let t_old = Totals { alloc: old_alloc, logical: old_logical, files: 0, dirs: 0 };
        let parent = n.parent;
        self.nodes[id as usize].alloc = alloc;
        self.nodes[id as usize].logical = logical;
        self.nodes[id as usize].mtime = own.mtime;
        self.nodes[id as usize].atime = own.atime;
        self.nodes[id as usize].created = own.created;
        if parent != NONE {
            self.apply(parent, t_old, false);
            self.apply(parent, Totals { alloc, logical, files: 0, dirs: 0 }, true);
        }
        self.generation += 1;
    }
}

pub struct ChildIter<'a> {
    tree: &'a Tree,
    next: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.next == NONE {
            return None;
        }
        let id = self.next;
        self.next = self.tree.node(id).next_sibling;
        Some(id)
    }
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// File extension in lower case (without the dot); empty when there is none.
pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Builders shared by the unit tests of several modules.
#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub fn entry(name: &str, kind: Kind, size: u64) -> NewEntry {
        NewEntry { name: name.into(), kind, flags: 0, cat: 0, alloc: size, logical: size, mtime: 0, atime: 0, created: 0 }
    }

    pub fn entry_at(name: &str, kind: Kind, size: u64, mtime: i64) -> NewEntry {
        NewEntry { mtime, ..entry(name, kind, size) }
    }

    /// `/r` with `node_modules/big.js` (5000), `a.mp4` (1000), `src/main.rs` (10), `src/clip.MP4` (300).
    pub fn sample_tree() -> Tree {
        let mut t = Tree::new("/r".into(), entry("", Kind::Dir, 0), 0);
        let ids = t.add_children(
            ROOT,
            vec![
                entry_at("node_modules", Kind::Dir, 0, 300),
                entry_at("a.mp4", Kind::File, 1000, 100),
                entry_at("src", Kind::Dir, 0, 200),
            ],
        );
        t.add_children(ids[0], vec![entry("big.js", Kind::File, 5000)]);
        t.add_children(ids[2], vec![entry("main.rs", Kind::File, 10), entry("clip.MP4", Kind::File, 300)]);
        t
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::entry;
    use super::*;

    #[test]
    fn detaching_below_a_removed_folder_subtracts_nothing() {
        let mut t = Tree::new("/x".into(), entry("", Kind::Dir, 0), 0);
        let a = t.add_children(ROOT, vec![entry("a", Kind::Dir, 0), entry("keep", Kind::File, 1)])[0];
        let g = t.add_children(a, vec![entry("g", Kind::File, 50)])[0];
        t.detach(a);
        assert!(!t.is_attached(a) && !t.is_attached(g) && t.is_attached(ROOT));
        t.detach(g);
        assert_eq!((t.node(ROOT).alloc, t.node(ROOT).files), (1, 1));
    }

    #[test]
    fn compacting_drops_removed_nodes_and_keeps_the_rest() {
        let mut t = super::testutil::sample_tree();
        let src = t.find_child(ROOT, "src").unwrap();
        let main = t.find_child(src, "main.rs").unwrap();
        t.state.links = vec![Link { dev: 1, ino: 2, id: main }, Link { dev: 1, ino: 2, id: t.find_child(ROOT, "a.mp4").unwrap() }];
        let modules = t.find_child(ROOT, "node_modules").unwrap();
        t.detach(modules);
        t.detach(t.state.links[1].id);
        assert_eq!((t.len(), t.live_len()), (7, 4));

        let c = t.compacted();
        assert_eq!((c.len(), c.live_len()), (4, 4));
        let (a, b) = (t.node(ROOT), c.node(ROOT));
        assert_eq!((a.alloc, a.files, a.dirs), (b.alloc, b.files, b.dirs));
        let src = c.find_child(ROOT, "src").unwrap();
        let names: Vec<&str> = c.children(src).map(|i| c.name(i)).collect();
        let old_names: Vec<&str> = t.children(t.find_child(ROOT, "src").unwrap()).map(|i| t.name(i)).collect();
        assert_eq!(names, old_names);
        assert_eq!(c.path(c.find_child(src, "main.rs").unwrap()), PathBuf::from("/r/src/main.rs"));
        assert_eq!(c.state.links, vec![Link { dev: 1, ino: 2, id: c.find_child(src, "main.rs").unwrap() }]);
        assert!((1..c.len() as u32).all(|i| c.node(i).parent < i));
    }

    #[test]
    fn totals_propagate_and_detach() {
        let mut t = Tree::new("/x".into(), entry("", Kind::Dir, 10), 0);
        let ids = t.add_children(ROOT, vec![entry("a", Kind::Dir, 5), entry("f", Kind::File, 100)]);
        let a = ids[0];
        t.add_children(a, vec![entry("g", Kind::File, 50), entry("h", Kind::File, 7)]);
        assert_eq!(t.node(ROOT).alloc, 10 + 5 + 100 + 50 + 7);
        assert_eq!(t.node(ROOT).files, 3);
        assert_eq!(t.node(ROOT).dirs, 1);
        assert_eq!(t.node(a).alloc, 62);
        assert_eq!(t.path(a), PathBuf::from("/x/a"));

        t.detach(a);
        assert_eq!(t.node(ROOT).alloc, 110);
        assert_eq!(t.node(ROOT).files, 1);
        assert_eq!(t.node(ROOT).dirs, 0);
        assert_eq!(t.children(ROOT).count(), 1);
    }

    #[test]
    fn clear_children_keeps_own_size() {
        let mut t = Tree::new("/x".into(), entry("", Kind::Dir, 10), 0);
        let a = t.add_children(ROOT, vec![entry("a", Kind::Dir, 5)])[0];
        t.add_children(a, vec![entry("g", Kind::File, 50)]);
        t.clear_children(a);
        assert_eq!(t.node(a).alloc, 5);
        assert_eq!(t.node(ROOT).alloc, 15);
        assert_eq!(t.node(ROOT).files, 0);
    }

    #[test]
    fn extensions() {
        assert_eq!(extension_of("a.MP4"), "mp4");
        assert_eq!(extension_of(".zshrc"), "");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("a.tar.gz"), "gz");
    }
}
