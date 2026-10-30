import { getLocale } from "./locale.ts";
import { translateError } from "./errors.ts";

/** Only generated root/drive labels; mounted volume names stay opaque. */
export function volumeLabel(name: string, path: string): string {
  const rootLabel = path === "/" && ["Dysk systemowy (/)", "System plików (/)", "Dysk systemowy"].includes(name);
  const drive = /^([A-Za-z]:)[\\/]?$/.exec(path);
  if (rootLabel || (drive && name === `Dysk ${drive[1]}`)) return translateError(name, getLocale());
  return name;
}
