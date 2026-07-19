import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseTOML, extractScopes, exportTOML, unescapeBasic } from '../src/lib/toml.js';
import { bgc, lookupRaw } from '../src/lib/theme.js';

test('unescapeBasic handles \\", \\\\, \\n, \\t, \\r', () => {
  assert.equal(unescapeBasic('a\\"b'), 'a"b');
  assert.equal(unescapeBasic('a\\\\b'), 'a\\b');
  assert.equal(unescapeBasic('a\\nb'), 'a\nb');
  assert.equal(unescapeBasic('a\\tb'), 'a\tb');
  assert.equal(unescapeBasic('a\\rb'), 'a\rb');
});

test('parseTOML unescapes double-quoted strings, keeps single-quoted literal', () => {
  const parsed = parseTOML('k = "a\\"b"\nlit = \'raw\\nstays\'');
  assert.equal(parsed.k, 'a"b');
  assert.equal(parsed.lit, 'raw\\nstays');
});

test('escaped-quote string round-trips through export byte-for-byte', () => {
  const parsed = parseTOML('k = "a\\"b\\ntab\\tend"');
  const exported = exportTOML({}, { k: parsed.k });
  assert.equal(exported, '"k" = "a\\"b\\ntab\\tend"\n\n[palette]\n');
});

test('parseTOML parses numbers and booleans as their real types', () => {
  const parsed = parseTOML('x = 42\ny = true\nz = 3.14\nw = -7\nq = false');
  assert.equal(parsed.x, 42);
  assert.equal(parsed.y, true);
  assert.equal(parsed.z, 3.14);
  assert.equal(parsed.w, -7);
  assert.equal(parsed.q, false);
});

test('number/boolean values round-trip through export unquoted', () => {
  const exported = exportTOML({}, { x: 42, y: true });
  assert.match(exported, /^"x" = 42\n"y" = true\n/);
});

test('a string that merely looks like a hex color still parses as a string', () => {
  const parsed = parseTOML('k = "#aabbcc"');
  assert.equal(parsed.k, '#aabbcc');
  assert.equal(typeof parsed.k, 'string');
});

test('extractScopes preserves an empty scope def ({}), e.g. ui.cursor.insert', () => {
  const parsed = parseTOML('"ui.cursor.insert" = {}');
  assert.deepEqual(extractScopes(parsed), { 'ui.cursor.insert': {} });
});

test('extractScopes flattens a depth-3 section header into one dotted scope', () => {
  const parsed = parseTOML(
    '[ui.statusline.normal]\nfg = "black"\nbg = "blue"\n[palette]\nblue = "#7aa2f7"'
  );
  assert.deepEqual(extractScopes(parsed), {
    'ui.statusline.normal': { fg: 'black', bg: 'blue' },
  });
});

test('extractScopes flattens a mixed def-and-children table', () => {
  // [ui] with both a direct style key and a nested sub-table.
  const parsed = { ui: { fg: 'white', cursor: { fg: 'blue' } } };
  assert.deepEqual(extractScopes(parsed), {
    ui: { fg: 'white' },
    'ui.cursor': { fg: 'blue' },
  });
});

test('extractScopes skips palette and inherits at the top level only', () => {
  const parsed = parseTOML(
    'inherits = "base16_default_dark"\ncomment = "gray"\n[palette]\nblack = "#000000"'
  );
  assert.deepEqual(extractScopes(parsed), { comment: 'gray' });
});

test('bgc returns "transparent" for a bare fg-only string scope value', () => {
  const sc = { 'ui.background': 'black' };
  const pal = { black: '#1a1b26' };
  assert.equal(bgc('ui.background', sc, pal, '#fallback'), 'transparent');
});

test('an empty scope def ({}) blocks dotted-chain fallback (intentional Helix semantics)', () => {
  // ui.cursorline defines a real bg; ui.cursorline.primary is explicitly {} —
  // that should NOT fall back to the parent's bg, it should resolve to nothing.
  const sc = { 'ui.cursorline': { bg: 'red' }, 'ui.cursorline.primary': {} };
  assert.deepEqual(lookupRaw('ui.cursorline.primary', sc), {});
});

test('lookupRaw falls back through the dotted chain when no exact key is defined', () => {
  const sc = { 'ui.cursorline': { bg: 'red' } };
  assert.deepEqual(lookupRaw('ui.cursorline.primary', sc), { bg: 'red' });
});
