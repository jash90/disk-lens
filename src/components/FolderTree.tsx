import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, Flags, type Page, type Row, type SortKey } from "../api";
import { kindIcon } from "../lib/colors";
import { formatBytes, formatCount, formatDate, formatPct } from "../lib/format";
import { useStore, useViewArgs } from "../store";

const PAGE = 500;
const MAX_PAGE = 5000;
const ROW_H = 26;
const INDENT = 12;

/** Poniżej tych szerokości chowamy kolumny liczbowe, żeby nazwy miały miejsce. */
function widthClass(w: number): string {
  if (w < 540) return "w-sm";
  if (w < 720) return "w-md";
  return "";
}

type Flat =
  | { type: "row"; row: Row; depth: number; parentSize: number }
  | { type: "more"; parent: number; depth: number; hidden: number; canLoad: boolean };

const COLUMNS: { key: SortKey | null; label: string; className: string }[] = [
  { key: "name", label: "Nazwa", className: "c-name" },
  { key: "size", label: "Rozmiar", className: "c-size" },
  { key: null, label: "% rodzica", className: "c-pct" },
  { key: "files", label: "Pliki", className: "c-num c-files" },
  { key: "dirs", label: "Foldery", className: "c-num c-dirs" },
  { key: "mtime", label: "Zmieniono", className: "c-date" },
];

function FlagBadges({ flags }: { flags: number }) {
  return (
    <>
      {flags & Flags.UNREADABLE ? <span className="badge warn" title="Brak dostępu – zawartość nie została policzona">⚠</span> : null}
      {flags & Flags.DATALESS ? <span className="badge" title="Plik tylko w iCloud – nie zajmuje miejsca lokalnie">☁</span> : null}
      {flags & Flags.OTHER_VOLUME ? <span className="badge" title="Inny wolumen – nie wchodzimy do środka">⛁</span> : null}
      {flags & Flags.HARDLINK_DUP ? <span className="badge" title="Kolejne twarde dowiązanie – rozmiar policzony gdzie indziej">🔗</span> : null}
    </>
  );
}

export function FolderTree() {
  const root = useStore((s) => s.root);
  const scanId = useStore((s) => s.info?.scanId ?? 0);
  const version = useStore((s) => s.version);
  const sortKey = useStore((s) => s.sortKey);
  const sortDesc = useStore((s) => s.sortDesc);
  const selected = useStore((s) => s.selected);
  const reveal = useStore((s) => s.reveal);
  const select = useStore((s) => s.select);
  const navigate = useStore((s) => s.navigate);
  const setSort = useStore((s) => s.setSort);
  const openMenu = useStore((s) => s.openMenu);
  const view = useViewArgs();

  const [pages, setPages] = useState<Map<number, Page>>(new Map());
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const limits = useRef(new Map<number, number>());
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;
  const [scrollTo, setScrollTo] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  const [sizeClass, setSizeClass] = useState("");

  useEffect(() => {
    const el = treeRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSizeClass(widthClass(el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const fetchPage = useCallback(
    (id: number) => api.getChildren(id, view, sortKey, sortDesc, 0, limits.current.get(id) ?? PAGE),
    [view, sortKey, sortDesc],
  );

  // Nowy korzeń / nowy skan: zwijamy wszystko.
  useEffect(() => {
    setExpanded(new Set());
    setPages(new Map());
    limits.current.clear();
  }, [root, scanId]);

  // Odświeżenie wszystkich widocznych poziomów po zmianie danych, sortowania lub filtra.
  useEffect(() => {
    let alive = true;
    const ids = [root, ...expandedRef.current];
    Promise.all(ids.map((id) => fetchPage(id).then((p) => [id, p] as const).catch(() => null))).then((res) => {
      if (!alive) return;
      const next = new Map<number, Page>();
      for (const r of res) if (r) next.set(r[0], r[1]);
      setPages(next);
      // Foldery, które zniknęły (Kosz, reskan), wypadają z rozwiniętych.
      setExpanded((prev) => {
        const keep = new Set([...prev].filter((id) => next.has(id)));
        return keep.size === prev.size ? prev : keep;
      });
    });
    return () => {
      alive = false;
    };
  }, [root, version, fetchPage]);

  const toggle = useCallback(
    async (id: number) => {
      if (expandedRef.current.has(id)) {
        setExpanded((prev) => {
          const n = new Set(prev);
          n.delete(id);
          return n;
        });
        return;
      }
      const page = await fetchPage(id).catch(() => null);
      if (!page) return;
      setPages((prev) => new Map(prev).set(id, page));
      setExpanded((prev) => new Set(prev).add(id));
    },
    [fetchPage],
  );

  const loadMore = useCallback(
    async (id: number) => {
      limits.current.set(id, Math.min((limits.current.get(id) ?? PAGE) * 4, MAX_PAGE));
      const page = await fetchPage(id).catch(() => null);
      if (page) setPages((prev) => new Map(prev).set(id, page));
    },
    [fetchPage],
  );

  // Zaznaczenie z treemapy / listy: rozwiń przodków i przewiń do wiersza.
  useEffect(() => {
    if (!reveal) return;
    let alive = true;
    (async () => {
      const chain = await api.getAncestors(reveal.id).catch(() => null);
      if (!chain || !alive) return;
      const at = chain.findIndex((c) => c.id === root);
      if (at < 0) return;
      const toOpen = chain.slice(at + 1, -1).map((c) => c.id);
      const missing = toOpen.filter((id) => !expandedRef.current.has(id));
      const fetched = await Promise.all(missing.map((id) => fetchPage(id).then((p) => [id, p] as const).catch(() => null)));
      if (!alive) return;
      setPages((prev) => {
        const n = new Map(prev);
        for (const f of fetched) if (f) n.set(f[0], f[1]);
        return n;
      });
      setExpanded((prev) => new Set([...prev, ...toOpen]));
      setScrollTo(reveal.id);
    })();
    return () => {
      alive = false;
    };
  }, [reveal, root, fetchPage]);

  const flat = useMemo(() => {
    const out: Flat[] = [];
    const walk = (id: number, depth: number) => {
      const page = pages.get(id);
      if (!page) return;
      for (const row of page.rows) {
        out.push({ type: "row", row, depth, parentSize: page.parentSize });
        if (expanded.has(row.id) && row.kind === "dir") walk(row.id, depth + 1);
      }
      if (page.total > page.rows.length) {
        out.push({
          type: "more",
          parent: id,
          depth,
          hidden: page.total - page.rows.length,
          canLoad: page.rows.length < MAX_PAGE,
        });
      }
    };
    walk(root, 0);
    return out;
  }, [pages, expanded, root]);

  const virt = useVirtualizer({
    count: flat.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 20,
  });

  useEffect(() => {
    if (scrollTo === null) return;
    const i = flat.findIndex((f) => f.type === "row" && f.row.id === scrollTo);
    if (i >= 0) {
      virt.scrollToIndex(i, { align: "auto" });
      setScrollTo(null);
    }
  }, [scrollTo, flat, virt]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const i = flat.findIndex((f) => f.type === "row" && f.row.id === selected);
    const rowAt = (j: number) => {
      const f = flat[j];
      return f && f.type === "row" ? f.row : null;
    };
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const dir = e.key === "ArrowDown" ? 1 : -1;
      let j = i < 0 ? 0 : i + dir;
      while (j >= 0 && j < flat.length && !rowAt(j)) j += dir;
      const r = rowAt(j);
      if (r) {
        select(r.id, "tree");
        virt.scrollToIndex(j, { align: "auto" });
      }
    } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      const r = rowAt(i);
      if (r && r.kind === "dir" && r.hasChildren && (e.key === "ArrowRight") !== expanded.has(r.id)) {
        e.preventDefault();
        toggle(r.id);
      }
    } else if (e.key === "Enter") {
      const r = rowAt(i);
      if (r && (r.kind === "dir" || r.kind === "package")) navigate(r.id);
    }
  };

  const rootPage = pages.get(root);

  return (
    <div className={`tree ${sizeClass}`} ref={treeRef}>
      <div className="tree-head" role="row">
        {COLUMNS.map((c) => (
          <button
            key={c.label}
            className={`th ${c.className} ${c.key && c.key === sortKey ? "active" : ""}`}
            disabled={!c.key}
            onClick={() => c.key && setSort(c.key)}
          >
            {c.label}
            {c.key === sortKey ? <span className="sort-arrow">{sortDesc ? "▼" : "▲"}</span> : null}
          </button>
        ))}
      </div>
      <div className="tree-body" ref={scrollRef} tabIndex={0} onKeyDown={onKeyDown}>
        {rootPage && rootPage.total === 0 ? <div className="empty-note">Brak elementów do pokazania.</div> : null}
        <div style={{ height: virt.getTotalSize(), position: "relative" }}>
          {virt.getVirtualItems().map((vi) => {
            const f = flat[vi.index];
            const style = { transform: `translateY(${vi.start}px)`, height: ROW_H };
            if (f.type === "more") {
              return (
                <div key={`more-${f.parent}`} className="tr more" style={style}>
                  <span style={{ paddingLeft: 12 + f.depth * INDENT }}>
                    … i {formatCount(f.hidden)} mniejszych elementów{" "}
                    {f.canLoad ? (
                      <button className="link" onClick={() => loadMore(f.parent)}>
                        pokaż więcej
                      </button>
                    ) : (
                      <em>(zawęź filtrem)</em>
                    )}
                  </span>
                </div>
              );
            }
            const r = f.row;
            const isDir = r.kind === "dir";
            const isOpen = expanded.has(r.id);
            return (
              <div
                key={r.id}
                className={`tr ${selected === r.id ? "selected" : ""} ${r.flags & Flags.HIDDEN ? "hidden-file" : ""}`}
                style={style}
                onClick={() => select(r.id, "tree")}
                onDoubleClick={() => (isDir || r.kind === "package" ? navigate(r.id) : undefined)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  select(r.id, "tree");
                  openMenu({ x: e.clientX, y: e.clientY, id: r.id });
                }}
              >
                <span className="c-name" style={{ paddingLeft: 6 + f.depth * INDENT }}>
                  <span
                    className={`chev ${isDir && r.hasChildren ? "" : "invisible"} ${isOpen ? "open" : ""}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (isDir && r.hasChildren) toggle(r.id);
                    }}
                  >
                    ▶
                  </span>
                  <span className="icon">{kindIcon(r.kind, r.cat)}</span>
                  <span className="name" title={r.name}>
                    {r.name}
                  </span>
                  <FlagBadges flags={r.flags} />
                </span>
                <span className="c-size">{formatBytes(r.size)}</span>
                <span className="c-pct">
                  <span className="bar">
                    <span className="fill" style={{ width: `${Math.min(100, r.pct)}%` }} />
                  </span>
                  <span className="pct-label">{formatPct(r.pct)}</span>
                </span>
                <span className="c-num c-files">{r.kind === "dir" || r.kind === "package" ? formatCount(r.files) : ""}</span>
                <span className="c-num c-dirs">{r.kind === "dir" || r.kind === "package" ? formatCount(r.dirs) : ""}</span>
                <span className="c-date">{formatDate(r.mtime, true)}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
