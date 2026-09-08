// The sixteen terminal colour names HUME's loader resolves to a fixed RGB
// value (see `ANSI_COLORS` in hume-engine/src/theme/loader.rs, the source of
// truth for these — the two tables are hand-kept in sync). A theme's own
// `[palette]` entry of the same name still wins, matching the loader's order.
const ANSI_COLORS = {
  black: "#000000", red: "#cd0000", green: "#00cd00", yellow: "#cdcd00",
  blue: "#0000ee", magenta: "#cd00cd", cyan: "#00cdcd", "light-gray": "#e5e5e5",
  gray: "#7f7f7f", "light-red": "#ff0000", "light-green": "#00ff00", "light-yellow": "#ffff00",
  "light-blue": "#5c5cff", "light-magenta": "#ff00ff", "light-cyan": "#00ffff", white: "#ffffff",
};

// Mirrors STYLE_KEYS in hume-engine/src/theme/loader.rs — a key here is part
// of a scope's style, anything else is a child scope. The single copy other
// modules import, rather than each hand-typing their own (as toml.js's
// `SCOPE_KEYS` and ScopeRow.jsx's own `STYLE_KEYS` used to). `underline`'s
// nested `style` key belongs to that sub-table, not this list, so it stays
// out even though it's an underline-related word.
export const STYLE_KEYS = ["fg", "bg", "underline", "modifiers"];

export function resolveColor(c, pal) {
  if (!c || typeof c !== "string") return null;
  return c.startsWith("#") ? c : (pal[c] || ANSI_COLORS[c] || c);
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

// The colours an unmatched scope falls back to. `Theme::resolve_raw`
// (hume-engine/src/theme/mod.rs) ends its dotted walk at `Theme::default`,
// which the loader folds `ui.text` into, and the pane background comes from
// `ui.background`. Every preview fallback resolves through these two rather
// than carrying a literal of its own, so a theme that leaves a chrome scope
// unset previews the colour HUME would actually render for it. The literals
// here are the true last resort: a theme that defines neither.
export const baseFg = (sc, pal) => fgc("ui.text", sc, pal, "#c0caf5");
export const baseBg = (sc, pal) => bgc("ui.background", sc, pal, "#1a1b26");

// Normalise a raw scope value (a bare color string, or a `{fg,bg,modifiers,
// underline}` table) into a fully-resolved style. Shared by `fullStyle` and
// `fullStyleChain` so the two lookup strategies below produce the same shape.
function normalizeStyle(v, pal) {
  if (typeof v === "string") return { fg: resolveColor(v, pal), bg: null, mods: [], underline: null };
  const u = v.underline;
  return {
    fg: resolveColor(v.fg, pal),
    bg: resolveColor(v.bg, pal),
    mods: Array.isArray(v.modifiers) ? v.modifiers : [],
    // A table form with no `style` key resolves to no underline at all, not to
    // a solid one: `parse_style_table` (hume-engine/src/theme/loader.rs) sets
    // only `underline_color` in that case, and `ResolvedStyle::normalized`
    // (hume-grid/src/style.rs) then clears the colour because no style was
    // ever set. Defaulting to "line" here previews an underline HUME doesn't
    // draw.
    underline: u
      ? (typeof u === "string"
          ? { style: u, color: null }
          : (u.style ? { style: u.style, color: resolveColor(u.color, pal) } : null))
      : null,
  };
}

// Returns a fully normalised style for a scope (including modifiers and underline),
// walking the dotted fallback chain. Returns null when no scope matches.
export function fullStyle(id, sc, pal) {
  const v = lookupRaw(id, sc);
  return v == null ? null : normalizeStyle(v, pal);
}

// The text-span squiggle a diagnostic of severity `sev` draws — HUME's
// `diagnostic.<sev>` scope, distinct from the bare gutter dot (`error` etc.)
// and the end-of-line summary (`<sev>.diagnostic.inline`) the preview also
// shows. A thin wrapper over `fullStyle` so the scope-name string lives in
// one place and the resolution is unit-testable without importing JSX.
export function diagnosticStyle(sev, sc, pal) {
  return fullStyle("diagnostic." + sev, sc, pal);
}

// Resolve the first key in `ids` that has an explicit scope entry — no
// dotted-chain fallback beyond what `ids` itself lists. Mirrors
// `resolve_cursor_chain` in hume-engine/src/theme/mod.rs. Returns null when
// no listed key is defined — `cursorColors` below is the only caller, and it
// always lists a final rung (`ui.selection`) a real theme is expected to
// define, so this is the rare "theme defines none of its cursor scopes at
// all" case, not the common one.
export function fullStyleChain(ids, sc, pal) {
  for (const id of ids) {
    const v = sc[id];
    if (v !== undefined && v !== "") return normalizeStyle(v, pal);
  }
  return null;
}

// Resolve a selection-head's full style (fg/bg/modifiers) for one of HUME's
// three cursor-scope chains, mirroring `cursor_ladder` in
// hume-engine/src/theme/mod.rs exactly:
//   secondary: `ui.cursor.<chain>` -> `ui.cursor` -> `ui.selection`
//   primary:   `ui.cursor.primary.<chain>` -> `ui.cursor.primary` ->
//              `ui.cursor` -> `ui` -> `ui.selection`
// `chain` is `"normal"`, `"insert"`, or `"select"` — see `MODES` in
// preview/samples.js for which chain each of HUME's modes actually uses.
// Always returns a normalized style object, never null: both ladders end at
// `ui.selection`, matching how the real ladder falls back to an all-`None`
// `ResolvedStyle` rather than "no style at all".
export function cursorColors(chain, primary, sc, pal) {
  const ids = primary
    ? [`ui.cursor.primary.${chain}`, "ui.cursor.primary", "ui.cursor", "ui", "ui.selection"]
    : [`ui.cursor.${chain}`, "ui.cursor", "ui.selection"];
  return fullStyleChain(ids, sc, pal) ?? { fg: null, bg: null, mods: [], underline: null };
}

export function cssUnderlineStyle(s) {
  if (s === "curl") return "wavy";
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
  // The tag is layered over the token's own scope, so its underline wins the
  // same way its fg/bg do — the "diag" tag exists precisely to draw a squiggle,
  // and reading only the token's own scope dropped it for every token that
  // carries no underline itself.
  const u = tag?.underline ?? s?.underline;

  if (mods.includes("reversed")) { const t = fg; fg = editorBg; bg = t; }

  const decos = [];
  if (mods.includes("crossed_out")) decos.push("line-through");
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
  return css;
}
