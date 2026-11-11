import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type StaleArgs, type TrashOutcome } from "../api";
import { useStore } from "../store";
import { formatCount } from "./format";

const st = () => useStore.getState();

async function guarded<T>(label: string | null, fn: () => Promise<T>): Promise<T | undefined> {
  if (label) st().setBusy(label);
  try {
    return await fn();
  } catch (e) {
    st().toast(errorMessage(e), "error");
    return undefined;
  } finally {
    if (label) st().setBusy(null);
  }
}

export async function refreshInfo() {
  const info = await api.treeInfo();
  st().setInfo(info);
  st().setScanning(info?.scanning ?? false);
  return info;
}

/** Starts a scan. A folder scanned before is only checked for changes, unless `full` is set. */
export async function startScan(path: string, crossVolumes: boolean, full = false) {
  await guarded(null, async () => {
    await api.scanStart(path, crossVolumes, full);
    const info = await api.treeInfo();
    st().resetForScan(info, true);
  });
}

/** Shows the last kept scan, if any, when the app starts with nothing else to show. */
export async function restoreLastScan() {
  await guarded(null, async () => {
    if ((await api.restoreLastScan()) === null) return;
    const info = await api.treeInfo();
    st().resetForScan(info, info?.scanning ?? false);
  });
}

export async function chooseAndScan(crossVolumes: boolean) {
  const path = await open({ directory: true, multiple: false, title: "Wybierz folder do przeskanowania" });
  if (typeof path === "string") await startScan(path, crossVolumes);
}

export async function cancelScan() {
  await guarded(null, () => api.scanCancel());
}

export async function rescan(id: number) {
  await guarded("Ponowne skanowanie…", async () => {
    await api.rescanNode(id);
    await refreshInfo();
    st().bump();
    st().toast("Przeskanowano ponownie.", "success");
  });
}

/** Right-click on a node: select it and open its context menu at the pointer. */
export function openNodeMenu(e: { preventDefault(): void; clientX: number; clientY: number }, id: number, source: "tree" | "map" | "other" = "other") {
  e.preventDefault();
  st().select(id, source);
  st().openMenu({ x: e.clientX, y: e.clientY, id });
}

export async function reveal(id: number) {
  await guarded(null, () => api.reveal(id));
}

export async function openTerminal(id: number) {
  await guarded(null, () => api.openTerminal(id));
}

export async function copyPath(id: number) {
  const p = await guarded(null, () => api.copyPath(id));
  if (p) st().toast(`Skopiowano: ${p}`, "success");
}

/**
 * Moves the items to the Trash and refreshes the views. Returns what happened, or `undefined`
 * when the command itself failed (the error is shown as a toast).
 */
export async function moveToTrash(ids: number[], verify: boolean): Promise<TrashOutcome | undefined> {
  const outcome = await guarded("Przenoszenie do Kosza…", () => api.moveToTrash(ids, verify));
  if (!outcome) return undefined;
  const s = st();
  if (outcome.trashed > 0) {
    // The selected node may be one of the moved items or lie inside one.
    s.select(null);
    await refreshInfo();
    s.bump();
  }
  return outcome;
}

/** Exports the current view, or the given stale list when `stale` is set. */
export async function exportView(format: "csv" | "json", stale: StaleArgs | null = null) {
  const s = st();
  const path = await save({
    title: stale ? "Eksportuj listę" : "Eksportuj bieżący widok",
    defaultPath: `disk-lens${stale ? "-stare" : ""}.${format}`,
    filters: [{ name: format.toUpperCase(), extensions: [format] }],
  });
  if (!path) return;
  const n = await guarded("Eksport…", () =>
    api.exportView(s.root, { mode: s.mode, filter: s.filter }, s.sortKey, s.sortDesc, format, path, stale),
  );
  if (n !== undefined) s.toast(`Wyeksportowano ${formatCount(n)} pozycji.`, "success");
}

export async function saveScan() {
  const path = await save({
    title: "Zapisz wynik skanu",
    defaultPath: `skan-${new Date().toISOString().slice(0, 10)}.disklens`,
    filters: [{ name: "Skan Disk Lens", extensions: ["disklens"] }],
  });
  if (!path) return;
  await guarded("Zapisywanie skanu…", async () => {
    await api.saveScan(path);
    st().toast("Zapisano skan.", "success");
  });
}

export async function pickScanFile(title: string): Promise<string | null> {
  const path = await open({ title, multiple: false, filters: [{ name: "Skan Disk Lens", extensions: ["disklens"] }] });
  return typeof path === "string" ? path : null;
}

export async function loadScan() {
  const path = await pickScanFile("Wczytaj zapisany skan");
  if (!path) return;
  await guarded("Wczytywanie skanu…", async () => {
    await api.loadScan(path);
    const info = await api.treeInfo();
    st().resetForScan(info, false);
    st().toast("Wczytano skan.", "success");
  });
}
