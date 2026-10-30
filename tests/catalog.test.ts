import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync, readdirSync } from "node:fs";
import ts from "typescript";
import { general } from "../src/i18n/general.ts";
import { controls } from "../src/i18n/controls.ts";
import { panels } from "../src/i18n/panels.ts";

const catalog = { ...general, ...controls, ...panels };
const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();

test("every translation is present and preserves its placeholders", () => {
  for (const [key, translations] of Object.entries(catalog)) {
    for (const locale of ["de", "en"] as const) {
      assert.ok(translations[locale].trim(), `${locale}: ${key}`);
      assert.deepEqual(placeholders(translations[locale]), placeholders(key), `${locale}: ${key}`);
    }
  }
});

test("literal translation calls have entries in both languages", () => {
  const paths = ["src/App.tsx", "src/lib/actions.ts", ...readdirSync("src/components").filter((name) => name.endsWith(".tsx")).map((name) => `src/components/${name}`)];
  for (const path of paths) {
    const source = ts.createSourceFile(path, readFileSync(path, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    function visit(node: ts.Node) {
      if (ts.isCallExpression(node) && node.expression.getText(source) === "t" && node.arguments[0] && ts.isStringLiteral(node.arguments[0])) {
        assert.ok(Object.hasOwn(catalog, node.arguments[0].text), `${path}: ${node.arguments[0].text}`);
      }
      if (ts.isJsxText(node)) {
        const text = node.text.trim();
        assert.ok(!/[a-ząćęłńóśźż]{3}/i.test(text) || ["Polski", "Deutsch", "English"].includes(text), `${path}: untranslated JSX text: ${text}`);
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
  }
});
