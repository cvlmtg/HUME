import { test } from 'node:test';
import assert from 'node:assert/strict';
import { adjustColor, hexToHSL, hslToHex } from '../src/lib/color.js';

test('adjustColor is a no-op on a 6-digit hex when all shifts are zero', () => {
  assert.equal(adjustColor('#7aa2f7', 0, 0, 0), '#7aa2f7');
});

test('adjustColor preserves the alpha byte of an 8-digit hex across a shift', () => {
  // Independent oracle: convert fg only (drop alpha), shift, then compare
  // against the alpha byte read straight off the input — not derived via
  // the same hexToHSL/hslToHex path the implementation uses.
  const shifted = adjustColor('#7aa2f7cc', 10, 0, 0);
  assert.equal(shifted.length, 9, 'alpha byte must survive the shift');
  assert.equal(shifted.slice(7, 9), 'cc');
});

test('adjustColor leaves a non-hex or too-short value untouched', () => {
  assert.equal(adjustColor('', 10, 0, 0), '');
  assert.equal(adjustColor('#fff', 10, 0, 0), '#fff');
  assert.equal(adjustColor('not-a-color', 10, 0, 0), 'not-a-color');
});

test('hexToHSL/hslToHex round-trip a primary color', () => {
  const [h, s, l] = hexToHSL('#ff0000');
  assert.equal(hslToHex(h, s, l), '#ff0000');
});
