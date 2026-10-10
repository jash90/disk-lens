import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, TILE_DIR, TILE_REST, type NodeInfo, type Tile } from "../api";
import { AGE_COLORS, AGE_NAMES, CATEGORY_COLORS, CATEGORY_NAMES, tileColor } from "../lib/colors";
import { formatBytes, formatPct } from "../lib/format";
import { useStore, useViewArgs } from "../store";

const CELL = 24;
/** W trakcie skanu treemapę odświeżamy rzadziej niż listę – layout jest droższy. */
const SCAN_REFRESH_MS = 1200;

interface Grid {
  cols: number;
  rows: number;
  cells: number[][];
}

function buildGrid(tiles: Tile[], w: number, h: number): Grid {
  const cols = Math.max(1, Math.ceil(w / CELL));
  const rows = Math.max(1, Math.ceil(h / CELL));
  const cells: number[][] = Array.from({ length: cols * rows }, () => []);
  tiles.forEach((t, i) => {
    if (t.w <= 0 || t.h <= 0) return;
    const c0 = Math.max(0, Math.floor(t.x / CELL));
    const c1 = Math.min(cols - 1, Math.floor((t.x + t.w) / CELL));
    const r0 = Math.max(0, Math.floor(t.y / CELL));
    const r1 = Math.min(rows - 1, Math.floor((t.y + t.h) / CELL));
    for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) cells[r * cols + c].push(i);
  });
  return { cols, rows, cells };
}

function hitTest(grid: Grid, tiles: Tile[], x: number, y: number): number {
  const c = Math.floor(x / CELL);
  const r = Math.floor(y / CELL);
  if (c < 0 || r < 0 || c >= grid.cols || r >= grid.rows) return -1;
  const list = grid.cells[r * grid.cols + c];
  // Kafle są w kolejności BFS – głębsze (leżące na wierzchu) są dalej.
  for (let k = list.length - 1; k >= 0; k--) {
    const t = tiles[list[k]];
    if (x >= t.x && x < t.x + t.w && y >= t.y && y < t.y + t.h) return list[k];
  }
  return -1;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function Treemap() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const scanning = useStore((s) => s.scanning);
  const colorMode = useStore((s) => s.colorMode);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const navigate = useStore((s) => s.navigate);
  const openMenu = useStore((s) => s.openMenu);
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

  // Rozmiar płótna = rozmiar kontenera.
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: Math.floor(el.clientWidth), h: Math.floor(el.clientHeight) }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Przerysowanie po zmianie motywu systemowego.
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const on = () => setTheme((t) => t + 1);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);

  // Pobranie layoutu (z debounce przy zmianie rozmiaru i dławieniem w trakcie skanu).
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

  // Zaznaczenie spoza mapy (np. głęboki plik): podświetl najgłębszego widocznego przodka.
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

  // Rysowanie.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.w === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.floor(size.w * dpr);
    canvas.height = Math.floor(size.h * dpr);
    canvas.style.width = `${size.w}px`;
    canvas.style.height = `${size.h}px`;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = cssVar("--map-bg");
    ctx.fillRect(0, 0, size.w, size.h);
    const dirFill = [cssVar("--map-dir-0"), cssVar("--map-dir-1")];
    const restFill = cssVar("--map-rest");

    for (const t of tiles) {
      if (t.w <= 0 || t.h <= 0) continue;
      if (t.kind === TILE_DIR) {
        ctx.fillStyle = dirFill[t.depth % 2];
        ctx.fillRect(t.x, t.y, t.w, t.h);
        continue;
      }
      if (t.kind === TILE_REST) {
        ctx.fillStyle = restFill;
        ctx.fillRect(t.x, t.y, t.w, t.h);
        continue;
      }
      const color = tileColor(colorMode, t.cat, t.age);
      if (t.w * t.h > 300) {
        // „Poduszka” jak w GrandPerspective: jaśniej u góry z lewej, ciemniej na dole z prawej.
        const g = ctx.createLinearGradient(t.x, t.y, t.x + t.w, t.y + t.h);
        g.addColorStop(0, color);
        g.addColorStop(1, shade(color, -0.35));
        ctx.fillStyle = g;
      } else {
        ctx.fillStyle = color;
      }
      ctx.fillRect(t.x, t.y, t.w, t.h);
      if (t.w > 3 && t.h > 3) {
        ctx.strokeStyle = "rgba(0,0,0,0.28)";
        ctx.lineWidth = 0.5;
        ctx.strokeRect(t.x + 0.25, t.y + 0.25, t.w - 0.5, t.h - 0.5);
      }
    }

    const outline = (t: Tile, color: string, width: number) => {
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.strokeRect(t.x + width / 2, t.y + width / 2, Math.max(0, t.w - width), Math.max(0, t.h - width));
    };
    if (hover && tiles[hover.idx]) outline(tiles[hover.idx], "rgba(255,255,255,0.85)", 1);
    if (highlight !== null) {
      const i = idToIdx.get(highlight);
      if (i !== undefined) {
        outline(tiles[i], "rgba(0,0,0,0.9)", 4);
        outline(tiles[i], "#ffffff", 2);
      }
    }
  }, [tiles, size, colorMode, hover, highlight, idToIdx, theme]);

  // Tooltip: dane węzła pod kursorem (z małym opóźnieniem).
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

  // Zoom stopniowy: wchodzimy o jeden poziom – w folder (dziecko korzenia) leżący pod kursorem.
  const onDoubleClick = async (e: React.MouseEvent) => {
    const idx = pick(e);
    if (idx < 0) return;
    const chain = await api.getAncestors(tiles[idx].id).catch(() => null);
    if (!chain) return;
    const at = chain.findIndex((c) => c.id === root);
    const next = at >= 0 ? chain[at + 1] : undefined;
    if (!next) return;
    const n = await api.getNode(next.id, view).catch(() => null);
    if (n && (n.kind === "dir" || n.kind === "package")) navigate(n.id);
  };

  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    const idx = pick(e);
    if (idx < 0) return;
    const id = tiles[idx].id;
    select(id, "map");
    openMenu({ x: e.clientX, y: e.clientY, id });
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

/** Przyciemnia (ujemne) lub rozjaśnia kolor #rrggbb. */
function shade(hex: string, amount: number): string {
  const n = parseInt(hex.slice(1), 16);
  const f = (c: number) => Math.round(amount < 0 ? c * (1 + amount) : c + (255 - c) * amount);
  const r = f((n >> 16) & 255);
  const g = f((n >> 8) & 255);
  const b = f(n & 255);
  return `rgb(${r},${g},${b})`;
}

