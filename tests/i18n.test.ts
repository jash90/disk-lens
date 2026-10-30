import assert from 'node:assert/strict';
import test from 'node:test';

const module = await import('../src/i18n/core.ts').catch(() => null);

test('locale selection supports regional tags and falls back to English', () => {
  assert.ok(module, 'localization core must exist');
  assert.equal(module.resolveLocale('de-AT'), 'de');
  assert.equal(module.resolveLocale('pl-PL'), 'pl');
  assert.equal(module.resolveLocale('en-US'), 'en');
  assert.equal(module.resolveLocale('fr'), 'en');
});

test('translations interpolate values without changing paths', () => {
  assert.ok(module, 'localization core must exist');
  const catalog = { 'Skopiowano: {path}': { de: 'Kopiert: {path}', en: 'Copied: {path}' } };
  assert.equal(module.translate(catalog, 'de', 'Skopiowano: {path}', { path: 'C:/Plik/{name}.txt' }), 'Kopiert: C:/Plik/{name}.txt');
  assert.equal(module.translate(catalog, 'pl', 'Skopiowano: {path}', { path: '/tmp/a' }), 'Skopiowano: /tmp/a');
});

test('unknown text is preserved', () => {
  assert.ok(module, 'localization core must exist');
  assert.equal(module.translate({}, 'de', '/tmp/Polski.txt'), '/tmp/Polski.txt');
});
