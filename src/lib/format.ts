const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];
const num1 = new Intl.NumberFormat("pl-PL", { maximumFractionDigits: 1, minimumFractionDigits: 1 });
const num0 = new Intl.NumberFormat("pl-PL", { maximumFractionDigits: 0 });
const dateFmt = new Intl.DateTimeFormat("pl-PL", { dateStyle: "medium", timeStyle: "short" });
const dateShort = new Intl.DateTimeFormat("pl-PL", { dateStyle: "short" });

/** Rozmiar w systemie dziesiętnym (1 KB = 1000 B), tak jak liczy Finder i GNOME. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  let i = 0;
  let v = bytes;
  while (v >= 1000 && i < UNITS.length - 1) {
    v /= 1000;
    i++;
  }
  return i === 0 ? `${num0.format(v)} B` : `${num1.format(v)} ${UNITS[i]}`;
}

export function formatCount(n: number): string {
  return num0.format(n);
}

export function formatPct(p: number): string {
  if (p > 0 && p < 0.1) return "<0,1%";
  return `${num1.format(p)}%`;
}

export function formatDate(unixSeconds: number, short = false): string {
  if (!unixSeconds) return "—";
  return (short ? dateShort : dateFmt).format(new Date(unixSeconds * 1000));
}

export function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return m > 0 ? `${m} min ${s % 60} s` : `${(ms / 1000).toFixed(1).replace(".", ",")} s`;
}

/** Odmiana: 1 plik, 2 pliki, 5 plików. */
export function plural(n: number, one: string, few: string, many: string): string {
  const n10 = n % 10;
  const n100 = n % 100;
  if (n === 1) return one;
  if (n10 >= 2 && n10 <= 4 && (n100 < 12 || n100 > 14)) return few;
  return many;
}

/** Skraca ścieżkę ze środka, zostawiając początek i koniec. */
export function middleEllipsis(s: string, max: number): string {
  if (s.length <= max) return s;
  const keep = max - 1;
  const head = Math.ceil(keep * 0.4);
  return `${s.slice(0, head)}…${s.slice(s.length - (keep - head))}`;
}

/** „1 plik”, „3 pliki”, „12 plików”. */
export function filesLabel(n: number): string {
  return `${formatCount(n)} ${plural(n, "plik", "pliki", "plików")}`;
}

/** „1 folder”, „3 foldery”, „12 folderów”. */
export function foldersLabel(n: number): string {
  return `${formatCount(n)} ${plural(n, "folder", "foldery", "folderów")}`;
}
