import { resolveLocale, type Locale } from "./core.ts";

const STORAGE_KEY = "disk-lens-locale";
const listeners = new Set<() => void>();

export function loadLocale(storage: Pick<Storage, "getItem"> | undefined, language: string): Locale {
  try {
    const saved = storage?.getItem(STORAGE_KEY);
    if (saved === "pl" || saved === "de" || saved === "en") return saved;
  } catch { /* Storage may be unavailable in a restricted webview. */ }
  return resolveLocale(language);
}

function initialLocale(): Locale {
  try { return loadLocale(globalThis.localStorage, navigator.language); }
  catch { return resolveLocale(navigator.language); }
}

let locale = initialLocale();
if (typeof document !== "undefined") document.documentElement.lang = locale;

export function getLocale(): Locale { return locale; }

export function setLocale(next: Locale): void {
  if (next !== "pl" && next !== "de" && next !== "en") return;
  locale = next;
  if (typeof document !== "undefined") document.documentElement.lang = next;
  try { localStorage.setItem(STORAGE_KEY, next); } catch { /* Keep the in-memory selection. */ }
  listeners.forEach((listener) => listener());
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
