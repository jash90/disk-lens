import { useSyncExternalStore } from "react";
import { controls } from "./controls";
import { panels } from "./panels";
import { general } from "./general";
import { translate, type Locale, type Values } from "./core";

import { getLocale, setLocale, subscribe } from "./locale";
export { getLocale, setLocale };

export type { Locale } from "./core";
export const catalog = { ...general, ...controls, ...panels };
export function useLocale(): Locale {
  return useSyncExternalStore(subscribe, getLocale);
}

export function t(source: string, values?: Values): string {
  return translate(catalog, getLocale(), source, values);
}
