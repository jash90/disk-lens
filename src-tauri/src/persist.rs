//! Saving/loading a scan, exporting a view to CSV/JSON and comparing two scans.

use crate::model::{SizeMode, Tree, ROOT};
use crate::query::{Row, View};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const MAGIC: &[u8; 8] = b"DLENS\0\0\x01";

#[derive(Serialize, Deserialize)]
struct SavedScan {
    tree: Tree,
}

pub fn save(tree: &Tree, path: &Path) -> Result<(), String> {
    let bytes = postcard::to_allocvec(&SavedScan { tree: tree.clone() }).map_err(|e| e.to_string())?;
    let compressed = zstd::encode_all(&bytes[..], 3).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(compressed.len() + MAGIC.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&compressed);
    fs::write(path, out).map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Result<Tree, String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
        return Err("To nie jest plik skanu disk-lens (.disklens) albo pochodzi z innej wersji.".into());
    }
    let raw = zstd::decode_all(&data[MAGIC.len()..]).map_err(|e| e.to_string())?;
    let saved: SavedScan = postcard::from_bytes(&raw).map_err(|e| format!("Uszkodzony plik skanu: {e}"))?;
    let mut tree = saved.tree;
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
    match format {
        "csv" => {
            let mut w = csv::Writer::from_path(path).map_err(|e| e.to_string())?;
            for r in &records {
                w.serialize(r).map_err(|e| e.to_string())?;
            }
            w.flush().map_err(|e| e.to_string())?;
        }
        "json" => {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Doc<'a> {
                root: String,
                size_mode: SizeMode,
                total: u64,
                items: &'a [ExportRow<'a>],
            }
            let doc = Doc {
                root: tree.path(root).to_string_lossy().into_owned(),
                size_mode: view.mode,
                total: view.size(root),
                items: &records,
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
    fn dates() {
        assert_eq!(iso_date(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_date(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}
