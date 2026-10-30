import { t } from "../i18n";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import { formatBytes, formatCount } from "./format";

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

export async function startScan(path: string, crossVolumes: boolean) {
  await guarded(null, async () => {
    await api.scanStart(path, crossVolumes);
    const info = await api.treeInfo();
    st().resetForScan(info, true);
  });
}

export async function chooseAndScan(crossVolumes: boolean) {
  const path = await open({ directory: true, multiple: false, title: t("Wybierz folder do przeskanowania") });
  if (typeof path === "string") await startScan(path, crossVolumes);
}

export async function cancelScan() {
  await guarded(null, () => api.scanCancel());
}

export async function rescan(id: number) {
  await guarded(t("Ponowne skanowanie…"), async () => {
    await api.rescanNode(id);
    await refreshInfo();
    st().bump();
    st().toast(t("Przeskanowano ponownie."), "success");
  });
}

export async function reveal(id: number) {
  await guarded(null, () => api.reveal(id));
}

export async function openTerminal(id: number) {
  await guarded(null, () => api.openTerminal(id));
}

export async function copyPath(id: number) {
  const p = await guarded(null, () => api.copyPath(id));
  if (p) st().toast(t("Skopiowano: {path}", { path: p }), "success");
}

export async function confirmTrash(id: number) {
  const freed = await guarded(t("Przenoszenie do Kosza…"), () => api.moveToTrash(id));
  if (freed === undefined) return;
  const s = st();
  if (s.selected === id) s.select(null);
  s.setTrashTarget(null);
  await refreshInfo();
  s.bump();
  s.toast(t("Przeniesiono do Kosza ({size}).", { size: formatBytes(freed) }), "success");
}

export async function exportView(format: "csv" | "json") {
  const s = st();
  const path = await save({
    title: t("Eksportuj bieżący widok"),
    defaultPath: `disk-lens.${format}`,
    filters: [{ name: format.toUpperCase(), extensions: [format] }],
  });
  if (!path) return;
  const n = await guarded(t("Eksport…"), () =>
    api.exportView(s.root, { mode: s.mode, filter: s.filter }, s.sortKey, s.sortDesc, format, path),
  );
  if (n !== undefined) s.toast(t("Wyeksportowano {count} pozycji.", { count: formatCount(n) }), "success");
}

export async function saveScan() {
  const path = await save({
    title: t("Zapisz wynik skanu"),
    defaultPath: `skan-${new Date().toISOString().slice(0, 10)}.disklens`,
    filters: [{ name: t("Skan Disk Lens"), extensions: ["disklens"] }],
  });
  if (!path) return;
  await guarded(t("Zapisywanie skanu…"), async () => {
    await api.saveScan(path);
    st().toast(t("Zapisano skan."), "success");
  });
}

export async function pickScanFile(title: string): Promise<string | null> {
  const path = await open({ title, multiple: false, filters: [{ name: t("Skan Disk Lens"), extensions: ["disklens"] }] });
  return typeof path === "string" ? path : null;
}

export async function loadScan() {
  const path = await pickScanFile(t("Wczytaj zapisany skan"));
  if (!path) return;
  await guarded(t("Wczytywanie skanu…"), async () => {
    await api.loadScan(path);
    const info = await api.treeInfo();
    st().resetForScan(info, false);
    st().toast(t("Wczytano skan."), "success");
  });
}
