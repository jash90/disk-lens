import assert from "node:assert/strict";
import test from "node:test";
import { setLocale } from "../src/i18n/locale.ts";
const module = await import("../src/i18n/volume.ts").catch(() => null);

test("generated root and drive labels are localized", () => {
  assert.ok(module);
  setLocale("de");
  assert.equal(module.volumeLabel("Dysk C:", "C:\\"), "Laufwerk C:");
  assert.equal(module.volumeLabel("System plików (/)", "/"), "Dateisystem (/)");
});

test("mounted volume names and paths remain unchanged", () => {
  assert.ok(module);
  setLocale("en");
  assert.equal(module.volumeLabel("Dysk C:", "/Volumes/Dysk C:"), "Dysk C:");
  assert.equal(module.volumeLabel("My Disk", "/Volumes/My Disk"), "My Disk");
});
