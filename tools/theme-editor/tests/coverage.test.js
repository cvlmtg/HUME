import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ALL_SCOPES, DEFAULT_SC } from '../src/data.js';
import {
  RUST_SAMPLE, HTML_SAMPLE, MARKDOWN_SAMPLE, DIFF_SAMPLE, CHROME_SCOPES,
  NEIGHBOR_TOP, NEIGHBOR_BOTTOM,
} from '../src/preview/samples.js';

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
