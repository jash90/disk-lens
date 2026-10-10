import { useMemo } from "react";
import { create } from "zustand";
import { EMPTY_FILTER, type Filter, type Progress, type SizeMode, type SortKey, type TreeInfo, type ViewArgs } from "./api";
import type { ColorMode } from "./lib/colors";

export type Tab = "treemap" | "top" | "types" | "skipped" | "diff";
export type SelectSource = "tree" | "map" | "other";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error" | "success";
}

interface ContextMenuState {
  x: number;
  y: number;
  id: number;
}

interface State {
  info: TreeInfo | null;
  progress: Progress | null;
  scanning: boolean;
  /** Korzeń bieżącego widoku (id węzła). */
  root: number;
  back: number[];
  selected: number | null;
  /** Prośba do drzewa folderów o rozwinięcie i przewinięcie do zaznaczenia. */
  reveal: { id: number; seq: number } | null;
  mode: SizeMode;
  filter: Filter;
  sortKey: SortKey;
  sortDesc: boolean;
  colorMode: ColorMode;
  tab: Tab;
  /** Rośnie przy każdej zmianie danych – komponenty pobierają je ponownie. */
  version: number;
  searchFocus: number;
  busy: string | null;
  toasts: Toast[];
  trashTarget: number | null;
  menu: ContextMenuState | null;

  setInfo: (info: TreeInfo | null) => void;
  resetForScan: (info: TreeInfo | null, scanning: boolean) => void;
  setProgress: (p: Progress | null) => void;
  setScanning: (s: boolean) => void;
  navigate: (id: number) => void;
  goBack: () => void;
  select: (id: number | null, source?: SelectSource) => void;
  setMode: (m: SizeMode) => void;
  setFilter: (f: Filter) => void;
  setSort: (key: SortKey) => void;
  setColorMode: (m: ColorMode) => void;
  setTab: (t: Tab) => void;
  bump: () => void;
  focusSearch: () => void;
  setBusy: (b: string | null) => void;
  toast: (text: string, kind?: Toast["kind"]) => void;
  dismissToast: (id: number) => void;
  setTrashTarget: (id: number | null) => void;
  openMenu: (m: ContextMenuState | null) => void;
}

let toastSeq = 0;
let revealSeq = 0;

export const useStore = create<State>((set, get) => ({
  info: null,
  progress: null,
  scanning: false,
  root: 0,
  back: [],
  selected: null,
  reveal: null,
  mode: "alloc",
  filter: EMPTY_FILTER,
  sortKey: "size",
  sortDesc: true,
  colorMode: "type",
  tab: "treemap",
  version: 0,
  searchFocus: 0,
  busy: null,
  toasts: [],
  trashTarget: null,
  menu: null,

  setInfo: (info) => set({ info }),
  resetForScan: (info, scanning) =>
    set((s) => ({ info, scanning, root: 0, back: [], selected: null, reveal: null, progress: null, version: s.version + 1 })),
  setProgress: (progress) => set({ progress }),
  setScanning: (scanning) => set({ scanning }),
  navigate: (id) => {
    const { root, back } = get();
    if (id === root) return;
    set({ root: id, back: [...back, root].slice(-100) });
  },
  goBack: () => {
    const { back } = get();
    if (back.length === 0) return;
    set({ root: back[back.length - 1], back: back.slice(0, -1) });
  },
  select: (id, source = "other") =>
    set(() => ({
      selected: id,
      reveal: id !== null && source !== "tree" ? { id, seq: ++revealSeq } : null,
    })),
  setMode: (mode) => set({ mode }),
  setFilter: (filter) => set({ filter }),
  setSort: (key) =>
    set((s) => (s.sortKey === key ? { sortDesc: !s.sortDesc } : { sortKey: key, sortDesc: key !== "name" })),
  setColorMode: (colorMode) => set({ colorMode }),
  setTab: (tab) => set({ tab }),
  bump: () => set((s) => ({ version: s.version + 1 })),
  focusSearch: () => set((s) => ({ searchFocus: s.searchFocus + 1 })),
  setBusy: (busy) => set({ busy }),
  toast: (text, kind = "info") => {
    const id = ++toastSeq;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { id, text, kind }] }));
    setTimeout(() => get().dismissToast(id), kind === "error" ? 7000 : 3500);
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  setTrashTarget: (trashTarget) => set({ trashTarget }),
  openMenu: (menu) => set({ menu }),
}));

/** Parametry widoku przekazywane do każdego zapytania. */
export function useViewArgs(): ViewArgs {
  const mode = useStore((s) => s.mode);
  const filter = useStore((s) => s.filter);
  return useMemo(() => ({ mode, filter }), [mode, filter]);
}
