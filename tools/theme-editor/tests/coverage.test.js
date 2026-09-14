import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ALL_SCOPES, DEFAULT_SC } from '../src/data.js';
import {
  RUST_SAMPLE, HTML_SAMPLE, MARKDOWN_SAMPLE, DIFF_SAMPLE, CHROME_SCOPES,
  NEIGHBOR_TOP, NEIGHBOR_BOTTOM,
} from '../src/preview/samples.js';

// The preview components' own source. `CHROME_SCOPES` is derived from the
// catalog, so it can't be evidence that a chrome scope is actually drawn —
// only the components that resolve it can. Read as text rather than executed:
// these are JSX, which `node --test` can't import without a build step.
const PREVIEW_SOURCE = [
  '../src/preview/EditorPane.jsx',
  '../src/preview/MessagesPane.jsx',
  '../src/preview/samples.js',
  '../src/lib/theme.js',
].map(f => readFileSync(new URL(f, import.meta.url), 'utf8')).join('\n');

// Reached only as a dot-fallback parent: the preview resolves
// `ui.cursorline.primary`, and `lookupRaw` walks to `ui.cursorline` when that
// is unset — so editing it does change the preview, just never by being
// looked up under its own name. Same rule `UiScopes::cursorline` follows in
// hume-engine.
const FALLBACK_ONLY_CHROME = ['ui.cursorline'];

// Editable and exportable, but the mock preview draws no tab bar — a real
// gap, not a design choice; extend the preview if that changes.
const NOT_YET_PREVIEWED_CHROME = ['ui.tabline', 'ui.tabline.active', 'ui.bufferline', 'ui.bufferline.active'];

// Families the preview builds at runtime instead of naming in full. Each is
// paired with the source fragment that generates it, so the check still fails
// if that call site goes away — it just can't match on the whole name.
const COMPOSED = [
  [/^ui\.(background|text)$/, 'baseBg(sc, pal)'],
  [/^ui\.cursor\.(primary\.)?(normal|insert|select)$/, 'cursorColors('],
  [/^diagnostic\.(error|warning|info|hint)$/, 'diagnosticStyle('],
  [/^diagnostic\.(error|warning|info|hint)\.message(-text)?$/, '"diagnostic." + e.sev'],
  [/^(error|warning|info|hint)\.diagnostic\.inline$/, '".diagnostic.inline"'],
];

function isResolvedInPreview(id) {
  if (PREVIEW_SOURCE.includes(`"${id}"`)) return true;
  const composed = COMPOSED.find(([pattern]) => pattern.test(id));
  return composed !== undefined && PREVIEW_SOURCE.includes(composed[1]);
}

// Every scope a token/row/sign references, across every sample buffer.
function referencedScopes() {
  const scopes = new Set(CHROME_SCOPES);
  for (const buf of [RUST_SAMPLE, HTML_SAMPLE, MARKDOWN_SAMPLE]) {
    for (const line of buf.lines) {
      for (const [, scope] of line.t) if (scope) scopes.add(scope);
    }
  }
  for (const row of DIFF_SAMPLE.rows) {
    if (row.rowScope) scopes.add(row.rowScope);
    if (row.sign) scopes.add(row.sign.scope);
    for (const [, scope] of row.t) if (scope) scopes.add(scope);
  }
  for (const lines of [NEIGHBOR_TOP, NEIGHBOR_BOTTOM]) {
    for (const line of lines) {
      for (const [, scope] of line.t) if (scope) scopes.add(scope);
    }
  }
  return scopes;
}

test('every catalog scope is referenced by some preview sample', () => {
  const referenced = referencedScopes();
  const missing = ALL_SCOPES.filter(id => !referenced.has(id));
  assert.deepEqual(missing, [], `editable but never previewed: ${missing.join(', ')}`);
});

// Chrome scopes reach `referencedScopes` via `CHROME_SCOPES`, which is derived
// from the catalog — so the check above passes for them by construction and
// says nothing. This is the real one: each must be named by the source of a
// component that resolves it, or it is catalogued and editable while nothing
// in the preview ever draws it.
test('every chrome scope is resolved somewhere in the preview components', () => {
  const undrawn = CHROME_SCOPES.filter(
    id => !isResolvedInPreview(id) && !FALLBACK_ONLY_CHROME.includes(id) && !NOT_YET_PREVIEWED_CHROME.includes(id)
  );
  assert.deepEqual(undrawn, [], `chrome scope in the catalog but never resolved: ${undrawn.join(', ')}`);
});

test('every scope a preview sample references is in the catalog', () => {
  const catalog = new Set(ALL_SCOPES);
  const extra = [...referencedScopes()].filter(id => !catalog.has(id));
  assert.deepEqual(extra, [], `previewed but not editable: ${extra.join(', ')}`);
});

// Scopes deliberately left blank in DEFAULT_SC: undefined here is what makes
// HUME's default `cursor-shape-insert = bar` show the real terminal cursor
// through instead of a themed block (see `cursorColors` in lib/theme.js and
// its comment in data.js).
const DEFAULT_SC_INTENTIONAL_GAPS = ['ui.cursor.insert', 'ui.cursor.primary.insert'];

test('every catalog scope has a DEFAULT_SC entry, except the documented cursor gaps', () => {
  const missing = ALL_SCOPES.filter(
    id => !(id in DEFAULT_SC) && !DEFAULT_SC_INTENTIONAL_GAPS.includes(id)
  );
  assert.deepEqual(missing, [], `editable but blank on a fresh theme: ${missing.join(', ')}`);
});
