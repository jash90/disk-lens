/** Nazwy kategorii – zgodne z `category::NAMES` w Rust. */
export const CATEGORY_NAMES = [
  "Inne",
  "Wideo",
  "Obrazy",
  "Audio",
  "Archiwa i obrazy dysków",
  "Dokumenty",
  "Kod i dane",
  "Aplikacje i biblioteki",
  "Foldery",
];

/** Kolory kategorii (te same w obu motywach – są nasycone, czytelne na jasnym i ciemnym tle). */
export const CATEGORY_COLORS = [
  "#8a94a6", // inne
  "#e5484d", // wideo
  "#30a46c", // obrazy
  "#f76b15", // audio
  "#8e4ec6", // archiwa
  "#0090ff", // dokumenty
  "#12a594", // kod
  "#d6409f", // aplikacje
  "#6e7a8a", // foldery
];

export const AGE_NAMES = ["< tydzień", "< miesiąc", "< pół roku", "< rok", "< 3 lata", "starsze"];
/** Od najświeższego (ciepły) do najstarszego (chłodny). */
export const AGE_COLORS = ["#e5484d", "#f76b15", "#f5d90a", "#46a758", "#0090ff", "#5b5bd6"];

export type ColorMode = "type" | "age";

export function tileColor(mode: ColorMode, cat: number, age: number): string {
  return mode === "type" ? (CATEGORY_COLORS[cat] ?? CATEGORY_COLORS[0]) : (AGE_COLORS[age] ?? AGE_COLORS[5]);
}

/** Ikona tekstowa dla rodzaju węzła. */
export function kindIcon(kind: string, cat: number): string {
  if (kind === "dir") return "📁";
  if (kind === "package") return cat === 7 ? "📦" : "🗂️";
  if (kind === "symlink") return "↪︎";
  return ["📄", "🎞️", "🖼️", "🎵", "🗜️", "📑", "🧾", "⚙️", "📁"][cat] ?? "📄";
}
