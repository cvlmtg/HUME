import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveColor, cursorColors, diagnosticStyle } from '../src/lib/theme.js';

test('resolveColor resolves a bare ANSI name to its fixed hex value', () => {
  assert.equal(resolveColor('red', {}), '#cd0000');
  assert.equal(resolveColor('light-gray', {}), '#e5e5e5');
});

test("resolveColor lets a theme's own palette entry override an ANSI name", () => {
  assert.equal(resolveColor('red', { red: '#123456' }), '#123456');
});

test('resolveColor passes an unrecognised name straight through', () => {
  assert.equal(resolveColor('crimson', {}), 'crimson');
});

// ── cursorColors — mirrors hume-engine/src/theme/mod.rs's cursor_ladder ────

test('cursorColors resolves the most specific rung of each chain', () => {
  const sc = {
    'ui.cursor.normal': '#100000',
    'ui.cursor.insert': '#200000',
    'ui.cursor.select': '#300000',
    'ui.cursor.primary.normal': '#400000',
    'ui.cursor.primary.insert': '#500000',
    'ui.cursor.primary.select': '#600000',
  };
  assert.equal(cursorColors('normal', false, sc, {}).fg, '#100000');
  assert.equal(cursorColors('insert', false, sc, {}).fg, '#200000');
  assert.equal(cursorColors('select', false, sc, {}).fg, '#300000');
  assert.equal(cursorColors('normal', true, sc, {}).fg, '#400000');
  assert.equal(cursorColors('insert', true, sc, {}).fg, '#500000');
  assert.equal(cursorColors('select', true, sc, {}).fg, '#600000');
});

test('cursorColors secondary chain falls back through ui.cursor to ui.selection', () => {
  assert.equal(cursorColors('insert', false, { 'ui.cursor': '#abcdef' }, {}).fg, '#abcdef');
  assert.equal(cursorColors('insert', false, { 'ui.selection': '#123456' }, {}).fg, '#123456');
});

test('cursorColors primary chain reaches the bare "ui" rung the secondary chain skips', () => {
  const sc = { ui: '#654321' };
  assert.equal(cursorColors('insert', true, sc, {}).fg, '#654321');
  // The secondary ladder has no "ui" rung at all — it must fall through
  // past it rather than resolving the same way the primary ladder did.
  assert.equal(cursorColors('insert', false, sc, {}).fg, null);
});

test('cursorColors always returns a normalized style, never null', () => {
  assert.deepEqual(cursorColors('insert', true, {}, {}), { fg: null, bg: null, mods: [], underline: null });
  assert.deepEqual(cursorColors('select', false, {}, {}), { fg: null, bg: null, mods: [], underline: null });
});

// ── diagnosticStyle — the diagnostic.<sev> text-span squiggle ──────────────

test('diagnosticStyle resolves the diagnostic.<sev> scope, distinct per severity', () => {
  const sc = {
    'diagnostic.error': { fg: '#ff0000', modifiers: ['underlined'] },
    'diagnostic.warning': '#ffff00',
  };
  const err = diagnosticStyle('error', sc, {});
  assert.equal(err.fg, '#ff0000');
  assert.deepEqual(err.mods, ['underlined']);
  assert.equal(diagnosticStyle('warning', sc, {}).fg, '#ffff00');
});

test('diagnosticStyle returns null for a severity the theme leaves unset', () => {
  assert.equal(diagnosticStyle('hint', {}, {}), null);
});
