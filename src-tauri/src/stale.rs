//! Finding things nobody has used for a long time: stale files, stale folders and the build or
//! cache folders of projects that are no longer worked on.
//!
//! "Activity" of an item is the newest of its modification time, the time it appeared on this
//! disk and, optionally, its last access. A time of 0 means "unknown", never "old": an item
//! with an unknown time is never reported, and neither is a folder that contains one.

use crate::actions::Protected;
use crate::category;
use crate::model::{flags, Kind, Node, Tree, NONE};
use crate::query::{is_leaf, View};
use crate::treemap::age_bucket;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

pub const SECS_PER_DAY: i64 = 86_400;

/// Marks a subtree that must never be reported as stale.
const NEVER_STALE: i64 = i64::MAX;

/// Flags that make an item, and every folder above it, unsafe to judge or to trash as a whole:
/// content that was not read, content of another volume, content that lives only in iCloud.
const DISQUALIFYING: u8 = flags::UNREADABLE | flags::OTHER_VOLUME | flags::DATALESS;

/// Folders that tools recreate on demand. `target` and `Pods` are too common as plain names,
/// so they count only next to the file that marks their project.
const CACHE_DIRS: &[(&str, Option<&str>)] = &[
    ("node_modules", None),
    (".gradle", None),
    ("__pycache__", None),
    (".next", None),
    (".nuxt", None),
    (".tox", None),
    (".pytest_cache", None),
    (".mypy_cache", None),
    ("target", Some("Cargo.toml")),
    ("Pods", Some("Podfile")),
];

/// Xcode keeps one build folder per project under this one; each is reported on its own.
const PER_PROJECT_CACHE_PARENT: &str = "DerivedData";

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ListKind {
    Files,
    Folders,
    Caches,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StaleSort {
    Size,
    Age,
    Path,
    /// Size multiplied by age: large and old first.
    Score,
}

#[derive(Clone, Debug, Deserialize, Hash)]
#[serde(rename_all = "camelCase")]
pub struct StaleArgs {
    pub list: ListKind,
    pub older_than_days: u32,
    /// Count the last access as activity (files only; less reliable than the other times).
    pub use_access: bool,
    /// Include application bundles (and the folders that hold them), which look stale by their
    /// dates although they are in use.
    pub show_apps: bool,
    pub sort: StaleSort,
    pub desc: bool,
}

impl StaleArgs {
    pub fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

/// The node's own activity, or `None` when its modification time is unknown.
fn own_activity(n: &Node, use_access: bool) -> Option<i64> {
    if n.mtime <= 0 {
        return None;
    }
    let mut t = n.mtime.max(i64::from(n.created));
    // A folder's access time says nothing: listing it (as a scan does) updates it.
    if use_access && !n.kind.is_dir() {
        t = t.max(i64::from(n.atime));
    }
    Some(t)
}

fn is_app(n: &Node) -> bool {
    n.kind == Kind::Package && n.cat == category::APP
}

/// For every node, the newest activity anywhere in its subtree.
pub struct StaleIndex {
    newest: Vec<i64>,
}

impl StaleIndex {
    /// One pass from the end of the arena: children come after their parents, so every node is
    /// complete by the time it is folded into its parent.
    ///
    /// Without `show_apps` an application bundle counts as "never stale", and so does every
    /// folder that holds one: an installed app looks old by its dates although it is in use.
    pub fn compute(tree: &Tree, use_access: bool, show_apps: bool) -> Self {
        let mut newest = vec![0i64; tree.len()];
        for i in (0..tree.len()).rev() {
            let n = &tree.nodes[i];
            if !n.is_live() {
                continue;
            }
            let hidden_app = !show_apps && is_app(n);
            let own = match own_activity(n, use_access) {
                Some(t) if n.flags & DISQUALIFYING == 0 && !hidden_app => t,
                _ => NEVER_STALE,
            };
            newest[i] = newest[i].max(own);
            if n.parent != NONE {
                let p = n.parent as usize;
                newest[p] = newest[p].max(newest[i]);
            }
        }
        StaleIndex { newest }
    }

    /// Newest activity in the subtree of `id`; `None` when the subtree must not be judged.
    pub fn newest(&self, id: u32) -> Option<i64> {
        let t = self.newest[id as usize];
        (t != NEVER_STALE).then_some(t)
    }

    fn older_than(&self, id: u32, cutoff: i64) -> bool {
        self.newest[id as usize] < cutoff
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Match {
    pub id: u32,
    pub size: u64,
    /// Newest activity in the item (its subtree, for folders).
    pub activity: i64,
}

#[derive(Default)]
pub struct StaleResult {
    pub total_size: u64,
    pub matches: Vec<Match>,
}

/// The requested list under `root`, sorted. `now` is the Unix time the cutoff counts back from.
pub fn find(view: &View, root: u32, args: &StaleArgs, index: &StaleIndex, protected: &Protected, now: i64) -> StaleResult {
    let cutoff = now - i64::from(args.older_than_days) * SECS_PER_DAY;
    let mut matches = match args.list {
        ListKind::Files => stale_files(view, root, index, protected, cutoff),
        ListKind::Folders => stale_folders(view, root, index, protected, cutoff),
        ListKind::Caches => stale_caches(view, root, index, protected, cutoff),
    };
    sort(view.tree, root, &mut matches, args.sort, now);
    if args.desc {
        matches.reverse();
    }
    StaleResult { total_size: matches.iter().map(|m| m.size).sum(), matches }
}

/// Depth-first walk over the visible folders under `root`. `visit` gets each child of a visited
/// folder with the protection zone it lies in, and returns whether to descend into it.
fn walk(view: &View, root: u32, protected: &Protected, mut visit: impl FnMut(u32, u32, bool) -> bool) {
    let tree = view.tree;
    let mut stack = vec![(root, protected.zone_of_children(tree, root))];
    while let Some((dir, zone)) = stack.pop() {
        for child in tree.children(dir).filter(|&c| view.visible(c)) {
            if visit(dir, child, zone) {
                stack.push((child, protected.zone_below(child, zone)));
            }
        }
    }
}

fn stale_files(view: &View, root: u32, index: &StaleIndex, protected: &Protected, cutoff: i64) -> Vec<Match> {
    let tree = view.tree;
    let mut out = Vec::new();
    walk(view, root, protected, |_, id, zone| {
        if !is_leaf(tree, id, root) {
            return true;
        }
        // A further hard link frees nothing when trashed.
        let listed = tree.node(id).flags & flags::HARDLINK_DUP == 0 && !protected.is_protected(id, zone);
        if listed && index.older_than(id, cutoff) {
            out.push(Match { id, size: view.size(id), activity: index.newest[id as usize] });
        }
        false
    });
    out
}

/// Topmost folders whose whole content is older than the cutoff. The filter only decides which
/// folders are looked at; the size is that of the whole folder, because trashing takes all of it.
fn stale_folders(view: &View, root: u32, index: &StaleIndex, protected: &Protected, cutoff: i64) -> Vec<Match> {
    let tree = view.tree;
    let mut out = Vec::new();
    walk(view, root, protected, |_, id, zone| {
        let n = tree.node(id);
        if n.kind != Kind::Dir {
            return false;
        }
        if n.files > 0 && !protected.is_protected(id, zone) && index.older_than(id, cutoff) {
            out.push(Match { id, size: n.size(view.mode), activity: index.newest[id as usize] });
            return false;
        }
        true
    });
    out
}

/// Build and cache folders of projects in which nothing, the cache included, is newer than the cutoff.
fn stale_caches(view: &View, root: u32, index: &StaleIndex, protected: &Protected, cutoff: i64) -> Vec<Match> {
    let tree = view.tree;
    let mut out = Vec::new();
    walk(view, root, protected, |parent, id, zone| {
        let n = tree.node(id);
        if n.kind != Kind::Dir {
            return false;
        }
        let name = tree.name(id);
        let in_per_project_parent = tree.name(parent) == PER_PROJECT_CACHE_PARENT;
        let is_cache = in_per_project_parent
            || CACHE_DIRS.iter().any(|&(dir, marker)| {
                dir == name && marker.is_none_or(|m| tree.children(parent).any(|s| tree.name(s) == m))
            });
        if !is_cache {
            return true;
        }
        // A per-project cache is judged alone; the others together with their project.
        let judged = if in_per_project_parent { id } else { parent };
        if !protected.is_protected(id, zone) && index.older_than(judged, cutoff) && index.newest(id).is_some() {
            out.push(Match { id, size: n.size(view.mode), activity: index.newest[id as usize] });
        }
        false
    });
    out
}

/// Ascending by the key, with the id as a tie-breaker so that paging is stable.
fn sort(tree: &Tree, root: u32, matches: &mut [Match], key: StaleSort, now: i64) {
    let age_days = |m: &Match| ((now - m.activity).max(0) / SECS_PER_DAY) as u128;
    match key {
        StaleSort::Size => matches.sort_by_key(|m| (m.size, m.id)),
        // Older items have a smaller activity time, so "larger age" sorts later.
        StaleSort::Age => matches.sort_by_key(|m| (std::cmp::Reverse(m.activity), m.id)),
        StaleSort::Path => matches.sort_by_cached_key(|m| tree.rel_path(root, m.id).to_lowercase()),
        StaleSort::Score => matches.sort_by_key(|m| (u128::from(m.size) * age_days(m), m.id)),
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StaleRow {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub cat: u8,
    pub size: u64,
    pub files: u32,
    pub mtime: i64,
    pub atime: u32,
    pub created: u32,
    pub last_activity: i64,
}

pub fn row(tree: &Tree, root: u32, m: &Match) -> StaleRow {
    let n = tree.node(m.id);
    StaleRow {
        id: m.id,
        name: tree.name(m.id).to_string(),
        path: tree.rel_path(root, m.id),
        kind: n.kind.as_str(),
        cat: n.cat,
        size: m.size,
        files: n.files,
        mtime: n.mtime,
        atime: n.atime,
        created: n.created,
        last_activity: m.activity,
    }
}

#[derive(Serialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AgeBucket {
    pub size: u64,
    pub files: u64,
}

/// Space per age bucket (the buckets of the treemap's age colouring), by modification time.
#[derive(Serialize, Debug, Default)]
pub struct AgeHistogram {
    pub buckets: [AgeBucket; 6],
    /// Files whose modification time is unknown.
    pub unknown: AgeBucket,
}

pub fn age_histogram(view: &View, root: u32, now: i64) -> AgeHistogram {
    let mut h = AgeHistogram::default();
    view.for_each_leaf(root, |id| {
        let mtime = view.tree.node(id).mtime;
        let b = if mtime > 0 { &mut h.buckets[age_bucket(mtime, now) as usize] } else { &mut h.unknown };
        b.size += view.size(id);
        b.files += 1;
    });
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::LINUX;
    use crate::model::testutil::entry;
    use crate::model::{NewEntry, SizeMode, ROOT};
    use crate::query::{compute_filtered, Filter};
    use std::sync::Arc;

    const NOW: i64 = 2_000_000_000;
    const DAY: i64 = SECS_PER_DAY;

    /// An entry last modified `days` days before `NOW`.
    fn aged(name: &str, kind: Kind, size: u64, days: i64) -> NewEntry {
        NewEntry { mtime: NOW - days * DAY, ..entry(name, kind, size) }
    }

    fn args(list: ListKind, days: u32) -> StaleArgs {
        StaleArgs { list, older_than_days: days, use_access: false, show_apps: false, sort: StaleSort::Size, desc: true }
    }

    fn tree() -> Tree {
        Tree::new("/data".into(), aged("", Kind::Dir, 0, 1), 0)
    }

    fn names(t: &Tree, a: &StaleArgs, filter: Option<Filter>) -> Vec<String> {
        let sums = filter.map(|f| Arc::new(compute_filtered(t, &f, SizeMode::Alloc)));
        let v = View::new(t, SizeMode::Alloc, sums);
        let index = StaleIndex::compute(t, a.use_access, a.show_apps);
        let protected = Protected::resolve(t, None, &LINUX);
        find(&v, ROOT, a, &index, &protected, NOW).matches.iter().map(|m| t.name(m.id).to_string()).collect()
    }

    #[test]
    fn threshold_is_strict() {
        let mut t = tree();
        t.add_children(
            ROOT,
            vec![aged("older", Kind::File, 3, 366), aged("exactly", Kind::File, 2, 365), aged("newer", Kind::File, 1, 364)],
        );
        assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["older"]);
    }

    #[test]
    fn unknown_time_is_never_old() {
        let mut t = tree();
        let d = t.add_children(ROOT, vec![aged("dir", Kind::Dir, 0, 900), entry("no-mtime", Kind::File, 5)])[0];
        t.add_children(d, vec![aged("old", Kind::File, 5, 900), entry("unknown", Kind::File, 5)]);
        assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["old"]);
        assert!(names(&t, &args(ListKind::Folders, 365), None).is_empty(), "a folder with an unknown time inside is not judged");
    }

    #[test]
    fn recent_arrival_keeps_an_old_file() {
        let mut t = tree();
        let mut downloaded = aged("downloaded.zip", Kind::File, 5, 2000);
        downloaded.created = (NOW - 3 * DAY) as u32;
        t.add_children(ROOT, vec![downloaded, aged("really-old.zip", Kind::File, 5, 2000)]);
        assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["really-old.zip"]);
    }

    #[test]
    fn access_time_counts_only_when_asked() {
        let mut t = tree();
        let mut read_recently = aged("read-recently", Kind::File, 5, 900);
        read_recently.atime = (NOW - 2 * DAY) as u32;
        t.add_children(ROOT, vec![read_recently]);
        assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["read-recently"]);
        let with_access = StaleArgs { use_access: true, ..args(ListKind::Files, 365) };
        assert!(names(&t, &with_access, None).is_empty());
    }

    #[test]
    fn folders_report_only_the_topmost_stale_one() {
        let mut t = tree();
        let ids = t.add_children(ROOT, vec![aged("archive", Kind::Dir, 0, 800), aged("work", Kind::Dir, 0, 800)]);
        let inner = t.add_children(ids[0], vec![aged("2019", Kind::Dir, 0, 800), aged("a.txt", Kind::File, 10, 800)])[0];
        t.add_children(inner, vec![aged("b.txt", Kind::File, 20, 900)]);
        let sub = t.add_children(ids[1], vec![aged("done", Kind::Dir, 0, 800), aged("today.txt", Kind::File, 1, 0)])[0];
        t.add_children(sub, vec![aged("c.txt", Kind::File, 5, 700)]);
        // "archive" is stale as a whole; "work" has a fresh file, so only its stale subfolder counts.
        assert_eq!(names(&t, &args(ListKind::Folders, 365), None), ["archive", "done"]);
    }

    #[test]
    fn empty_folders_are_not_listed() {
        let mut t = tree();
        t.add_children(ROOT, vec![aged("empty", Kind::Dir, 0, 900)]);
        assert!(names(&t, &args(ListKind::Folders, 365), None).is_empty());
    }

    #[test]
    fn packages_are_single_items_and_apps_are_opt_in() {
        let mut t = tree();
        let mut app = aged("Old.app", Kind::Package, 0, 900);
        app.cat = category::APP;
        let ids = t.add_children(ROOT, vec![app, aged("Photos.photoslibrary", Kind::Package, 0, 900)]);
        t.add_children(ids[0], vec![aged("binary", Kind::File, 100, 900)]);
        t.add_children(ids[1], vec![aged("db", Kind::File, 50, 900)]);
        assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["Photos.photoslibrary"]);
        let with_apps = StaleArgs { show_apps: true, ..args(ListKind::Files, 365) };
        assert_eq!(names(&t, &with_apps, None), ["Old.app", "Photos.photoslibrary"]);
        assert!(names(&t, &args(ListKind::Folders, 365), None).is_empty(), "packages are not folders");
    }

    #[test]
    fn folder_holding_an_app_is_listed_only_with_apps_shown() {
        let mut t = tree();
        let ids = t.add_children(ROOT, vec![aged("Vendor", Kind::Dir, 0, 900), aged("Papers", Kind::Dir, 0, 900)]);
        let mut app = aged("Tool.app", Kind::Package, 0, 900);
        app.cat = category::APP;
        let app_id = t.add_children(ids[0], vec![app, aged("license.txt", Kind::File, 1, 900)])[0];
        t.add_children(app_id, vec![aged("binary", Kind::File, 100, 900)]);
        t.add_children(ids[1], vec![aged("a.pdf", Kind::File, 5, 900)]);
        assert_eq!(names(&t, &args(ListKind::Folders, 365), None), ["Papers"]);
        let with_apps = StaleArgs { show_apps: true, ..args(ListKind::Folders, 365) };
        assert_eq!(names(&t, &with_apps, None), ["Vendor", "Papers"]);
    }

    #[test]
    fn unsafe_content_disqualifies_the_folders_above_it() {
        for flag in [flags::DATALESS, flags::UNREADABLE, flags::OTHER_VOLUME] {
            let mut t = tree();
            let d = t.add_children(ROOT, vec![aged("dir", Kind::Dir, 0, 900)])[0];
            let mut odd = aged("odd", if flag == flags::DATALESS { Kind::File } else { Kind::Dir }, 5, 900);
            odd.flags = flag;
            t.add_children(d, vec![odd, aged("plain", Kind::File, 5, 900)]);
            assert!(names(&t, &args(ListKind::Folders, 365), None).is_empty(), "flag {flag}");
            assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["plain"], "flag {flag}");
        }
    }

    #[test]
    fn hard_link_duplicates_are_not_listed() {
        let mut t = tree();
        let mut dup = aged("second-link", Kind::File, 0, 900);
        dup.flags = flags::HARDLINK_DUP;
        t.add_children(ROOT, vec![dup, aged("first-link", Kind::File, 5, 900)]);
        assert_eq!(names(&t, &args(ListKind::Files, 365), None), ["first-link"]);
    }

    #[test]
    fn filter_narrows_files_but_folders_keep_their_full_size() {
        let mut t = tree();
        let d = t.add_children(ROOT, vec![aged("old", Kind::Dir, 0, 900), aged("notes.txt", Kind::File, 7, 900)])[0];
        t.add_children(d, vec![aged("a.mp4", Kind::File, 1000, 900), aged("b.txt", Kind::File, 10, 900)]);
        let mp4 = || Some(Filter { extensions: vec!["mp4".into()], ..Default::default() });
        assert_eq!(names(&t, &args(ListKind::Files, 365), mp4()), ["a.mp4"]);

        let sums = Arc::new(compute_filtered(&t, &mp4().unwrap(), SizeMode::Alloc));
        let v = View::new(&t, SizeMode::Alloc, Some(sums));
        let index = StaleIndex::compute(&t, false, false);
        let res = find(&v, ROOT, &args(ListKind::Folders, 365), &index, &Protected::default(), NOW);
        assert_eq!(res.matches.len(), 1);
        assert_eq!((res.matches[0].size, res.total_size), (1010, 1010), "the folder is trashed whole, so its whole size counts");
    }

    #[test]
    fn caches_need_an_inactive_project() {
        let mut t = tree();
        let ids = t.add_children(ROOT, vec![aged("dormant", Kind::Dir, 0, 900), aged("active", Kind::Dir, 0, 900), aged("plain", Kind::Dir, 0, 900)]);
        for (project, src_age) in [(ids[0], 900), (ids[1], 2)] {
            let kids = t.add_children(
                project,
                vec![aged("node_modules", Kind::Dir, 0, 900), aged("target", Kind::Dir, 0, 900), aged("Cargo.toml", Kind::File, 1, 900), aged("main.rs", Kind::File, 1, src_age)],
            );
            let nested = t.add_children(kids[0], vec![aged("dep", Kind::Dir, 0, 900), aged("x.js", Kind::File, 100, 900)])[0];
            t.add_children(nested, vec![aged("node_modules", Kind::Dir, 0, 900)]);
            t.add_children(kids[1], vec![aged("out.bin", Kind::File, 500, 900)]);
        }
        // A folder that is merely called "target", with no Cargo.toml next to it, is not a cache.
        let plain = t.add_children(ids[2], vec![aged("target", Kind::Dir, 0, 900)])[0];
        t.add_children(plain, vec![aged("data.bin", Kind::File, 9, 900)]);

        let a = args(ListKind::Caches, 365);
        let v = View::new(&t, SizeMode::Alloc, None);
        let index = StaleIndex::compute(&t, false, false);
        let res = find(&v, ROOT, &a, &index, &Protected::default(), NOW);
        let found: Vec<_> = res.matches.iter().map(|m| t.rel_path(ROOT, m.id)).collect();
        assert_eq!(found, ["dormant/target", "dormant/node_modules"], "largest first, nested caches folded into the top one");
    }

    #[test]
    fn derived_data_children_are_judged_one_by_one() {
        let mut t = tree();
        let dd = t.add_children(ROOT, vec![aged("DerivedData", Kind::Dir, 0, 1)])[0];
        let ids = t.add_children(dd, vec![aged("OldApp-abc", Kind::Dir, 0, 900), aged("NewApp-def", Kind::Dir, 0, 900)]);
        t.add_children(ids[0], vec![aged("build.o", Kind::File, 10, 900)]);
        t.add_children(ids[1], vec![aged("build.o", Kind::File, 10, 1)]);
        assert_eq!(names(&t, &args(ListKind::Caches, 365), None), ["OldApp-abc"]);
    }

    #[test]
    fn protected_paths_are_never_listed() {
        let mut t = Tree::new("/".into(), aged("", Kind::Dir, 0, 1), 0);
        let ids = t.add_children(ROOT, vec![aged("etc", Kind::Dir, 0, 900), aged("home", Kind::Dir, 0, 900), aged("data", Kind::Dir, 0, 900)]);
        t.add_children(ids[0], vec![aged("fstab", Kind::File, 1, 900)]);
        t.add_children(ids[1], vec![aged("old-user-file", Kind::File, 1, 900)]);
        t.add_children(ids[2], vec![aged("dump.sql", Kind::File, 1, 900)]);
        // /etc is protected with its content; /home only as a whole.
        assert_eq!(names(&t, &args(ListKind::Folders, 365), None), ["data"]);
        let mut files = names(&t, &args(ListKind::Files, 365), None);
        files.sort();
        assert_eq!(files, ["dump.sql", "old-user-file"]);
    }

    #[test]
    fn sort_orders() {
        let mut t = tree();
        t.add_children(
            ROOT,
            vec![aged("b-big-recent", Kind::File, 1000, 400), aged("a-small-ancient", Kind::File, 10, 4000), aged("c-mid", Kind::File, 500, 1000)],
        );
        let by = |sort, desc| names(&t, &StaleArgs { sort, desc, ..args(ListKind::Files, 365) }, None);
        assert_eq!(by(StaleSort::Size, true), ["b-big-recent", "c-mid", "a-small-ancient"]);
        assert_eq!(by(StaleSort::Age, true), ["a-small-ancient", "c-mid", "b-big-recent"]);
        assert_eq!(by(StaleSort::Path, false), ["a-small-ancient", "b-big-recent", "c-mid"]);
        // 500 x 1000 days beats 1000 x 400 days and 10 x 4000 days.
        assert_eq!(by(StaleSort::Score, true), ["c-mid", "b-big-recent", "a-small-ancient"]);
    }

    #[test]
    fn histogram_adds_up() {
        let mut t = tree();
        t.add_children(
            ROOT,
            vec![aged("today", Kind::File, 1, 0), aged("last-year", Kind::File, 10, 300), aged("ancient", Kind::File, 100, 3000), entry("unknown", Kind::File, 1000)],
        );
        let v = View::new(&t, SizeMode::Alloc, None);
        let h = age_histogram(&v, ROOT, NOW);
        assert_eq!(h.buckets[0], AgeBucket { size: 1, files: 1 });
        assert_eq!(h.buckets[3], AgeBucket { size: 10, files: 1 });
        assert_eq!(h.buckets[5], AgeBucket { size: 100, files: 1 });
        assert_eq!(h.unknown, AgeBucket { size: 1000, files: 1 });
        let total: u64 = h.buckets.iter().map(|b| b.size).sum::<u64>() + h.unknown.size;
        assert_eq!(total, t.node(ROOT).alloc);
    }
}
