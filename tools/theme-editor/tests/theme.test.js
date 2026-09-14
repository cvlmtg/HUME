import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolveColor, cursorColors, diagnosticStyle, fullStyle, tokenStyle, MODIFIERS, UNDERLINE_STYLES } from '../src/lib/theme.js';
import { MODIFIER_NAMES } from '../src/lib/vocabulary.generated.js';

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

// ── vocabulary — generated from the loader, checked here for gaps the
// generated data itself can't rule out ─────────────────────────────────────

// `UNDERLINE_STYLES`' keys come from the generated `UNDERLINE_NAMES`, but its
// CSS values are hand-written (theme.js's `UNDERLINE_STYLE_CSS`) — a loader
// style added with no matching entry there would silently fall back to
// "solid" via `cssUnderlineStyle` instead of failing anywhere.
test('every generated underline style has a CSS mapping', () => {
  const missing = Object.entries(UNDERLINE_STYLES)
    .filter(([, css]) => css === undefined)
    .map(([name]) => name);
  assert.deepEqual(missing, [], `no CSS mapping in theme.js's UNDERLINE_STYLE_CSS: ${missing.join(', ')}`);
});

// `tokenStyle` (theme.js) handles each modifier as its own
// `mods.includes("...")` branch — a fourth hand-copy of the loader's
// modifier vocabulary, nothing else checks. Reads theme.js's own source as
// text (same idiom as coverage.test.js's `PREVIEW_SOURCE`), since the
// vocabulary is data but the branches are code with no shared list to pin
// against directly.
test('every generated modifier name is handled somewhere in theme.js', () => {
  const src = readFileSync(new URL('../src/lib/theme.js', import.meta.url), 'utf8');
  const missing = MODIFIER_NAMES.filter(name => !src.includes(`"${name}"`));
  assert.deepEqual(missing, [], `not referenced in theme.js: ${missing.join(', ')}`);
});

// ── underline resolution — mirrors parse_style_table + ResolvedStyle ───────

// An `underline` table carrying only a colour leaves the style unset in Rust
// (`parse_style_table` sets `underline_color` alone), and `normalized()` in
// hume-grid/src/style.rs then drops the colour because no style was set — so
// HUME draws nothing. gruvbox's `"definition" = { underline = { color = … } }`
// is exactly this shape, and defaulting it to a solid line previewed an
// underline the editor it previews never renders.
test('an underline table with a colour but no style resolves to no underline', () => {
  const sc = { definition: { underline: { color: '#8ec07c' } } };
  assert.equal(fullStyle('definition', sc, {}).underline, null);
});

test('an underline table with an explicit style keeps both fields', () => {
  const sc = { a: { underline: { color: '#ff0000', style: 'curl' } } };
  assert.deepEqual(fullStyle('a', sc, {}).underline, { style: 'curl', color: '#ff0000' });
});

test('a shorthand string underline resolves with no colour of its own', () => {
  assert.deepEqual(fullStyle('a', { a: { underline: 'curl' } }, {}).underline, {
    style: 'curl',
    color: null,
  });
});

// `tokenStyle`'s `tag` is a fully-resolved style layered over the token's own
// scope — the "diag" tag passes `diagnosticStyle(...)`, whose whole point is
// the squiggle. Reading `underline` from the token's scope alone dropped it
// for every token that carries no underline itself, which is nearly all of
// them: the squiggle simply never rendered.
test('tokenStyle takes the underline from an overlaid tag', () => {
  const sc = { variable: '#ebdbb2' };
  const tag = fullStyle('diagnostic.error', {
    'diagnostic.error': { fg: '#fb4934', underline: { color: '#fb4934', style: 'curl' } },
  }, {});
  const css = tokenStyle('variable', sc, {}, '#fff', '#282828', tag);
  assert.equal(css.textDecoration, 'underline');
  assert.equal(css.textDecorationStyle, 'wavy');
  assert.equal(css.textDecorationColor, '#fb4934');
});

// A tag that carries no underline must not erase the token's own.
test("tokenStyle keeps the token's own underline when the tag has none", () => {
  const sc = { a: { fg: '#ffffff', underline: 'dotted' } };
  const css = tokenStyle('a', sc, {}, '#fff', '#282828', { fg: '#ff0000', mods: [] });
  assert.equal(css.textDecorationStyle, 'dotted');
});
