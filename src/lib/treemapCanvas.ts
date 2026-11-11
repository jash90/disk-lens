import { TILE_DIR, TILE_REST, type Tile } from "../api";
import { tileColor, type ColorMode } from "./colors";

const CELL = 24;

/** Spatial index over the tiles: for each grid cell, the tiles that touch it. */
export interface Grid {
  cols: number;
  rows: number;
  cells: number[][];
}

export function buildGrid(tiles: Tile[], w: number, h: number): Grid {
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

/** Index of the topmost tile at (x, y), or -1. */
export function hitTest(grid: Grid, tiles: Tile[], x: number, y: number): number {
  const c = Math.floor(x / CELL);
  const r = Math.floor(y / CELL);
  if (c < 0 || r < 0 || c >= grid.cols || r >= grid.rows) return -1;
  const list = grid.cells[r * grid.cols + c];
  // Tiles are in BFS order: deeper ones (drawn on top) come later.
  for (let k = list.length - 1; k >= 0; k--) {
    const t = tiles[list[k]];
    if (x >= t.x && x < t.x + t.w && y >= t.y && y < t.y + t.h) return list[k];
  }
  return -1;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** Darkens (negative) or lightens a #rrggbb colour. */
function shade(hex: string, amount: number): string {
  const n = parseInt(hex.slice(1), 16);
  const f = (c: number) => Math.round(amount < 0 ? c * (1 + amount) : c + (255 - c) * amount);
  const r = f((n >> 16) & 255);
  const g = f((n >> 8) & 255);
  const b = f(n & 255);
  return `rgb(${r},${g},${b})`;
}

/** Draws the tiles, then the hover outline and the selection outline, on a canvas of `w` x `h` CSS pixels. */
export function drawTiles(
  canvas: HTMLCanvasElement,
  tiles: Tile[],
  w: number,
  h: number,
  colorMode: ColorMode,
  hoverTile: Tile | undefined,
  highlightTile: Tile | undefined,
) {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.floor(w * dpr);
  canvas.height = Math.floor(h * dpr);
  canvas.style.width = `${w}px`;
  canvas.style.height = `${h}px`;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = cssVar("--map-bg");
  ctx.fillRect(0, 0, w, h);
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
      // A "cushion" as in GrandPerspective: lighter top-left, darker bottom-right.
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
  if (hoverTile) outline(hoverTile, "rgba(255,255,255,0.85)", 1);
  if (highlightTile) {
    outline(highlightTile, "rgba(0,0,0,0.9)", 4);
    outline(highlightTile, "#ffffff", 2);
  }
}
