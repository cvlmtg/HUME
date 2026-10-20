import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Theme } from '../src/lib/theme.js';

// A JSON object literal is the one way to write a `__proto__` key as data.
const withProto = (json) => JSON.parse(json);

const entries = (pairs) => Object.fromEntries(pairs);

test('scope and color read only what the theme defines', () => {
  const t = Theme.from({
    palette: withProto('{"__proto__": "#111111"}'),
    scopes: { constructor: 'fg' },
  });
  assert.equal(t.scope('constructor'), 'fg');
  assert.equal(t.color('__proto__'), '#111111');
  assert.equal(t.scope('toString'), undefined);
  assert.equal(t.scope('__proto__'), undefined);
  assert.equal(t.color('constructor'), undefined);
});

test('with/without return a new theme and leave the original unchanged', () => {
  const t = Theme.from({ palette: { fg: '#111111' }, scopes: { keyword: 'fg' } });

  const added = t.withScope('constructor', 'fg').withColor('bg', '#222222');
  assert.notEqual(added, t);
  assert.equal(added.scope('constructor'), 'fg');
  assert.equal(added.color('bg'), '#222222');
  assert.equal(t.scope('constructor'), undefined);
  assert.equal(t.color('bg'), undefined);

  const removed = t.withoutScope('keyword').withoutColor('fg');
  assert.equal(removed.scope('keyword'), undefined);
  assert.equal(removed.color('fg'), undefined);
  assert.equal(t.scope('keyword'), 'fg');
  assert.equal(t.color('fg'), '#111111');
});

test('colors and scopes list entries in insertion order', () => {
  const t = Theme.from({ palette: { b: '#2', a: '#1' }, scopes: { y: 'b', x: 'a' } });
  assert.deepEqual(t.colors(), [['b', '#2'], ['a', '#1']]);
  assert.deepEqual(t.scopes(), [['y', 'b'], ['x', 'a']]);
});

test('overlay lets the more derived theme win at every level', () => {
  const root = Theme.from({ palette: { a: '#1', b: '#1', c: '#1' }, scopes: { s: 'a', t: 'a' } });
  const mid = Theme.from({ palette: { b: '#2', c: '#2' }, scopes: { t: 'b' } });
  const leaf = Theme.from({ palette: { c: '#3' } });
  const merged = root.overlay(mid).overlay(leaf);
  assert.deepEqual(entries(merged.colors()), { a: '#1', b: '#2', c: '#3' });
  assert.deepEqual(entries(merged.scopes()), { s: 'a', t: 'b' });
});

test('changedFrom keeps only the entries that differ from the baseline', () => {
  const baseline = Theme.from({
    palette: { fg: '#111111', bg: '#222222' },
    scopes: { keyword: 'fg', comment: { fg: 'bg', modifiers: ['italic'] } },
  });
  const current = baseline
    .withColor('fg', '#999999')
    .withScope('keyword', 'bg')
    .withScope('constructor', 'fg')
    .withColor('__proto__', '#333333');
  const changed = current.changedFrom(baseline);
  assert.deepEqual(entries(changed.colors()), entries([['fg', '#999999'], ['__proto__', '#333333']]));
  assert.deepEqual(entries(changed.scopes()), { keyword: 'bg', constructor: 'fg' });
});

test('mapColors transforms every palette entry, "__proto__" included', () => {
  const t = Theme.from({ palette: withProto('{"__proto__": "a", "fg": "b"}'), scopes: { keyword: 'fg' } });
  const mapped = t.mapColors((c) => c.toUpperCase());
  assert.deepEqual(mapped.colors(), [['__proto__', 'A'], ['fg', 'B']]);
  assert.deepEqual(mapped.scopes(), [['keyword', 'fg']]);
  assert.deepEqual(t.colors(), [['__proto__', 'a'], ['fg', 'b']]);
});
