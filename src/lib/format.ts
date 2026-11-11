const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];
const num1 = new Intl.NumberFormat("pl-PL", { maximumFractionDigits: 1, minimumFractionDigits: 1 });
const num0 = new Intl.NumberFormat("pl-PL", { maximumFractionDigits: 0 });
const dateFmt = new Intl.DateTimeFormat("pl-PL", { dateStyle: "medium", timeStyle: "short" });
const dateShort = new Intl.DateTimeFormat("pl-PL", { dateStyle: "short" });

/** Size in decimal units (1 KB = 1000 B), as Finder and GNOME count. */
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

/** Size change with an explicit sign: "+1,2 MB", "−300 B". */
export function formatSignedBytes(delta: number): string {
  return `${delta >= 0 ? "+" : "−"}${formatBytes(Math.abs(delta))}`;
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

/** Rough age for lists: "45 dni", "8 mies.", "3,5 roku". */
export function formatAge(seconds: number): string {
  const days = Math.max(0, Math.floor(seconds / 86_400));
  if (days < 60) return `${num0.format(days)} ${plural(days, "dzień", "dni", "dni")}`;
  if (days < 365) return `${num0.format(Math.floor(days / 30))} mies.`;
  const years = Math.floor((days / 365) * 2) / 2;
  if (!Number.isInteger(years)) return `${num1.format(years)} roku`;
  return `${num0.format(years)} ${plural(years, "rok", "lata", "lat")}`;
}

export function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return m > 0 ? `${m} min ${s % 60} s` : `${(ms / 1000).toFixed(1).replace(".", ",")} s`;
}

/** Polish plural forms: 1 plik, 2 pliki, 5 plików. */
function plural(n: number, one: string, few: string, many: string): string {
  const n10 = n % 10;
  const n100 = n % 100;
  if (n === 1) return one;
  if (n10 >= 2 && n10 <= 4 && (n100 < 12 || n100 > 14)) return few;
  return many;
}

/** Shortens a path in the middle, keeping its start and end. */
export function middleEllipsis(s: string, max: number): string {
  if (s.length <= max) return s;
  const keep = max - 1;
  const head = Math.ceil(keep * 0.4);
  return `${s.slice(0, head)}…${s.slice(s.length - (keep - head))}`;
}

/** The noun alone, for when the number is rendered separately: "plik", "pliki", "plików". */
export function filesNoun(n: number): string {
  return plural(n, "plik", "pliki", "plików");
}

/** "1 plik", "3 pliki", "12 plików". */
export function filesLabel(n: number): string {
  return `${formatCount(n)} ${filesNoun(n)}`;
}

/** "1 folder", "3 foldery", "12 folderów". */
export function foldersLabel(n: number): string {
  return `${formatCount(n)} ${plural(n, "folder", "foldery", "folderów")}`;
}

/** "1 element", "3 elementy", "12 elementów". */
export function itemsLabel(n: number): string {
  return `${formatCount(n)} ${plural(n, "element", "elementy", "elementów")}`;
}
