//! Drzewo wyników skanu jako arena: węzły w jednym `Vec`, powiązania przez indeksy `u32`.
//!
//! Dzieci zawsze mają większy indeks niż rodzic (dodawane są po nim), co pozwala liczyć
//! sumy przefiltrowane jednym przebiegiem od końca tablicy.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const NONE: u32 = u32::MAX;
pub const ROOT: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Kind {
    File,
    Dir,
    /// Pakiet macOS (`.app`, `.photoslibrary`…): skanowany w całości, ale prezentowany jako jeden węzeł.
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
    /// Placeholder iCloud (SF_DATALESS) – zawartość nie jest na dysku.
    pub const DATALESS: u8 = 1 << 1;
    /// Nie udało się odczytać zawartości folderu.
    pub const UNREADABLE: u8 = 1 << 2;
    /// Kolejne twarde dowiązanie do już policzonego i-węzła (rozmiar liczony jako 0).
    pub const HARDLINK_DUP: u8 = 1 << 3;
    /// Punkt montowania innego wolumenu – nie wchodzimy do środka.
    pub const OTHER_VOLUME: u8 = 1 << 4;
    /// Węzeł usunięty z drzewa (kosz / reskan); zostaje w arenie jako nieosiągalny.
    pub const REMOVED: u8 = 1 << 5;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SizeMode {
    /// Miejsce faktycznie zajęte na dysku (`st_blocks * 512`).
    #[default]
    Alloc,
    /// Rozmiar logiczny (`st_size`).
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
    /// Dla folderów: suma poddrzewa łącznie z blokami samego folderu.
    pub alloc: u64,
    pub logical: u64,
    /// Liczba plików (nie-folderów) w poddrzewie, bez samego węzła.
    pub files: u32,
    /// Liczba folderów w poddrzewie, bez samego węzła.
    pub dirs: u32,
    pub mtime: i64,
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

/// Wpis gotowy do wstawienia do drzewa (wynik `lstat`).
#[derive(Clone, Debug)]
pub struct NewEntry {
    pub name: String,
    pub kind: Kind,
    pub flags: u8,
    pub cat: u8,
    pub alloc: u64,
    pub logical: u64,
    pub mtime: i64,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    pub names: String,
    pub skipped: Vec<Skipped>,
    /// Unix timestamp (s) rozpoczęcia skanu.
    pub scanned_at: i64,
    /// Rośnie przy każdej zmianie – unieważnia cache filtrów.
    #[serde(skip)]
    pub generation: u64,
}

impl Tree {
    /// Nowe drzewo z samym korzeniem. Nazwą korzenia jest pełna ścieżka.
    pub fn new(root_path: PathBuf, mut root: NewEntry, scanned_at: i64) -> Self {
        root.name = root_path.to_string_lossy().into_owned();
        let mut t = Tree {
            root_path,
            nodes: Vec::new(),
            names: String::new(),
            skipped: Vec::new(),
            scanned_at,
            generation: 0,
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
        // Nazwy w APFS/HFS+ mają ≤ 255 znaków UTF-8, więc u16 wystarcza z zapasem.
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
        });
        if parent != NONE {
            self.nodes[parent as usize].first_child = id;
        }
        id
    }

    /// Wstawia dzieci folderu i dolicza ich sumy do wszystkich przodków. Zwraca indeksy w kolejności wejścia.
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

    /// Dolicza (`add = true`) lub odejmuje sumy od węzła `from` i wszystkich jego przodków.
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

    /// Łańcuch od korzenia do `id` włącznie.
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

    /// Ścieżka względem `base` (np. bieżącego korzenia widoku).
    pub fn rel_path(&self, base: u32, id: u32) -> String {
        let chain = self.ancestors(id);
        let start = chain.iter().position(|&a| a == base).map(|i| i + 1).unwrap_or(1);
        chain[start..]
            .iter()
            .map(|&a| self.name(a))
            .collect::<Vec<_>>()
            .join("/")
    }

    pub fn is_ancestor_or_self(&self, ancestor: u32, id: u32) -> bool {
        let mut cur = id;
        while cur != NONE {
            if cur == ancestor {
                return true;
            }
            cur = self.node(cur).parent;
        }
        false
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

    /// Odpina węzeł (np. po przeniesieniu do Kosza) i odejmuje jego rozmiar od przodków.
    pub fn detach(&mut self, id: u32) {
        let parent = self.node(id).parent;
        if parent == NONE || !self.node(id).is_live() {
            return;
        }
        let t = self.subtree_totals(id);
        // Usunięcie z listy jednokierunkowej rodzeństwa.
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

    /// Usuwa wszystkie dzieci folderu (przed reskanem). Sam folder zostaje z rozmiarem własnych bloków.
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

    /// Nadpisuje „własne” metadane folderu (po ponownym `lstat` przy reskanie).
    pub fn set_own(&mut self, id: u32, alloc: u64, logical: u64, mtime: i64) {
        let n = self.node(id);
        let (old_alloc, old_logical) = (n.alloc, n.logical);
        // Po clear_children rozmiar węzła = jego własne bloki.
        let t_old = Totals { alloc: old_alloc, logical: old_logical, files: 0, dirs: 0 };
        let parent = n.parent;
        self.nodes[id as usize].alloc = alloc;
        self.nodes[id as usize].logical = logical;
        self.nodes[id as usize].mtime = mtime;
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

/// Rozszerzenie pliku małymi literami (bez kropki); pusty napis, gdy brak.
pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: Kind, alloc: u64) -> NewEntry {
        NewEntry { name: name.into(), kind, flags: 0, cat: 0, alloc, logical: alloc, mtime: 0 }
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
