//! Saving/loading a scan, exporting a view to CSV/JSON and comparing two scans.

use crate::model::{Kind, Node, ScanState, SizeMode, Skipped, Tree, ROOT};
use crate::query::{Row, View};
use crate::stale::StaleRow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// File header: 7 fixed bytes, then the format version.
const MAGIC_PREFIX: &[u8; 7] = b"DLENS\0\0";
/// Version 2 added `Node::atime` and `Node::created`. Version 3 appends the `ScanState` after
/// the tree, whose own layout did not change.
const VERSION: u8 = 3;
/// Removed nodes are left out of the file once they are more than this share of the tree.
const COMPACT_ABOVE: f64 = 0.02;

/// The version 1 layout (Disk Lens 0.2–0.3), frozen so that old files keep loading.
mod v1 {
    use super::*;

    #[derive(Deserialize)]
    pub struct NodeV1 {
        parent: u32,
        first_child: u32,
        next_sibling: u32,
        name_off: u32,
        name_len: u16,
        kind: Kind,
        flags: u8,
        cat: u8,
        alloc: u64,
        logical: u64,
        files: u32,
        dirs: u32,
        mtime: i64,
    }

    #[derive(Deserialize)]
    pub struct TreeV1 {
        root_path: PathBuf,
        nodes: Vec<NodeV1>,
        names: String,
        skipped: Vec<Skipped>,
        scanned_at: i64,
    }

    impl From<TreeV1> for Tree {
        /// Access and creation times were not recorded: they become 0 ("unknown").
        fn from(t: TreeV1) -> Tree {
            let nodes = t
                .nodes
                .into_iter()
                .map(|n| Node {
                    parent: n.parent,
                    first_child: n.first_child,
                    next_sibling: n.next_sibling,
                    name_off: n.name_off,
                    name_len: n.name_len,
                    kind: n.kind,
                    flags: n.flags,
                    cat: n.cat,
                    alloc: n.alloc,
                    logical: n.logical,
                    files: n.files,
                    dirs: n.dirs,
                    mtime: n.mtime,
                    atime: 0,
                    created: 0,
                })
                .collect();
            Tree { root_path: t.root_path, nodes, names: t.names, skipped: t.skipped, scanned_at: t.scanned_at, generation: 0, state: Default::default() }
        }
    }
}

pub fn save(tree: &Tree, path: &Path) -> Result<(), String> {
    let dead = tree.len() - tree.live_len();
    let compact;
    let tree = if dead as f64 > tree.len() as f64 * COMPACT_ABOVE {
        compact = tree.compacted();
        &compact
    } else {
        tree
    };
    // Straight into the compressor: a serialized tree of a whole disk is over a gigabyte.
    let io = |e: std::io::Error| e.to_string();
    let mut file = io::BufWriter::new(fs::File::create(path).map_err(io)?);
    file.write_all(MAGIC_PREFIX).map_err(io)?;
    file.write_all(&[VERSION]).map_err(io)?;
    let mut encoder = zstd::Encoder::new(file, 3).map_err(io)?;
    postcard::to_io(tree, &mut encoder).map_err(|e| e.to_string())?;
    postcard::to_io(&tree.state, &mut encoder).map_err(|e| e.to_string())?;
    encoder.finish().map_err(io)?.flush().map_err(io)
}

pub fn load(path: &Path) -> Result<Tree, String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let header = MAGIC_PREFIX.len() + 1;
    if data.len() < header || &data[..MAGIC_PREFIX.len()] != MAGIC_PREFIX {
        return Err("To nie jest plik skanu Disk Lens (.disklens).".into());
    }
    let version = data[MAGIC_PREFIX.len()];
    if !(1..=VERSION).contains(&version) {
        return Err("Ten plik skanu pochodzi z nowszej wersji Disk Lens. Zaktualizuj aplikację.".into());
    }
    let raw = zstd::decode_all(&data[header..]).map_err(|e| e.to_string())?;
    drop(data);
    let corrupt = |e: postcard::Error| format!("Uszkodzony plik skanu: {e}");
    let mut tree: Tree = match version {
        1 => postcard::from_bytes::<v1::TreeV1>(&raw).map_err(corrupt)?.into(),
        2 => postcard::from_bytes(&raw).map_err(corrupt)?,
        _ => {
            let (mut tree, rest) = postcard::take_from_bytes::<Tree>(&raw).map_err(corrupt)?;
            tree.state = postcard::from_bytes::<ScanState>(rest).map_err(corrupt)?;
            tree
        }
    };
    tree.generation = 1;
    Ok(tree)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportRow<'a> {
    name: &'a str,
    path: String,
    kind: &'a str,
    size: u64,
    allocated: u64,
    logical: u64,
    files: u32,
    dirs: u32,
    modified: String,
    percent: String,
}

fn iso_date(ts: i64) -> String {
    // UTC date without external dependencies (H. Hinnant's "civil from days" algorithm).
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

pub fn export(view: &View, root: u32, rows: &[Row], format: &str, path: &Path) -> Result<usize, String> {
    let tree = view.tree;
    let records: Vec<ExportRow> = rows
        .iter()
        .map(|r| ExportRow {
            name: &r.name,
            path: tree.path(r.id).to_string_lossy().into_owned(),
            kind: r.kind,
            size: r.size,
            allocated: r.alloc,
            logical: r.logical,
            files: r.files,
            dirs: r.dirs,
            modified: iso_date(r.mtime),
            percent: format!("{:.2}", r.pct),
        })
        .collect();
    write_records(view, root, &records, format, path)
}

#[derive(Serialize)]
struct StaleExportRow<'a> {
    name: &'a str,
    path: String,
    kind: &'a str,
    size: u64,
    files: u32,
    modified: String,
    accessed: String,
    created: String,
    last_activity: String,
}

/// Export of a stale list (see [`crate::stale`]). Unknown times are left empty.
pub fn export_stale(view: &View, root: u32, rows: &[StaleRow], format: &str, path: &Path) -> Result<usize, String> {
    let date = |ts: i64| if ts > 0 { iso_date(ts) } else { String::new() };
    let records: Vec<StaleExportRow> = rows
        .iter()
        .map(|r| StaleExportRow {
            name: &r.name,
            path: view.tree.path(r.id).to_string_lossy().into_owned(),
            kind: r.kind,
            size: r.size,
            files: r.files,
            modified: date(r.mtime),
            accessed: date(i64::from(r.atime)),
            created: date(i64::from(r.created)),
            last_activity: date(r.last_activity),
        })
        .collect();
    write_records(view, root, &records, format, path)
}

fn write_records<T: Serialize>(view: &View, root: u32, records: &[T], format: &str, path: &Path) -> Result<usize, String> {
    let tree = view.tree;
    match format {
        "csv" => {
            let mut w = csv::Writer::from_path(path).map_err(|e| e.to_string())?;
            for r in records {
                w.serialize(r).map_err(|e| e.to_string())?;
            }
            w.flush().map_err(|e| e.to_string())?;
        }
        "json" => {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Doc<'a, T> {
                root: String,
                size_mode: SizeMode,
                total: u64,
                items: &'a [T],
            }
            let doc = Doc {
                root: tree.path(root).to_string_lossy().into_owned(),
                size_mode: view.mode,
                total: view.size(root),
                items: records,
            };
            let s = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
            fs::write(path, s).map_err(|e| e.to_string())?;
        }
        other => return Err(format!("Nieznany format eksportu: {other}")),
    }
    Ok(records.len())
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub path: String,
    pub before: u64,
    pub after: u64,
    pub delta: i64,
    /// grown | shrunk | added | removed
    pub status: &'static str,
    pub is_dir: bool,
    pub depth: u32,
}

impl DiffEntry {
    fn new(path: String, before: u64, after: u64, status: &'static str, is_dir: bool, depth: u32) -> Self {
        DiffEntry { path, before, after, delta: after as i64 - before as i64, status, is_dir, depth }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub before_root: String,
    pub after_root: String,
    pub before_scanned_at: i64,
    pub total_before: u64,
    pub total_after: u64,
    pub entries: Vec<DiffEntry>,
}

/// Compares two scans, matching nodes by name at every level (relative paths).
pub fn diff(before: &Tree, after: &Tree, mode: SizeMode, limit: usize) -> DiffResult {
    let mut entries = Vec::new();
    diff_dir(before, after, ROOT, ROOT, String::new(), 0, mode, &mut entries);
    entries.sort_by(|a, b| b.delta.unsigned_abs().cmp(&a.delta.unsigned_abs()).then(a.depth.cmp(&b.depth)));
    entries.truncate(limit);
    DiffResult {
        before_root: before.root_path.to_string_lossy().into_owned(),
        after_root: after.root_path.to_string_lossy().into_owned(),
        before_scanned_at: before.scanned_at,
        total_before: before.node(ROOT).size(mode),
        total_after: after.node(ROOT).size(mode),
        entries,
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_dir(
    b: &Tree,
    a: &Tree,
    bid: u32,
    aid: u32,
    prefix: String,
    depth: u32,
    mode: SizeMode,
    out: &mut Vec<DiffEntry>,
) {
    let mut before: HashMap<&str, u32> = b.children(bid).map(|c| (b.name(c), c)).collect();
    let join = |name: &str| if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
    for ac in a.children(aid) {
        let name = a.name(ac);
        let an = a.node(ac);
        let after_size = an.size(mode);
        match before.remove(name) {
            Some(bc) => {
                let bn = b.node(bc);
                let before_size = bn.size(mode);
                if before_size != after_size {
                    let status = if after_size > before_size { "grown" } else { "shrunk" };
                    out.push(DiffEntry::new(join(name), before_size, after_size, status, an.kind.is_dir(), depth));
                    if an.kind.is_dir() && bn.kind.is_dir() {
                        diff_dir(b, a, bc, ac, join(name), depth + 1, mode, out);
                    }
                }
            }
            None if after_size > 0 => out.push(DiffEntry::new(join(name), 0, after_size, "added", an.kind.is_dir(), depth)),
            None => {}
        }
    }
    for (name, bc) in before {
        let bn = b.node(bc);
        let s = bn.size(mode);
        if s > 0 {
            out.push(DiffEntry::new(join(name), s, 0, "removed", bn.kind.is_dir(), depth));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testutil::{entry as e, sample_tree};
    use crate::model::Kind;
    use crate::query::{children, SortKey};

    fn tree(sizes: &[(&str, u64)]) -> Tree {
        let mut t = Tree::new("/r".into(), e("", Kind::Dir, 0), 0);
        let d = t.add_children(ROOT, vec![e("d", Kind::Dir, 0)])[0];
        t.add_children(d, sizes.iter().map(|(n, s)| e(n, Kind::File, *s)).collect());
        t
    }

    #[test]
    fn export_writes_csv_and_json() {
        let dir = tempfile::tempdir().unwrap();
        let t = sample_tree();
        let v = View::new(&t, SizeMode::Alloc, None);
        let rows = children(&v, ROOT, SortKey::Size, true, 0, usize::MAX).rows;

        let csv_path = dir.path().join("out.csv");
        assert_eq!(export(&v, ROOT, &rows, "csv", &csv_path).unwrap(), 3);
        let csv = fs::read_to_string(&csv_path).unwrap();
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), "name,path,kind,size,allocated,logical,files,dirs,modified,percent");
        assert_eq!(lines.next().unwrap(), "node_modules,/r/node_modules,dir,5000,5000,5000,1,0,1970-01-01T00:05:00Z,79.24");

        let json_path = dir.path().join("out.json");
        export(&v, ROOT, &rows, "json", &json_path).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(doc["root"], "/r");
        assert_eq!(doc["sizeMode"], "alloc");
        assert_eq!(doc["total"], 6310);
        assert_eq!(doc["items"][1]["name"], "a.mp4");

        assert!(export(&v, ROOT, &rows, "xml", &json_path).is_err());
    }

    #[test]
    fn stale_export_leaves_unknown_times_empty() {
        let dir = tempfile::tempdir().unwrap();
        let t = sample_tree();
        let v = View::new(&t, SizeMode::Alloc, None);
        let id = t.children(ROOT).find(|&c| t.name(c) == "a.mp4").unwrap();
        let m = crate::stale::Match { id, size: 1000, activity: t.node(id).mtime };
        let rows = [crate::stale::row(&t, ROOT, &m)];
        let path = dir.path().join("stale.csv");
        assert_eq!(export_stale(&v, ROOT, &rows, "csv", &path).unwrap(), 1);
        let csv = fs::read_to_string(&path).unwrap();
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), "name,path,kind,size,files,modified,accessed,created,last_activity");
        let line = lines.next().unwrap();
        assert!(line.starts_with("a.mp4,/r/a.mp4,file,1000,0,1970-") && line.contains("Z,,,1970-"), "{line}");
    }

    #[test]
    fn roundtrip_and_diff() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.disklens");
        let before = tree(&[("a", 10), ("b", 20)]);
        save(&before, &p).unwrap();
        let loaded = load(&p).unwrap();
        assert_eq!(loaded.len(), before.len());
        assert_eq!(loaded.node(ROOT).alloc, 30);

        let after = tree(&[("a", 50), ("c", 5)]);
        let d = diff(&loaded, &after, SizeMode::Alloc, 100);
        let find = |p: &str| d.entries.iter().find(|e| e.path == p).unwrap();
        assert_eq!(find("d").delta, 25);
        assert_eq!(find("d/a").status, "grown");
        assert_eq!(find("d/b").status, "removed");
        assert_eq!(find("d/c").status, "added");
    }

    #[test]
    fn loads_a_file_saved_by_format_version_1() {
        let t = load(Path::new("tests/fixtures/v1.disklens")).unwrap();
        let expected = sample_tree();
        assert_eq!(t.len(), expected.len());
        assert_eq!(t.root_path, PathBuf::from("/r"));
        assert_eq!(t.scanned_at, 1_700_000_000);
        assert_eq!(t.skipped.len(), 1);
        assert_eq!(t.node(ROOT).alloc, 6310);
        for id in 0..t.len() as u32 {
            let (a, b) = (t.node(id), expected.node(id));
            assert_eq!((t.name(id), a.kind, a.alloc, a.files, a.mtime), (expected.name(id), b.kind, b.alloc, b.files, b.mtime));
            assert_eq!((a.atime, a.created), (0, 0), "times the old format did not record are unknown");
        }
    }

    #[test]
    fn loads_a_file_saved_by_format_version_2() {
        let t = load(Path::new("tests/fixtures/v2.disklens")).unwrap();
        let expected = sample_tree();
        assert_eq!((t.len(), t.node(ROOT).alloc, t.scanned_at), (expected.len(), 6310, expected.scanned_at));
        for id in 0..t.len() as u32 {
            let (a, b) = (t.node(id), expected.node(id));
            assert_eq!((t.name(id), a.kind, a.alloc, a.files, a.mtime), (expected.name(id), b.kind, b.alloc, b.files, b.mtime));
        }
        assert_eq!((t.node(1).atime, t.node(1).created), (1_600_000_000, 1_500_000_000));
        assert_eq!(t.state, ScanState::default(), "older files know nothing about the event log");
    }

    #[test]
    fn roundtrip_keeps_the_scan_state() {
        use crate::model::Link;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.disklens");
        let mut t = sample_tree();
        t.state = ScanState { links: vec![Link { dev: 7, ino: 9, id: 2 }], refreshed_at: 5, event_id: 123_456_789_012, event_uuid: "ABC".into() };
        save(&t, &p).unwrap();
        let loaded = load(&p).unwrap();
        assert_eq!(loaded.state, t.state);
        assert_eq!(loaded.len(), t.len());
    }

    #[test]
    fn saving_leaves_out_removed_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.disklens");
        let mut t = sample_tree();
        let modules = t.find_child(ROOT, "node_modules").unwrap();
        t.detach(modules);
        save(&t, &p).unwrap();
        let loaded = load(&p).unwrap();
        assert_eq!((loaded.len(), loaded.node(ROOT).alloc), (t.live_len(), t.node(ROOT).alloc));
        assert!(loaded.find_child(ROOT, "node_modules").is_none());
    }

    #[test]
    fn roundtrip_keeps_access_and_creation_times() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.disklens");
        let mut t = sample_tree();
        t.nodes[1].atime = 1_600_000_000;
        t.nodes[1].created = 1_500_000_000;
        save(&t, &p).unwrap();
        let loaded = load(&p).unwrap();
        assert_eq!((loaded.node(1).atime, loaded.node(1).created), (1_600_000_000, 1_500_000_000));
    }

    #[test]
    fn rejects_foreign_and_newer_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.disklens");
        fs::write(&p, b"not a scan").unwrap();
        assert!(load(&p).is_err());
        fs::write(&p, b"DLENS\0\0\x63rest").unwrap();
        assert!(load(&p).unwrap_err().contains("nowszej"));
    }

    #[test]
    fn dates() {
        assert_eq!(iso_date(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_date(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}
