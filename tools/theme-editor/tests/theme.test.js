import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolveColor, cursorColors, cursorLadderIds, diagnosticStyle, fullStyle, tokenStyle, MODIFIERS, UNDERLINE_STYLES } from '../src/lib/theme.js';

const LOADER_RS = new URL('../../../hume-engine/src/theme/loader.rs', import.meta.url);

// The string literals matched by one `fn <name>`'s `match` arms — the shape
// `parse_modifier`/`parse_underline` both use to define their vocabulary.
function matchArmNames(src, fnName) {
  const start = src.indexOf(`fn ${fnName}(`);
  assert.notEqual(start, -1, `${fnName} must still exist in hume-engine/src/theme/loader.rs`);
  const body = src.slice(src.indexOf('{', start), src.indexOf('\n}', start));
  return [...body.matchAll(/"([a-z_]+)"\s*=>/g)].map(m => m[1]);
}

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

// The rung lists are a cross-language copy of `cursor_ladder_ids`, which
// hume-engine exposes as a shared function precisely so the order and the
// primary-only bare `ui` rung exist once. Nothing else pins the JS copy, so
// this reads the Rust literals directly: a reordered or extended ladder there
// fails here instead of silently leaving the live preview wrong.
test('the cursor ladders match cursor_ladder_ids in hume-engine', () => {
  const src = readFileSync(
    new URL('../../../hume-engine/src/theme/mod.rs', import.meta.url), 'utf8');
  const start = src.indexOf('pub fn cursor_ladder_ids');
  assert.notEqual(start, -1, 'cursor_ladder_ids must still exist in hume-engine/src/theme/mod.rs');
  const bodyStart = src.indexOf('{', start);
  const body = src.slice(bodyStart, src.indexOf('\n}', bodyStart));
  const rungs = [...body.matchAll(/\[([^\]]*)\]/g)].map(m =>
    m[1].split(',').map(t => t.trim()).filter(Boolean).map(t => t.replace(/^"|"$/g, '')));
  assert.equal(rungs.length, 2, `expected the secondary and primary rung arrays, got ${rungs.length}`);

  // The two array literals name their mode rung by parameter; every other
  // rung is a literal shared by all three chains.
  const chain = 'insert';
  const expand = t =>
    t === 'mode_scope' ? `ui.cursor.${chain}`
      : t === 'primary_mode_scope' ? `ui.cursor.primary.${chain}`
        : t;
  assert.deepEqual(cursorLadderIds(chain, false), rungs[0].map(expand));
  assert.deepEqual(cursorLadderIds(chain, true), rungs[1].map(expand));
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

// ── vocabulary — pinned against the loader's own match arms ───────────────

// Offering a name the loader rejects lets the editor author a theme that
// fails to load; missing one it accepts hides a style the user can't reach.
// Both lists used to be hand-copied into ScopeRow with nothing checking them.
test('the modifier pills match the loader\'s parse_modifier vocabulary', () => {
  const src = readFileSync(LOADER_RS, 'utf8');
  // "underlined" is offered by the editor but handled in `parse_style_table`'s
  // modifiers loop, which routes it to the underline field instead of the
  // bitset — so it is deliberately absent from `parse_modifier` itself.
  assert.ok(MODIFIERS.includes('underlined'));
  assert.deepEqual(
    MODIFIERS.filter(m => m !== 'underlined').sort(),
    matchArmNames(src, 'parse_modifier').sort(),
  );
});

test('the underline styles match the loader\'s parse_underline vocabulary', () => {
  const src = readFileSync(LOADER_RS, 'utf8');
  assert.deepEqual(
    Object.keys(UNDERLINE_STYLES).sort(),
    matchArmNames(src, 'parse_underline').sort(),
  );
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
