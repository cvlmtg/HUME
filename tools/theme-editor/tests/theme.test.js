import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveColor } from '../src/lib/theme.js';

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
