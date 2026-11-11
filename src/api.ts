import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SizeMode = "alloc" | "logical";
export type SortKey = "name" | "size" | "files" | "dirs" | "mtime";
export type NodeKind = "file" | "dir" | "package" | "symlink" | "other";

/** Folders and packages: nodes that have totals and can be entered. */
export function isDirKind(kind: NodeKind | undefined): boolean {
  return kind === "dir" || kind === "package";
}

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

/** Node flag bits, matching `model::flags` in Rust. */
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

/** The volume the scan lives on. */
export interface VolumeInfo {
  name: string;
  mountPath: string;
  fsType: string;
  total: number;
  free: number;
  /** macOS: free space including what the system reclaims on its own (as Finder counts it). */
  availableImportant: number | null;
}

export interface Coverage {
  used: number;
  scanned: number;
  unaccounted: number;
  overcounted: number;
  purgeable: number;
}

/** Release found by the updater (`latest.json` from GitHub Releases). */
export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
}

interface UpdateProgress {
  done: number;
  /** 0 when the server did not report the length. */
  total: number;
  finished: boolean;
  error: string | null;
}

export interface Progress {
  scanId: number;
  files: number;
  dirs: number;
  bytes: number;
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
  volume: VolumeInfo | null;
  coverage: Coverage | null;
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
  size: number;
  alloc: number;
  logical: number;
  files: number;
  dirs: number;
  mtime: number;
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
  categories: { cat: number; name: string; size: number; files: number }[];
  extensions: { ext: string; cat: number; size: number; files: number }[];
}

export interface Skipped {
  path: string;
  error: string;
}

/** An item that was left alone, with the reason. */
export interface TrashProblem {
  path: string;
  reason: string;
}

export interface TrashCheck {
  /** Items that would be moved, after dropping those inside another selected item. */
  count: number;
  alloc: number;
  files: number;
  /** Paths of the first few items. */
  sample: string[];
  blocked: TrashProblem[];
}

export interface TrashOutcome {
  freed: number;
  trashed: number;
  /** Left alone on purpose: protected, or changed since the scan. */
  skipped: TrashProblem[];
  /** The system refused to move them. */
  failed: TrashProblem[];
}

export type StaleListKind = "files" | "folders" | "caches";
export type StaleSort = "size" | "age" | "path" | "score";

export interface StaleArgs {
  list: StaleListKind;
  olderThanDays: number;
  useAccess: boolean;
  showApps: boolean;
  sort: StaleSort;
  desc: boolean;
}

export interface StaleRow {
  id: number;
  name: string;
  path: string;
  kind: NodeKind;
  cat: number;
  size: number;
  files: number;
  mtime: number;
  /** 0 when unknown. */
  atime: number;
  created: number;
  /** Newest of the times that count as activity (for a folder: anywhere inside it). */
  lastActivity: number;
}

export interface StalePage {
  total: number;
  totalSize: number;
  rows: StaleRow[];
}

interface AgeBucket {
  size: number;
  files: number;
}

export interface AgeHistogram {
  /** Same buckets as the treemap's age colouring, newest first. */
  buckets: AgeBucket[];
  unknown: AgeBucket;
}

interface DiffEntry {
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
  totalBefore: number;
  totalAfter: number;
  entries: DiffEntry[];
}

/** Treemap tile decoded from the binary response (24 B per tile). */
export interface Tile {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
  depth: number;
  kind: number; // 0 file, 1 folder, 2 "other"
  cat: number;
  age: number;
}

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
  trashCheck: (ids: number[]) => invoke<TrashCheck>("trash_check_batch", { ids }),
  moveToTrash: (ids: number[], verify: boolean) => invoke<TrashOutcome>("move_to_trash_batch", { ids, verify }),
  staleList: (root: number, view: ViewArgs, args: StaleArgs, offset: number, limit: number) =>
    invoke<StalePage>("stale_list", { root, view, args, offset, limit }),
  ageHistogram: (root: number, view: ViewArgs) => invoke<AgeHistogram>("age_histogram", { root, view }),
  rescanNode: (id: number) => invoke<void>("rescan_node", { id }),
  exportView: (
    root: number,
    view: ViewArgs,
    sort: SortKey,
    desc: boolean,
    format: "csv" | "json",
    path: string,
    stale: StaleArgs | null = null,
  ) => invoke<number>("export_view", { root, view, sort, desc, format, path, stale }),
  saveScan: (path: string) => invoke<void>("save_scan", { path }),
  loadScan: (path: string) => invoke<number>("load_scan", { path }),
  diffScans: (path: string, mode: SizeMode) => invoke<DiffResult>("diff_scans", { path, mode }),
  openPrivacySettings: () => invoke<void>("open_privacy_settings"),
  onProgress: (cb: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("scan://progress", (e) => cb(e.payload)),
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  knownUpdate: () => invoke<UpdateInfo | null>("known_update"),
  /** After a successful install the app restarts, so the Promise settles only on error. */
  installUpdate: () => invoke<void>("install_update"),
  onUpdateStatus: (cb: (u: UpdateInfo | null) => void): Promise<UnlistenFn> =>
    listen<UpdateInfo | null>("update://status", (e) => cb(e.payload)),
  onUpdateProgress: (cb: (p: UpdateProgress) => void): Promise<UnlistenFn> =>
    listen<UpdateProgress>("update://progress", (e) => cb(e.payload)),
};

/** Error message from a Tauri command (the rejected Promise carries a string). */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
