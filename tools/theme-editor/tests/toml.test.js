import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseTOML, extractScopes, exportTOML, diffFromBaseline, unescapeBasic, parseInlineTable } from '../src/lib/toml.js';
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

test('exportTOML with an inherits name emits it as the first line and round-trips', () => {
  const exported = exportTOML({ bg0: '#eeeeee' }, { 'ui.cursorline': { bg: 'bg1' } }, 'gruvbox');
  assert.match(exported, /^inherits = "gruvbox"\n\n/);
  const reparsed = parseTOML(exported);
  assert.equal(reparsed.inherits, 'gruvbox');
  assert.deepEqual(extractScopes(reparsed), { 'ui.cursorline': { bg: 'bg1' } });
});

test('exportTOML without an inherits name omits the key, as before', () => {
  const exported = exportTOML({}, { k: 'v' });
  assert.doesNotMatch(exported, /^inherits/);
});

test('diffFromBaseline finds only the overridden and newly-added entries', () => {
  const baseline = { keyword: 'red', comment: 'gray' };
  const current = { keyword: 'blue', comment: 'gray', 'markup.link': 'cyan' };
  assert.deepEqual(diffFromBaseline(current, baseline), {
    keyword: 'blue',
    'markup.link': 'cyan',
  });
});

// The end-to-end shape App.jsx's handleExport relies on: a resolved child
// theme's merged (parent + child) state, diffed back against the parent's
// own baseline, exports as `inherits` plus only the child's own overrides —
// round-tripping as the small file it started as, not the merged state the
// editor renders from.
test('a merged inherits-child exports and round-trips as inherits plus only its own overrides', () => {
  const parentPalette = { bg0: '#111111', bg1: '#222222' };
  const parentScopes = { keyword: 'bg0', comment: { fg: 'bg1', modifiers: ['italic'] } };

  // What the app actually holds after merging the child onto the parent: a
  // new `bg0` color, and `keyword` re-pointed at `bg1` instead of `bg0`.
  const mergedPalette = { ...parentPalette, bg0: '#999999' };
  const mergedScopes = { ...parentScopes, keyword: 'bg1' };

  const exportPalette = diffFromBaseline(mergedPalette, parentPalette);
  const exportScopes = diffFromBaseline(mergedScopes, parentScopes);
  const exported = exportTOML(exportPalette, exportScopes, 'base');

  assert.match(exported, /^inherits = "base"\n\n/);
  const reparsed = parseTOML(exported);
  assert.equal(reparsed.inherits, 'base');
  // Only the child's own override survives — the untouched `comment` scope
  // and `bg1` palette entry, both still exactly the parent's, are absent.
  assert.deepEqual(extractScopes(reparsed), { keyword: 'bg1' });
  assert.deepEqual(reparsed.palette, { bg0: '#999999' });
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

// `style` is a style field only inside `underline = { color, style }`, which
// walkScopes keeps verbatim as part of the def. A bare `style` at the top of a
// scope table is not a style field, so it flattens like any other child —
// matching STYLE_KEYS in hume-engine/src/theme/loader.rs.
test('a bare `style` key in a scope table is a child, not a style field', () => {
  const parsed = parseTOML('"a" = { style = { bold = true } }');
  const original = extractScopes(parsed);
  assert.deepEqual(original, { 'a.style.bold': true });

  const reparsed = extractScopes(parseTOML(exportTOML({}, original)));
  assert.deepEqual(reparsed, original);
});

test('`underline = { color, style }` keeps its nested style field', () => {
  const parsed = parseTOML('"a" = { underline = { color = "#ff0000", style = "curl" } }');
  assert.deepEqual(extractScopes(parsed), {
    a: { underline: { color: '#ff0000', style: 'curl' } },
  });
});

// The catalog of style fields has to stay identical to STYLE_KEYS in
// hume-engine/src/theme/loader.rs: a key this list treats as a style field but
// HUME treats as a child scope (or vice versa) makes the same theme flatten
// two different ways in the editor and the editor it previews.
test('the style-field set matches the loader\'s STYLE_KEYS', () => {
  const parsed = parseTOML(
    '"a" = { fg = "#111111", bg = "#222222", underline = "curl", modifiers = ["bold"] }'
  );
  const def = extractScopes(parsed).a;
  assert.deepEqual(Object.keys(def).sort(), ['bg', 'fg', 'modifiers', 'underline']);
});

// A scalar that is neither a string nor a table used to vanish here, so an
// imported theme carrying one lost it on export.
test('a non-string scalar scope value is preserved, not dropped', () => {
  const original = extractScopes(parseTOML('"a" = 42\n"b" = true'));
  assert.deepEqual(original, { a: 42, b: true });

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

// ── Dotted keys inside an inline table ────────────────────────────────────

// TOML nests a dotted key, and so does the `toml` crate the Rust loader parses
// with: `{ underline.style = "line" }` is `underline = { style = "line" }`, not
// a key literally named "underline.style". Kept literal, `walkScopes` doesn't
// recognise it as the `underline` style field and promotes it into an invented
// child scope instead, losing the real scope's underline.
test('a dotted key inside an inline table nests instead of staying literal', () => {
  assert.deepEqual(parseInlineTable('{ underline.style = "line" }'), {
    underline: { style: 'line' },
  });
});

test('a dotted inline key merges with its sibling rather than replacing it', () => {
  assert.deepEqual(parseInlineTable('{ underline.color = "#ff0000", underline.style = "curl" }'), {
    underline: { color: '#ff0000', style: 'curl' },
  });
});

test('a quoted inline key containing "." stays one segment', () => {
  assert.deepEqual(parseInlineTable('{ "a.b" = 1 }'), { 'a.b': 1 });
});

// Reads the shipped theme rather than a fixture: `ui.picker.header.column`'s
// `{ underline.style = "line" }` is the exact shape that flattened into an
// invented `…column.underline.style` scope, dropping the real one entirely —
// and re-exported the invented name over it.
test('a shipped theme with a dotted inline key imports with its scope intact', () => {
  const src = readFileSync(
    new URL('../../../runtime/themes/gruvbox.toml', import.meta.url), 'utf8');
  const scopes = extractScopes(parseTOML(src));
  assert.deepEqual(scopes['ui.picker.header.column'], { underline: { style: 'line' } });
  assert.ok(!('ui.picker.header.column.underline.style' in scopes));
});

// ── Scalar siblings of a style field ──────────────────────────────────────

// Matches `walk_scope` in hume-engine/src/theme/loader.rs: once a table is
// known to be a style, a scalar sibling is indistinguishable from a misspelled
// attribute, so HUME warns and drops it rather than inventing a child scope
// nothing resolves.
test('a scalar beside a style field is dropped, not promoted to a child scope', () => {
  const parsed = parseTOML('[ui]\nfg = "white"\ntext = "#ffffff"');
  assert.deepEqual(extractScopes(parsed), { ui: { fg: 'white' } });
});

// The other half of the same rule: with no style field present the table is a
// pure container, so its scalar entries are real child scopes.
test('a scalar in a table with no style field is still a child scope', () => {
  const parsed = parseTOML('[ui]\ntext = "#ffffff"');
  assert.deepEqual(extractScopes(parsed), { 'ui.text': '#ffffff' });
});

// A sub-table beside a style field stays a child — there's nothing else it
// could be, so only scalars are dropped.
test('a sub-table beside a style field is still a child scope', () => {
  const parsed = { ui: { fg: 'white', cursor: { bg: 'blue' } } };
  assert.deepEqual(extractScopes(parsed), {
    ui: { fg: 'white' },
    'ui.cursor': { bg: 'blue' },
  });
});
