const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];
import { getLocale } from "../i18n/locale.ts";

let formattedLocale = "";
let num1: Intl.NumberFormat;
let num0: Intl.NumberFormat;
let dateFmt: Intl.DateTimeFormat;
let dateShort: Intl.DateTimeFormat;
function updateFormats() {
  const locale = getLocale();
  if (locale === formattedLocale) return;
  formattedLocale = locale;
  num1 = new Intl.NumberFormat(locale, { maximumFractionDigits: 1, minimumFractionDigits: 1 });
  num0 = new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
  dateFmt = new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" });
  dateShort = new Intl.DateTimeFormat(locale, { dateStyle: "short" });
}

/** Rozmiar w systemie dziesiętnym (1 KB = 1000 B), tak jak liczy Finder i GNOME. */
export function formatBytes(bytes: number): string {
  updateFormats();
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
  updateFormats();
  return num0.format(n);
}

export function formatPct(p: number): string {
  updateFormats();
  if (p > 0 && p < 0.1) return `<${num1.format(0.1)}%`;
  return `${num1.format(p)}%`;
}

export function formatDate(unixSeconds: number, short = false): string {
  updateFormats();
  if (!unixSeconds) return "—";
  return (short ? dateShort : dateFmt).format(new Date(unixSeconds * 1000));
}

export function formatDuration(ms: number): string {
  updateFormats();
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return m > 0 ? `${m} min ${s % 60} s` : `${num1.format(ms / 1000)} s`;
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
  const locale = getLocale();
  const label = locale === "de" ? (n === 1 ? "Datei" : "Dateien") : locale === "en" ? (n === 1 ? "file" : "files") : plural(n, "plik", "pliki", "plików");
  return `${formatCount(n)} ${label}`;
}

/** „1 folder”, „3 foldery”, „12 folderów”. */
export function foldersLabel(n: number): string {
  const locale = getLocale();
  const label = locale === "de" ? "Ordner" : locale === "en" ? (n === 1 ? "folder" : "folders") : plural(n, "folder", "foldery", "folderów");
  return `${formatCount(n)} ${label}`;
}
