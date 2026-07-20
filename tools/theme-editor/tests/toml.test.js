import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseTOML, extractScopes, exportTOML, unescapeBasic, parseInlineTable } from '../src/lib/toml.js';
import { bgc, lookupRaw } from '../src/lib/theme.js';

test('unescapeBasic handles \\", \\\\, \\n, \\t, \\r', () => {
  assert.equal(unescapeBasic('a\\"b'), 'a"b');
  assert.equal(unescapeBasic('a\\\\b'), 'a\\b');
  assert.equal(unescapeBasic('a\\nb'), 'a\nb');
  assert.equal(unescapeBasic('a\\tb'), 'a\tb');
  assert.equal(unescapeBasic('a\\rb'), 'a\rb');
});

test('unescapeBasic handles \\uXXXX (4-digit) and \\UXXXXXXXX (8-digit) unicode escapes', () => {
  assert.equal(unescapeBasic('\\u00e9'), 'é'); // é, within the BMP
  assert.equal(unescapeBasic('\\U0001F600'), '\u{1F600}'); // 😀, needs a surrogate pair
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

test('parseInlineTable splits on the "=" outside a quoted key containing one', () => {
  const parsed = parseInlineTable('{ "a=b" = "c", x = 1 }');
  assert.deepEqual(parsed, { 'a=b': 'c', x: 1 });
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

// ── Coverage gaps closed: modifiers/array, nested inline object, malformed
// input, quoted keys containing '#' or '.' ──────────────────────────────────

test('a scope def with a modifiers array round-trips through export and re-parse', () => {
  const parsed = parseTOML('"a" = { fg = "red", modifiers = ["bold", "italic"] }');
  const original = extractScopes(parsed);
  assert.deepEqual(original, { a: { fg: 'red', modifiers: ['bold', 'italic'] } });

  // Independent oracle: re-parse the exported text and compare against the
  // pre-export structure, not against a string the export step produced.
  const reparsed = extractScopes(parseTOML(exportTOML({}, original)));
  assert.deepEqual(reparsed, original);
});

test('an inline table with a nested object value (e.g. `style = {...}`) parses and round-trips', () => {
  const parsed = parseTOML('"a" = { style = { bold = true } }');
  const original = extractScopes(parsed);
  assert.deepEqual(original, { a: { style: { bold: true } } });

  const reparsed = extractScopes(parseTOML(exportTOML({}, original)));
  assert.deepEqual(reparsed, original);
});

test('parseTOML skips a malformed line with no "=" instead of throwing', () => {
  const parsed = parseTOML('not a valid toml line\nk = "v"');
  assert.deepEqual(parsed, { k: 'v' });
});

test('parseTOML does not throw on an unterminated string value', () => {
  // No closing quote — parseInlineVal doesn't match the quoted-string
  // pattern (start AND end with '"'), so the value falls through unchanged
  // rather than crashing the parser.
  const parsed = parseTOML('k = "unterminated');
  assert.equal(parsed.k, '"unterminated');
});

test('a quoted key containing "#" is not treated as a comment start', () => {
  const parsed = parseTOML('"a#b" = 1');
  assert.equal(parsed['a#b'], 1);
});

test('a quoted key containing "." is kept as one flat key, not split into a nested path', () => {
  const parsed = parseTOML('"a.b" = 1');
  assert.equal(parsed['a.b'], 1);
  assert.equal(parsed.a, undefined);
});
