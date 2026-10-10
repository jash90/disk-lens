//! Zapytania widoku: dzieci folderu, największe pliki, podział wg typu, filtry.

use crate::category;
use crate::model::{extension_of, Kind, SizeMode, Tree, NONE};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug, Default, Deserialize, Serialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    /// Fragment nazwy pliku (bez rozróżniania wielkości liter).
    pub query: String,
    /// Minimalny rozmiar pliku w bajtach.
    pub min_size: u64,
    /// Tylko te rozszerzenia (puste = wszystkie).
    pub extensions: Vec<String>,
    /// Nazwy folderów/plików pomijanych w całości (np. `node_modules`, `.git`).
    pub excludes: Vec<String>,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty() || self.min_size > 0 || !self.extensions.is_empty() || !self.excludes.is_empty()
    }

    pub fn key(&self, mode: SizeMode) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        mode.hash(&mut h);
        h.finish()
    }
}

/// Sumy policzone z uwzględnieniem filtra – jeden przebieg O(n) od końca areny.
pub struct FilteredSums {
    pub size: Vec<u64>,
    pub files: Vec<u32>,
}

pub fn compute_filtered(tree: &Tree, filter: &Filter, mode: SizeMode) -> FilteredSums {
    let n = tree.len();
    let mut size = vec![0u64; n];
    let mut files = vec![0u32; n];
    let query = filter.query.trim().to_lowercase();
    let exts: Vec<String> = filter.extensions.iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect();
    let excludes: Vec<&str> = filter.excludes.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    for i in (0..n).rev() {
        let node = &tree.nodes[i];
        if !node.is_live() {
            continue;
        }
        let name = tree.name(i as u32);
        if i != 0 && excludes.iter().any(|e| e.eq_ignore_ascii_case(name)) {
            continue;
        }
        if !node.kind.is_dir() {
            let s = node.size(mode);
            let ok = s >= filter.min_size
                && (exts.is_empty() || exts.contains(&extension_of(name)))
                && (query.is_empty() || name.to_lowercase().contains(&query));
            if ok {
                size[i] = s;
                files[i] = 1;
            }
        }
        if node.parent != NONE {
            let p = node.parent as usize;
            size[p] += size[i];
            files[p] += files[i];
        }
    }
    FilteredSums { size, files }
}

/// Widok drzewa w danym trybie rozmiaru, opcjonalnie przefiltrowany.
pub struct View<'a> {
    pub tree: &'a Tree,
    pub mode: SizeMode,
    pub sums: Option<Arc<FilteredSums>>,
}

impl<'a> View<'a> {
    pub fn new(tree: &'a Tree, mode: SizeMode, sums: Option<Arc<FilteredSums>>) -> Self {
        View { tree, mode, sums }
    }

    pub fn size(&self, id: u32) -> u64 {
        match &self.sums {
            Some(s) => s.size[id as usize],
            None => self.tree.node(id).size(self.mode),
        }
    }

    pub fn files(&self, id: u32) -> u32 {
        match &self.sums {
            Some(s) => s.files[id as usize],
            None => self.tree.node(id).files,
        }
    }

    pub fn visible(&self, id: u32) -> bool {
        match &self.sums {
            None => true,
            Some(s) => s.files[id as usize] > 0,
        }
    }

    pub fn visible_children(&self, id: u32) -> Vec<u32> {
        self.tree.children(id).filter(|&c| self.visible(c)).collect()
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    Name,
    #[default]
    Size,
    Files,
    Dirs,
    Mtime,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: u32,
    pub name: String,
    pub kind: &'static str,
    pub flags: u8,
    pub cat: u8,
    pub size: u64,
    pub alloc: u64,
    pub logical: u64,
    pub files: u32,
    pub dirs: u32,
    pub mtime: i64,
    /// Udział w rozmiarze rodzica (0–100).
    pub pct: f64,
    pub has_children: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub total: usize,
    pub parent_size: u64,
    pub rows: Vec<Row>,
}

pub fn row(view: &View, id: u32, parent_size: u64) -> Row {
    let t = view.tree;
    let n = t.node(id);
    let size = view.size(id);
    Row {
        id,
        name: t.name(id).to_string(),
        kind: n.kind.as_str(),
        flags: n.flags,
        cat: n.cat,
        size,
        alloc: n.alloc,
        logical: n.logical,
        files: view.files(id),
        dirs: n.dirs,
        mtime: n.mtime,
        pct: if parent_size > 0 { size as f64 * 100.0 / parent_size as f64 } else { 0.0 },
        has_children: n.kind.is_dir() && n.first_child != NONE && view.files(id) + n.dirs > 0,
    }
}

pub fn sorted_children(view: &View, id: u32, sort: SortKey, desc: bool) -> Vec<u32> {
    let t = view.tree;
    let mut ids = view.visible_children(id);
    match sort {
        SortKey::Name => ids.sort_by_cached_key(|&c| t.name(c).to_lowercase()),
        SortKey::Size => ids.sort_by_key(|&c| view.size(c)),
        SortKey::Files => ids.sort_by_key(|&c| view.files(c)),
        SortKey::Dirs => ids.sort_by_key(|&c| t.node(c).dirs),
        SortKey::Mtime => ids.sort_by_key(|&c| t.node(c).mtime),
    }
    if desc {
        ids.reverse();
    }
    ids
}

pub fn children(view: &View, id: u32, sort: SortKey, desc: bool, offset: usize, limit: usize) -> Page {
    let ids = sorted_children(view, id, sort, desc);
    let parent_size = view.size(id);
    let rows = ids.iter().skip(offset).take(limit).map(|&c| row(view, c, parent_size)).collect();
    Page { total: ids.len(), parent_size, rows }
}

/// Przejście poddrzewa w głąb (bez rekurencji).
pub fn for_each_descendant(tree: &Tree, root: u32, mut f: impl FnMut(u32)) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        f(id);
        stack.extend(tree.children(id));
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub cat: u8,
    pub kind: &'static str,
}

/// Największe pliki w poddrzewie. Pakiety (`.app` itd.) traktujemy jak pojedyncze pliki.
pub fn top_files(view: &View, root: u32, limit: usize) -> Vec<FileRow> {
    let t = view.tree;
    let mut heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::with_capacity(limit + 1);
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let n = t.node(id);
        let leaf = !n.kind.is_dir() || (n.kind == Kind::Package && id != root);
        if leaf {
            if view.visible(id) {
                heap.push(Reverse((view.size(id), id)));
                if heap.len() > limit {
                    heap.pop();
                }
            }
            continue;
        }
        stack.extend(t.children(id).filter(|&c| view.visible(c)));
    }
    let mut out: Vec<_> = heap.into_iter().map(|Reverse(x)| x).collect();
    out.sort_by_key(|x| Reverse(x.0));
    out.into_iter()
        .map(|(size, id)| {
            let n = t.node(id);
            FileRow {
                id,
                name: t.name(id).to_string(),
                path: t.rel_path(root, id),
                size,
                mtime: n.mtime,
                cat: n.cat,
                kind: n.kind.as_str(),
            }
        })
        .collect()
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatStat {
    pub cat: u8,
    pub name: &'static str,
    pub size: u64,
    pub files: u64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtStat {
    pub ext: String,
    pub cat: u8,
    pub size: u64,
    pub files: u64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub total: u64,
    pub categories: Vec<CatStat>,
    pub extensions: Vec<ExtStat>,
}

pub fn type_breakdown(view: &View, root: u32, max_ext: usize) -> Breakdown {
    let t = view.tree;
    let mut cats = [(0u64, 0u64); category::NAMES.len()];
    let mut exts: HashMap<String, (u64, u64)> = HashMap::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let n = t.node(id);
        if n.kind.is_dir() && !(n.kind == Kind::Package && id != root) {
            stack.extend(t.children(id).filter(|&c| view.visible(c)));
            continue;
        }
        if !view.visible(id) {
            continue;
        }
        let s = view.size(id);
        let c = &mut cats[n.cat as usize];
        c.0 += s;
        c.1 += 1;
        let name = t.name(id);
        let ext = extension_of(name);
        let key = if ext.is_empty() { "(bez rozszerzenia)".to_string() } else { ext };
        let e = exts.entry(key).or_default();
        e.0 += s;
        e.1 += 1;
    }
    let mut categories: Vec<CatStat> = cats
        .iter()
        .enumerate()
        .filter(|(_, (s, f))| *s > 0 || *f > 0)
        .map(|(i, &(size, files))| CatStat { cat: i as u8, name: category::NAMES[i], size, files })
        .collect();
    categories.sort_by_key(|c| Reverse(c.size));
    let mut extensions: Vec<ExtStat> = exts
        .into_iter()
        .map(|(ext, (size, files))| ExtStat { cat: category::by_extension(&ext), ext, size, files })
        .collect();
    extensions.sort_by_key(|e| Reverse(e.size));
    extensions.truncate(max_ext);
    Breakdown { total: view.size(root), categories, extensions }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub flags: u8,
    pub cat: u8,
    pub size: u64,
    pub alloc: u64,
    pub logical: u64,
    pub files: u32,
    pub dirs: u32,
    pub mtime: i64,
    pub parent: Option<u32>,
    pub pct_of_root: f64,
}

pub fn node_info(view: &View, id: u32) -> NodeInfo {
    let t = view.tree;
    let n = t.node(id);
    let root_size = view.size(crate::model::ROOT);
    let size = view.size(id);
    NodeInfo {
        id,
        name: t.name(id).to_string(),
        path: t.path(id).to_string_lossy().into_owned(),
        kind: n.kind.as_str(),
        flags: n.flags,
        cat: n.cat,
        size,
        alloc: n.alloc,
        logical: n.logical,
        files: view.files(id),
        dirs: n.dirs,
        mtime: n.mtime,
        parent: (n.parent != NONE).then_some(n.parent),
        pct_of_root: if root_size > 0 { size as f64 * 100.0 / root_size as f64 } else { 0.0 },
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    pub id: u32,
    pub name: String,
}

pub fn crumbs(tree: &Tree, id: u32) -> Vec<Crumb> {
    tree.ancestors(id).into_iter().map(|a| Crumb { id: a, name: tree.name(a).to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NewEntry, ROOT};

    fn e(name: &str, kind: Kind, size: u64) -> NewEntry {
        NewEntry { name: name.into(), kind, flags: 0, cat: 0, alloc: size, logical: size, mtime: 0 }
    }

    fn sample() -> Tree {
        let mut t = Tree::new("/r".into(), e("", Kind::Dir, 0), 0);
        let ids = t.add_children(ROOT, vec![e("node_modules", Kind::Dir, 0), e("a.mp4", Kind::File, 1000), e("src", Kind::Dir, 0)]);
        t.add_children(ids[0], vec![e("big.js", Kind::File, 5000)]);
        t.add_children(ids[2], vec![e("main.rs", Kind::File, 10), e("clip.MP4", Kind::File, 300)]);
        t
    }

    #[test]
    fn filter_excludes_and_extensions() {
        let t = sample();
        let f = Filter { excludes: vec!["node_modules".into()], ..Default::default() };
        let s = compute_filtered(&t, &f, SizeMode::Alloc);
        assert_eq!(s.size[0], 1310);
        assert_eq!(s.files[0], 3);

        let f = Filter { extensions: vec!["mp4".into()], ..Default::default() };
        let s = Arc::new(compute_filtered(&t, &f, SizeMode::Alloc));
        assert_eq!(s.size[0], 1300);
        let v = View::new(&t, SizeMode::Alloc, Some(s));
        let page = children(&v, ROOT, SortKey::Size, true, 0, 10);
        assert_eq!(page.total, 2, "node_modules nie ma plików mp4, więc znika");
        assert_eq!(page.rows[0].name, "a.mp4");
    }

    #[test]
    fn top_files_sorted() {
        let t = sample();
        let v = View::new(&t, SizeMode::Alloc, None);
        let top = top_files(&v, ROOT, 2);
        assert_eq!(top.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["big.js", "a.mp4"]);
        assert_eq!(top[0].path, "node_modules/big.js");
    }
}
