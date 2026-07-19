export function resolveColor(c, pal) {
  if (!c || typeof c !== "string") return null;
  return c.startsWith("#") ? c : (pal[c] || c);
}

export function resolve(val, pal) {
  if (!val) return null;
  if (typeof val === "object") return { fg: resolveColor(val.fg, pal), bg: resolveColor(val.bg, pal) };
  return resolveColor(val, pal);
}

export function lookupRaw(id, sc) {
  let key = id;
  while (key) {
    if (sc[key] !== undefined && sc[key] !== "") return sc[key];
    const dot = key.lastIndexOf(".");
    if (dot === -1) break;
    key = key.slice(0, dot);
  }
  return null;
}

export function scopeLookup(id, sc, pal) {
  const v = lookupRaw(id, sc);
  return v == null ? null : resolve(v, pal);
}

export function fgc(id, sc, pal, fb) {
  const c = scopeLookup(id, sc, pal);
  if (!c) return fb;
  return typeof c === "object" ? (c.fg || fb) : c;
}

export function bgc(id, sc, pal, fb) {
  const c = scopeLookup(id, sc, pal);
  if (!c) return fb;
  return typeof c === "object" ? (c.bg || fb) : "transparent";
}

// Returns a fully normalised style for a scope (including modifiers and underline),
// walking the dotted fallback chain. Returns null when no scope matches.
export function fullStyle(id, sc, pal) {
  const v = lookupRaw(id, sc);
  if (v == null) return null;
  if (typeof v === "string") return { fg: resolveColor(v, pal), bg: null, mods: [], underline: null };
  const u = v.underline;
  return {
    fg: resolveColor(v.fg, pal),
    bg: resolveColor(v.bg, pal),
    mods: Array.isArray(v.modifiers) ? v.modifiers : [],
    underline: u
      ? (typeof u === "string"
          ? { style: u, color: null }
          : { style: u.style || "line", color: resolveColor(u.color, pal) })
      : null,
  };
}

export function cssUnderlineStyle(s) {
  if (s === "curl" || s === "wavy" || s === "undercurl") return "wavy";
  if (s === "dotted") return "dotted";
  if (s === "dashed") return "dashed";
  if (s === "double_line") return "double";
  return "solid";
}

// Build a React style object for a token or markup span from its scope's theme style.
// `tag` (optional) overrides fg/bg and appends modifiers (used for cursor/selection/match).
// `editorBg` is the canvas background, needed for the `reversed` modifier.
export function tokenStyle(scopeId, sc, pal, fallbackFg, editorBg, tag) {
  const s = scopeId ? fullStyle(scopeId, sc, pal) : null;
  let fg = tag?.fg ?? s?.fg ?? fallbackFg;
  let bg = tag?.bg ?? s?.bg ?? null;
  const mods = [...(s?.mods ?? []), ...(tag?.mods ?? [])];
  const u = s?.underline;

  if (mods.includes("reversed")) { const t = fg; fg = editorBg; bg = t; }

  const decos = [];
  if (mods.includes("strikethrough") || mods.includes("crossed_out")) decos.push("line-through");
  if (mods.includes("underlined") || u) decos.push("underline");

  const css = {};
  css.color = mods.includes("hidden") ? "transparent" : fg;
  if (bg != null) css.background = bg;
  if (mods.includes("bold")) css.fontWeight = 700;
  if (mods.includes("italic")) css.fontStyle = "italic";
  if (mods.includes("dim")) css.opacity = 0.6;
  if (decos.length) {
    css.textDecoration = decos.join(" ");
    if (u) {
      css.textDecorationStyle = cssUnderlineStyle(u.style);
      if (u.color) css.textDecorationColor = u.color;
    }
  }
  if (mods.includes("slow_blink")) css.animation = "hume-blink 1s steps(1,end) infinite";
  else if (mods.includes("rapid_blink")) css.animation = "hume-blink 0.5s steps(1,end) infinite";
  if (tag) { css.borderRadius = 2; css.padding = "0 1px"; }
  return css;
}
