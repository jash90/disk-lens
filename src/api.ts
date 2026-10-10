import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SizeMode = "alloc" | "logical";
export type SortKey = "name" | "size" | "files" | "dirs" | "mtime";
export type NodeKind = "file" | "dir" | "package" | "symlink" | "other";

export interface Filter {
  query: string;
  minSize: number;
  extensions: string[];
  excludes: string[];
}

export interface ViewArgs {
  mode: SizeMode;
  filter: Filter;
}

export const EMPTY_FILTER: Filter = { query: "", minSize: 0, extensions: [], excludes: [] };

export function filterActive(f: Filter): boolean {
  return f.query.trim() !== "" || f.minSize > 0 || f.extensions.length > 0 || f.excludes.length > 0;
}

/** Bity flag węzła – zgodne z `model::flags` w Rust. */
export const Flags = {
  HIDDEN: 1 << 0,
  DATALESS: 1 << 1,
  UNREADABLE: 1 << 2,
  HARDLINK_DUP: 1 << 3,
  OTHER_VOLUME: 1 << 4,
} as const;

export interface Volume {
  name: string;
  path: string;
  total: number;
  free: number;
}

export interface Progress {
  scanId: number;
  files: number;
  dirs: number;
  bytes: number;
  errors: number;
  current: string;
  elapsedMs: number;
  done: boolean;
  cancelled: boolean;
}

export interface TreeInfo {
  scanId: number;
  rootPath: string;
  scanning: boolean;
  alloc: number;
  logical: number;
  files: number;
  dirs: number;
  skipped: number;
  scannedAt: number;
  nodes: number;
}

export interface Row {
  id: number;
  name: string;
  kind: NodeKind;
  flags: number;
  cat: number;
  size: number;
  alloc: number;
  logical: number;
  files: number;
  dirs: number;
  mtime: number;
  pct: number;
  hasChildren: boolean;
}

export interface Page {
  total: number;
  parentSize: number;
  rows: Row[];
}

export interface NodeInfo {
  id: number;
  name: string;
  path: string;
  kind: NodeKind;
  flags: number;
  cat: number;
  size: number;
  alloc: number;
  logical: number;
  files: number;
  dirs: number;
  mtime: number;
  parent: number | null;
  pctOfRoot: number;
}

export interface Crumb {
  id: number;
  name: string;
}

export interface FileRow {
  id: number;
  name: string;
  path: string;
  size: number;
  mtime: number;
  cat: number;
  kind: NodeKind;
}

export interface Breakdown {
  total: number;
  categories: { cat: number; name: string; size: number; files: number }[];
  extensions: { ext: string; cat: number; size: number; files: number }[];
}

export interface Skipped {
  path: string;
  error: string;
}

export interface TrashCheck {
  path: string;
  alloc: number;
  logical: number;
  files: number;
  blocked: string | null;
}

export interface DiffEntry {
  path: string;
  before: number;
  after: number;
  delta: number;
  status: "grown" | "shrunk" | "added" | "removed";
  isDir: boolean;
  depth: number;
}

export interface DiffResult {
  beforeRoot: string;
  afterRoot: string;
  beforeScannedAt: number;
  afterScannedAt: number;
  totalBefore: number;
  totalAfter: number;
  entries: DiffEntry[];
}

/** Kafel treemapy zdekodowany z binarnej odpowiedzi (24 B/kafel). */
export interface Tile {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
  depth: number;
  kind: number; // 0 plik, 1 folder, 2 „inne”
  cat: number;
  age: number;
}

export const TILE_FILE = 0;
export const TILE_DIR = 1;
export const TILE_REST = 2;

const TILE_BYTES = 24;

function decodeTiles(buf: ArrayBuffer): Tile[] {
  const dv = new DataView(buf);
  const n = Math.floor(buf.byteLength / TILE_BYTES);
  const out: Tile[] = new Array(n);
  for (let i = 0; i < n; i++) {
    const o = i * TILE_BYTES;
    out[i] = {
      id: dv.getUint32(o, true),
      x: dv.getFloat32(o + 4, true),
      y: dv.getFloat32(o + 8, true),
      w: dv.getFloat32(o + 12, true),
      h: dv.getFloat32(o + 16, true),
      depth: dv.getUint8(o + 20),
      kind: dv.getUint8(o + 21),
      cat: dv.getUint8(o + 22),
      age: dv.getUint8(o + 23),
    };
  }
  return out;
}

export const api = {
  launchPath: () => invoke<string | null>("launch_path"),
  listVolumes: () => invoke<Volume[]>("list_volumes"),
  scanStart: (path: string, crossVolumes: boolean) => invoke<number>("scan_start", { path, crossVolumes }),
  scanCancel: () => invoke<void>("scan_cancel"),
  treeInfo: () => invoke<TreeInfo | null>("tree_info"),
  getNode: (id: number, view: ViewArgs) => invoke<NodeInfo>("get_node", { id, view }),
  getAncestors: (id: number) => invoke<Crumb[]>("get_ancestors", { id }),
  getChildren: (id: number, view: ViewArgs, sort: SortKey, desc: boolean, offset: number, limit: number) =>
    invoke<Page>("get_children", { id, view, sort, desc, offset, limit }),
  treemap: async (root: number, view: ViewArgs, width: number, height: number) =>
    decodeTiles(await invoke<ArrayBuffer>("treemap_layout", { root, view, width, height })),
  topFiles: (root: number, view: ViewArgs, limit: number) => invoke<FileRow[]>("top_files", { root, view, limit }),
  typeBreakdown: (root: number, view: ViewArgs) => invoke<Breakdown>("type_breakdown", { root, view }),
  skippedPaths: (limit: number) => invoke<{ total: number; items: Skipped[] }>("skipped_paths", { limit }),
  reveal: (id: number) => invoke<void>("reveal", { id }),
  openTerminal: (id: number) => invoke<void>("open_terminal", { id }),
  copyPath: (id: number) => invoke<string>("copy_path", { id }),
  trashCheck: (id: number) => invoke<TrashCheck>("trash_check", { id }),
  moveToTrash: (id: number) => invoke<number>("move_to_trash", { id }),
  rescanNode: (id: number) => invoke<void>("rescan_node", { id }),
  exportView: (root: number, view: ViewArgs, sort: SortKey, desc: boolean, format: "csv" | "json", path: string) =>
    invoke<number>("export_view", { root, view, sort, desc, format, path }),
  saveScan: (path: string) => invoke<void>("save_scan", { path }),
  loadScan: (path: string) => invoke<number>("load_scan", { path }),
  diffScans: (path: string, mode: SizeMode) => invoke<DiffResult>("diff_scans", { path, mode }),
  openPrivacySettings: () => invoke<void>("open_privacy_settings"),
  onProgress: (cb: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("scan://progress", (e) => cb(e.payload)),
};

/** Komunikat błędu z komendy Tauri (odrzucone Promise niesie string). */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
