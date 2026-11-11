import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, isDirKind, TILE_REST, type NodeInfo, type Tile } from "../api";
import { openNodeMenu } from "../lib/actions";
import { AGE_COLORS, AGE_NAMES, CATEGORY_COLORS, CATEGORY_NAMES } from "../lib/colors";
import { formatBytes, formatPct } from "../lib/format";
import { buildGrid, drawTiles, hitTest } from "../lib/treemapCanvas";
import { useStore, useViewArgs } from "../store";

/** During a scan the treemap refreshes less often than the list: the layout is more expensive. */
const SCAN_REFRESH_MS = 1200;

export function Treemap() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const scanning = useStore((s) => s.scanning);
  const colorMode = useStore((s) => s.colorMode);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const navigate = useStore((s) => s.navigate);
  const setColorMode = useStore((s) => s.setColorMode);
  const view = useViewArgs();

  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [tiles, setTiles] = useState<Tile[]>([]);
  const [loading, setLoading] = useState(false);
  const [hover, setHover] = useState<{ idx: number; x: number; y: number } | null>(null);
  const [hoverInfo, setHoverInfo] = useState<NodeInfo | null>(null);
  const [highlight, setHighlight] = useState<number | null>(null);
  const [theme, setTheme] = useState(0);
  const lastFetch = useRef(0);

  // Canvas size = container size.
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: Math.floor(el.clientWidth), h: Math.floor(el.clientHeight) }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Redraw when the system theme changes.
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const on = () => setTheme((t) => t + 1);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);

  // Fetch the layout (debounced on resize and throttled during a scan).
  useEffect(() => {
    if (size.w < 10 || size.h < 10) return;
    let alive = true;
    const since = Date.now() - lastFetch.current;
    const delay = scanning ? Math.max(0, SCAN_REFRESH_MS - since) : 80;
    const timer = setTimeout(async () => {
      lastFetch.current = Date.now();
      setLoading(true);
      try {
        const t = await api.treemap(root, view, size.w, size.h);
        if (alive) setTiles(t);
      } catch {
        if (alive) setTiles([]);
      } finally {
        if (alive) setLoading(false);
      }
    }, delay);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [root, version, view, size, scanning]);

  const grid = useMemo(() => buildGrid(tiles, size.w, size.h), [tiles, size]);
  const idToIdx = useMemo(() => {
    const m = new Map<number, number>();
    tiles.forEach((t, i) => {
      if (t.kind !== TILE_REST) m.set(t.id, i);
    });
    return m;
  }, [tiles]);

  // Selection made outside the map (e.g. a deep file): highlight the deepest visible ancestor.
  useEffect(() => {
    if (selected === null) {
      setHighlight(null);
      return;
    }
    if (idToIdx.has(selected)) {
      setHighlight(selected);
      return;
    }
    let alive = true;
    api
      .getAncestors(selected)
      .then((chain) => {
        if (!alive) return;
        const hit = [...chain].reverse().find((c) => idToIdx.has(c.id));
        setHighlight(hit ? hit.id : null);
      })
      .catch(() => alive && setHighlight(null));
    return () => {
      alive = false;
    };
  }, [selected, idToIdx]);

  // Drawing.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.w === 0) return;
    const hoverTile = hover ? tiles[hover.idx] : undefined;
    const at = highlight !== null ? idToIdx.get(highlight) : undefined;
    drawTiles(canvas, tiles, size.w, size.h, colorMode, hoverTile, at !== undefined ? tiles[at] : undefined);
  }, [tiles, size, colorMode, hover, highlight, idToIdx, theme]);

  // Tooltip: data of the node under the pointer (with a short delay).
  const hoverTile = hover ? tiles[hover.idx] : undefined;
  const hoverId = hoverTile ? hoverTile.id : null;
  useEffect(() => {
    if (hoverId === null) {
      setHoverInfo(null);
      return;
    }
    let alive = true;
    const timer = setTimeout(() => {
      api
        .getNode(hoverId, view)
        .then((n) => alive && setHoverInfo(n))
        .catch(() => alive && setHoverInfo(null));
    }, 60);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [hoverId, view]);

  const pick = useCallback(
    (e: React.MouseEvent) => {
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!rect) return -1;
      return hitTest(grid, tiles, e.clientX - rect.left, e.clientY - rect.top);
    },
    [grid, tiles],
  );

  const onMove = (e: React.MouseEvent) => {
    const idx = pick(e);
    const rect = wrapRef.current!.getBoundingClientRect();
    setHover(idx < 0 ? null : { idx, x: e.clientX - rect.left, y: e.clientY - rect.top });
  };

  const onClick = (e: React.MouseEvent) => {
    const idx = pick(e);
    if (idx >= 0) select(tiles[idx].id, "map");
  };

  // Step-wise zoom: go one level down, into the folder (a child of the root) under the pointer.
  const onDoubleClick = async (e: React.MouseEvent) => {
    const idx = pick(e);
    if (idx < 0) return;
    const chain = await api.getAncestors(tiles[idx].id).catch(() => null);
    if (!chain) return;
    const at = chain.findIndex((c) => c.id === root);
    const next = at >= 0 ? chain[at + 1] : undefined;
    if (!next) return;
    const n = await api.getNode(next.id, view).catch(() => null);
    if (n && isDirKind(n.kind)) navigate(n.id);
  };

  const onContextMenu = (e: React.MouseEvent) => {
    const idx = pick(e);
    if (idx < 0) {
      e.preventDefault();
      return;
    }
    openNodeMenu(e, tiles[idx].id, "map");
  };

  const tip = (() => {
    if (!hover || !hoverTile) return null;
    const left = hover.x + 16 + 280 > size.w ? hover.x - 296 : hover.x + 16;
    const top = Math.min(hover.y + 16, Math.max(0, size.h - 90));
    if (hoverTile.kind === TILE_REST) {
      return (
        <div className="map-tip" style={{ left, top }}>
          <strong>Drobne elementy</strong>
          <span>zbyt małe, by narysować je osobno{hoverInfo ? ` · w „${hoverInfo.name}”` : ""}</span>
          <span className="muted">Dwuklik powiększa folder</span>
        </div>
      );
    }
    if (!hoverInfo) return null;
    return (
      <div className="map-tip" style={{ left, top }}>
        <strong>{hoverInfo.name}</strong>
        <span>
          {formatBytes(hoverInfo.size)} · {formatPct(hoverInfo.pctOfRoot)} całości
        </span>
        <span className="muted path">{hoverInfo.path}</span>
      </div>
    );
  })();

  const legend =
    colorMode === "type"
      ? CATEGORY_NAMES.slice(0, 8).map((n, i) => ({ name: n, color: CATEGORY_COLORS[i] }))
      : AGE_NAMES.map((n, i) => ({ name: n, color: AGE_COLORS[i] }));

  return (
    <div className="treemap">
      <div
        className="map-wrap"
        ref={wrapRef}
        onMouseMove={onMove}
        onMouseLeave={() => setHover(null)}
        onClick={onClick}
        onDoubleClick={onDoubleClick}
        onContextMenu={onContextMenu}
      >
        <canvas ref={canvasRef} />
        {tip}
        {tiles.length === 0 && !loading ? <div className="map-empty">Brak danych do narysowania.</div> : null}
      </div>
      <div className="legend">
        <div className="seg" role="group" aria-label="Kolorowanie">
          <button className={colorMode === "type" ? "on" : ""} onClick={() => setColorMode("type")}>
            Typ
          </button>
          <button className={colorMode === "age" ? "on" : ""} onClick={() => setColorMode("age")}>
            Wiek
          </button>
        </div>
        {legend.map((l) => (
          <span key={l.name} className="legend-item">
            <span className="swatch" style={{ background: l.color }} />
            {l.name}
          </span>
        ))}
        <span className="legend-item">
          <span className="swatch rest" />
          drobne
        </span>
        <span className="legend-hint">klik: zaznacz · dwuklik: wejdź poziom niżej · Esc: wstecz · prawy: akcje</span>
      </div>
    </div>
  );
}
