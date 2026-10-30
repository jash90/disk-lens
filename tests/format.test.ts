import assert from "node:assert/strict";
import test from "node:test";
import { setLocale, loadLocale, subscribe } from "../src/i18n/locale.ts";
import { filesLabel, foldersLabel, formatBytes, formatCount, formatDuration, formatPct, formatDate } from "../src/lib/format.ts";

test("English formatting and plural labels", () => {
  setLocale("en");
  assert.equal(formatBytes(1500), "1.5 KB");
  assert.equal(formatCount(1234), "1,234");
  assert.equal(formatPct(0.01), "<0.1%");
  assert.equal(formatDuration(1500), "1.5 s");
  assert.equal(filesLabel(1), "1 file");
  assert.equal(filesLabel(22), "22 files");
  assert.equal(foldersLabel(2), "2 folders");
});

test("German formatting and plural labels", () => {
  setLocale("de");
  assert.equal(formatBytes(1500), "1,5 KB");
  assert.equal(formatCount(1234), "1.234");
  assert.equal(filesLabel(1), "1 Datei");
  assert.equal(filesLabel(2), "2 Dateien");
  assert.equal(foldersLabel(2), "2 Ordner");
  const stamp = 1700000000;
  assert.equal(formatDate(stamp), new Intl.DateTimeFormat("de", { dateStyle: "medium", timeStyle: "short" }).format(new Date(stamp * 1000)));
});

test("Polish plural rules remain intact", () => {
  setLocale("pl");
  assert.equal(filesLabel(1), "1 plik");
  assert.equal(filesLabel(2), "2 pliki");
  assert.equal(filesLabel(12), "12 plików");
  assert.equal(filesLabel(22), "22 pliki");
  assert.equal(formatPct(0.01), "<0,1%");
});

test("saved locale overrides system language, denied or invalid storage falls back", () => {
  assert.equal(loadLocale({ getItem: () => "pl" }, "de-DE"), "pl");
  assert.equal(loadLocale({ getItem: () => "bad" }, "de-DE"), "de");
  assert.equal(loadLocale({ getItem: () => { throw Error("denied"); } }, "en-US"), "en");
});

test("language changes notify subscribers, cleanup stops notifications", () => {
  let calls = 0;
  const cleanup = subscribe(() => calls++);
  setLocale("de");
  assert.equal(calls, 1);
  cleanup();
  setLocale("en");
  assert.equal(calls, 1);
});
