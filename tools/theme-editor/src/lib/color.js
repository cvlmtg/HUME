export function hexToHSL(hex) {
  const r = parseInt(hex.slice(1, 3), 16) / 255;
  const g = parseInt(hex.slice(3, 5), 16) / 255;
  const b = parseInt(hex.slice(5, 7), 16) / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0;
  const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
    else if (max === g) h = ((b - r) / d + 2) / 6;
    else h = ((r - g) / d + 4) / 6;
  }
  return [h * 360, s * 100, l * 100];
}

export function hslToHex(h, s, l) {
  h = ((h % 360) + 360) % 360;
  s = Math.max(0, Math.min(100, s)) / 100;
  l = Math.max(0, Math.min(100, l)) / 100;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs((h / 60) % 2 - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  if (h < 60) { r = c; g = x; }
  else if (h < 120) { r = x; g = c; }
  else if (h < 180) { g = c; b = x; }
  else if (h < 240) { g = x; b = c; }
  else if (h < 300) { r = x; b = c; }
  else { r = c; b = x; }
  const toH = v => { const hx = Math.round((v + m) * 255).toString(16); return hx.length === 1 ? "0" + hx : hx; };
  return "#" + toH(r) + toH(g) + toH(b);
}

export function adjustColor(hex, hShift, sShift, lShift) {
  if (!hex || !hex.startsWith("#") || hex.length < 7) return hex;
  const hsl = hexToHSL(hex);
  return hslToHex(hsl[0] + hShift, hsl[1] + sShift, hsl[2] + lShift);
}

export function adjustPalette(pal, hShift, sShift, lShift) {
  if (hShift === 0 && sShift === 0 && lShift === 0) return pal;
  const out = {};
  for (const k of Object.keys(pal)) out[k] = adjustColor(pal[k], hShift, sShift, lShift);
  return out;
}
