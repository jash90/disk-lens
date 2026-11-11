//! Bringing an existing tree up to date without scanning everything again: given the folders
//! that changed since the tree was built, each is listed once and compared with its children in
//! the tree. Only new subfolders (and folders marked as changed in depth) are scanned in full.
//!
//! The result must equal a fresh scan, with two documented exceptions: which of several hard
//! links carries the size may differ (as it does between two full scans), and access times of
//! files in folders that were not listed keep their old values.

use crate::model::{flags, Kind, Link, NewEntry, Tree, ROOT};
use crate::platform;
use crate::scanner::{entry_from_meta, scan_dir, Classified, Ctx, ScanControl, ScanOptions};
use crate::{listing, model::Skipped};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// A folder whose content changed. `recursive`: anything below it may have changed too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub recursive: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshStats {
    /// Folders listed once and compared with the tree.
    pub listed: usize,
    /// Folders scanned again in depth.
    pub rescanned: usize,
}

/// Turns changed paths into tree nodes to refresh. A path that is not in the tree yet stands
/// for its nearest folder that is: listing that one finds the new subtree.
fn plan(tree: &Tree, changes: &[Change]) -> Vec<(u32, bool)> {
    let mut targets: HashMap<u32, bool> = HashMap::new();
    for change in changes {
        let Some((mut id, exact)) = tree.resolve(&change.path) else { continue };
        // A file where a folder is expected: its folder has to be listed.
        while id != ROOT && !tree.node(id).kind.is_dir() {
            id = tree.node(id).parent;
        }
        // Folders that are deliberately not entered stay that way.
        if tree.node(id).flags & flags::OTHER_VOLUME != 0 {
            continue;
        }
        *targets.entry(id).or_default() |= change.recursive && exact;
    }
    let deep: HashSet<u32> = targets.iter().filter(|&(_, &recursive)| recursive).map(|(&id, _)| id).collect();
    let mut out: Vec<(u32, bool)> = targets
        .into_iter()
        .filter(|&(id, _)| !tree.ancestors(id).iter().any(|a| *a != id && deep.contains(a)))
        .collect();
    out.sort_unstable();
    out
}

/// Refreshes the folders named by `changes`. Paths outside the tree's root are ignored.
pub fn refresh(tree: &RwLock<Tree>, changes: &[Change], opts: &ScanOptions, ctl: &ScanControl) -> RefreshStats {
    let _activity = platform::ScanActivity::begin();
    let (root_path, targets) = {
        let t = tree.read();
        (t.root_path.clone(), plan(&t, changes))
    };
    let ctx = Ctx::new(tree, opts, ctl, &root_path);
    let (listed, rescanned) = (AtomicUsize::new(0), AtomicUsize::new(0));
    {
        let (ctx, listed, rescanned) = (&ctx, &listed, &rescanned);
        ctx.pool().scope(move |s| {
            for (id, recursive) in targets {
                s.spawn(move |s| {
                    if ctx.ctl.cancelled() {
                        return;
                    }
                    let counter = if recursive { rescanned } else { listed };
                    counter.fetch_add(1, Relaxed);
                    refresh_dir(s, ctx, id, recursive);
                });
            }
        });
    }
    let mut t = tree.write();
    fix_hard_links(&mut t);
    t.state.refreshed_at = platform::now_secs();
    RefreshStats { listed: listed.into_inner(), rescanned: rescanned.into_inner() }
}

/// Drops the "skipped" records of `path` and of everything below it.
fn forget_skipped(t: &mut Tree, path: &Path) {
    t.skipped.retain(|s| !Path::new(&s.path).starts_with(path));
}

fn refresh_dir<'s>(s: &rayon::Scope<'s>, ctx: &'s Ctx<'s>, id: u32, recursive: bool) {
    let path = {
        let t = ctx.tree.read();
        if !t.is_attached(id) {
            return;
        }
        t.path(id)
    };
    let meta = match if id == ROOT { fs::metadata(&path) } else { fs::symlink_metadata(&path) } {
        Ok(m) if m.is_dir() => m,
        // The folder is gone, or something else took its place: its parent's listing settles it.
        Ok(_) => return remove(ctx, id, &path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return remove(ctx, id, &path),
        Err(e) => return unreadable(ctx, id, &path, &e),
    };
    let dev = platform::device(&meta);
    let own = entry_from_meta(String::new(), &meta);
    if id != ROOT && !ctx.may_enter(&path, dev) {
        // Another volume was mounted here since the scan.
        let mut t = ctx.tree.write();
        if t.is_attached(id) {
            t.clear_children(id);
            forget_skipped(&mut t, &path);
            t.update_own(id, &own, true);
            t.nodes[id as usize].flags |= flags::OTHER_VOLUME;
        }
        return;
    }

    if recursive {
        {
            let mut t = ctx.tree.write();
            if !t.is_attached(id) {
                return;
            }
            t.clear_children(id);
            t.update_own(id, &own, true);
            forget_skipped(&mut t, &path);
        }
        return scan_dir(s, ctx, id, path, dev);
    }

    let mut listed: Vec<Classified> = Vec::new();
    let mut skipped = Vec::new();
    let result = listing::read_dir(&path, ctx.backend(dev, &path), |item| {
        match item {
            Ok(raw) => listed.push(ctx.classify(&path, &raw, false)),
            Err(err) => {
                let p = err.name.map_or_else(|| path.clone(), |name| path.join(name));
                skipped.push(Skipped { path: p.to_string_lossy().into_owned(), error: err.error.to_string() });
            }
        }
        !ctx.ctl.cancelled()
    });
    if let Err(e) = result {
        return unreadable(ctx, id, &path, &e);
    }
    if ctx.ctl.cancelled() {
        return;
    }

    let mut to_scan = Vec::new();
    {
        let mut t = ctx.tree.write();
        if !t.is_attached(id) {
            return;
        }
        t.update_own(id, &own, true);
        t.nodes[id as usize].flags &= !flags::UNREADABLE;
        let mut existing: HashMap<String, u32> = t.children(id).map(|c| (t.name(c).to_string(), c)).collect();
        let mut fresh: Vec<Classified> = Vec::new();
        for item in listed {
            match existing.remove(&item.entry.name) {
                Some(child) if t.node(child).kind == item.entry.kind => {
                    if update_child(&mut t, child, &item) {
                        to_scan.extend(item.descend.map(|(p, d)| (child, p, d)));
                    }
                }
                Some(child) => {
                    forget_skipped(&mut t, &path.join(&item.entry.name));
                    t.detach(child);
                    fresh.push(item);
                }
                None => fresh.push(item),
            }
        }
        for (name, child) in existing {
            forget_skipped(&mut t, &path.join(name));
            t.detach(child);
        }
        let entries: Vec<NewEntry> = fresh.iter().map(|c| c.entry.clone()).collect();
        let ids = t.add_children(id, entries);
        for (item, &child) in fresh.iter().zip(&ids) {
            if let Some((dev, ino)) = item.link {
                t.state.links.push(Link { dev, ino, id: child });
            }
        }
        // Entries of this folder that could not be read last time are judged again by this listing.
        let still_closed: HashSet<PathBuf> =
            t.children(id).filter(|&c| t.node(c).flags & flags::UNREADABLE != 0).map(|c| path.join(t.name(c))).collect();
        // This folder itself was listed, so it is no longer among the skipped ones either.
        t.skipped.retain(|s| {
            let p = Path::new(&s.path);
            p != path && (p.parent() != Some(path.as_path()) || still_closed.contains(p))
        });
        t.skipped.extend(skipped);
        for (item, child) in fresh.into_iter().zip(ids) {
            if let Some((child_path, child_dev)) = item.descend {
                to_scan.push((child, child_path, child_dev));
            }
        }
    }
    for (child, child_path, child_dev) in to_scan {
        s.spawn(move |s| scan_dir(s, ctx, child, child_path, child_dev));
    }
}

/// An entry that is in the tree already: only its own data can have changed. A subfolder keeps
/// its children; if its content changed, it is on the list of changes itself. Returns true when
/// the folder was closed (another volume) and is open now, so it has to be scanned.
fn update_child(t: &mut Tree, child: u32, item: &Classified) -> bool {
    let was_closed = t.node(child).flags & flags::OTHER_VOLUME != 0;
    let is_closed = item.entry.flags & flags::OTHER_VOLUME != 0;
    if is_closed && !was_closed {
        t.clear_children(child);
        t.nodes[child as usize].flags |= flags::OTHER_VOLUME;
    } else if was_closed && !is_closed {
        t.nodes[child as usize].flags &= !flags::OTHER_VOLUME;
    }
    t.update_own(child, &item.entry, false);
    if let Some((dev, ino)) = item.link {
        // The file may have gained its second link only now; duplicates are dropped later.
        t.state.links.push(Link { dev, ino, id: child });
    }
    was_closed && !is_closed
}

fn remove(ctx: &Ctx<'_>, id: u32, path: &Path) {
    if id == ROOT {
        return;
    }
    let mut t = ctx.tree.write();
    forget_skipped(&mut t, path);
    t.detach(id);
}

fn unreadable(ctx: &Ctx<'_>, id: u32, path: &Path, error: &io::Error) {
    {
        let mut t = ctx.tree.write();
        if !t.is_attached(id) {
            return;
        }
        t.clear_children(id);
        forget_skipped(&mut t, path);
    }
    ctx.record_unreadable(id, path, error);
}

/// After nodes came and went, exactly one live link of every multi-link file must carry its
/// size. Links found by this refresh were inserted with their size; links whose size-carrying
/// sibling was removed are at 0.
fn fix_hard_links(t: &mut Tree) {
    let mut links = std::mem::take(&mut t.state.links);
    links.retain(|l| t.is_attached(l.id));
    links.sort_unstable();
    links.dedup();
    for group in links.chunk_by(|a, b| (a.dev, a.ino) == (b.dev, b.ino)) {
        let primaries: Vec<u32> = group.iter().map(|l| l.id).filter(|&id| t.node(id).flags & flags::HARDLINK_DUP == 0).collect();
        match primaries.as_slice() {
            [_] => {}
            [_, extra @ ..] => {
                for &id in extra {
                    t.set_link_primary(id, None);
                }
            }
            [] => {
                let id = group[0].id;
                if let Ok(meta) = fs::symlink_metadata(t.path(id)) {
                    t.set_link_primary(id, Some((platform::alloc_size(&meta), meta.len())));
                }
            }
        }
    }
    t.state.links = links;
    debug_assert!(t.node(ROOT).kind == Kind::Dir);
}
