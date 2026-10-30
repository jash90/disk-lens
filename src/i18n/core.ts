export type Locale = "pl" | "de" | "en";
export type Catalog = Record<string, { de: string; en: string }>;
export type Values = Record<string, string | number>;

export function resolveLocale(value: string): Locale {
  const language = value.toLowerCase().split(/[-_]/)[0];
  return language === "pl" || language === "de" ? language : "en";
}

export function translate(catalog: Catalog, locale: Locale, source: string, values: Values = {}): string {
  const text = locale === "pl" ? source : catalog[source]?.[locale] ?? source;
  return text.replace(/\{(\w+)\}/g, (placeholder, key: string) => String(values[key] ?? placeholder));
}
