import { useState } from 'react';
import { C, INPUT, MONO } from '../ui.js';
import { resolve } from '../lib/theme.js';
import Swatch from './Swatch.jsx';

// The loader's accepted modifier vocabulary (hume-engine/src/theme/loader.rs's
// `parse_modifier`) plus "underlined", which the loader routes to the
// dedicated underline field rather than the modifier bitset — offering
// anything else here would let the editor author a theme HUME's loader
// rejects on load.
const MODIFIERS = ["bold", "italic", "dim", "reversed", "hidden", "crossed_out", "slow_blink", "rapid_blink", "underlined"];
const UNDERLINE_STYLES = ["line", "curl", "dotted", "dashed", "double_line"];
const STYLE_KEYS = ["fg", "bg", "modifiers", "underline"];

export default function ScopeRow({ id, value, palette, onChange }) {
  const isObj = typeof value === "object" && value !== null;
  const fgVal = isObj ? (value.fg || "") : (value || "");
  const bgVal = isObj ? (value.bg || "") : "";
  const modifiers = isObj && Array.isArray(value.modifiers) ? value.modifiers : [];
  const underlineRaw = isObj ? (value.underline ?? null) : null;
  const underlineStyle = typeof underlineRaw === "string" ? underlineRaw
    : (underlineRaw && typeof underlineRaw === "object" ? (underlineRaw.style || "line") : "line");
  const underlineColorVal = underlineRaw && typeof underlineRaw === "object" ? (underlineRaw.color || "") : "";
  const hasUnderline = modifiers.includes("underlined");
  // Fields Helix themes carry that this editor doesn't author (e.g. `style`)
  // survive an import/export round-trip untouched — see README's Known limitations.
  const extra = isObj
    ? Object.fromEntries(Object.entries(value).filter(([k]) => !STYLE_KEYS.includes(k)))
    : {};
  const palNames = Object.keys(palette);

  const [fgCustom, setFgCustom] = useState(false);
  const [bgCustom, setBgCustom] = useState(false);
  const [fgHex, setFgHex] = useState("");
  const [bgHex, setBgHex] = useState("");
  const [ulColorCustom, setUlColorCustom] = useState(false);
  const [ulColorHex, setUlColorHex] = useState("");

  // Rows are keyed by a fixed `id` (see App.jsx), so a row is never remounted
  // when `value` changes out from under it (e.g. a theme import overwrites
  // `scopes` while this row is showing a typed-but-uncommitted custom hex).
  // Reset the local "custom hex" state whenever the resolved fg/bg actually
  // changes — this is the React-documented "adjust state during render"
  // pattern, not an effect, so it applies before paint with no extra render.
  // A self-triggered `emit()` also changes `value` (and so re-fires this),
  // but harmlessly: `showCustom`'s `val`-based fallback keeps the custom
  // input visible and in sync even with `*Custom` reset to false.
  const [prevFgVal, setPrevFgVal] = useState(fgVal);
  const [prevBgVal, setPrevBgVal] = useState(bgVal);
  const [prevUlColorVal, setPrevUlColorVal] = useState(underlineColorVal);
  if (fgVal !== prevFgVal) {
    setPrevFgVal(fgVal);
    setFgCustom(false);
    setFgHex("");
  }
  if (bgVal !== prevBgVal) {
    setPrevBgVal(bgVal);
    setBgCustom(false);
    setBgHex("");
  }
  if (underlineColorVal !== prevUlColorVal) {
    setPrevUlColorVal(underlineColorVal);
    setUlColorCustom(false);
    setUlColorHex("");
  }

  // Assemble the next scope value from a partial patch over the current
  // fg/bg/modifiers/underline, collapsing to a bare fg string when nothing
  // else is set — matches sand.toml's own shorthand (e.g.
  // `"punctuation.bracket" = "muted"`) instead of always writing a table.
  const emit = patch => {
    const next = {
      fg: patch.fg !== undefined ? patch.fg : fgVal,
      bg: patch.bg !== undefined ? patch.bg : bgVal,
      modifiers: patch.modifiers !== undefined ? patch.modifiers : modifiers,
      underline: patch.underline !== undefined ? patch.underline : underlineRaw,
    };
    const hasExtra = Object.keys(extra).length > 0;
    const hasBg = next.bg !== "";
    const hasMods = next.modifiers.length > 0;
    const hasUl = next.underline != null;
    if (next.fg === "" && !hasBg && !hasMods && !hasUl && !hasExtra) { onChange(null); return; }
    if (!hasBg && !hasMods && !hasUl && !hasExtra) { onChange(next.fg); return; }
    const def = { ...extra };
    if (next.fg !== "") def.fg = next.fg;
    if (hasBg) def.bg = next.bg;
    if (hasMods) def.modifiers = next.modifiers;
    if (hasUl) def.underline = next.underline;
    onChange(def);
  };

  function toggleModifier(name) {
    const has = modifiers.includes(name);
    const nextMods = has ? modifiers.filter(m => m !== name) : [...modifiers, name];
    const patch = { modifiers: nextMods };
    // Turning the modifier off drops any explicit style/color override too —
    // an `underline` field with no `underlined` modifier is meaningless.
    if (name === "underlined" && has) patch.underline = null;
    emit(patch);
  }

  // "line" + no color needs no explicit `underline` field at all: the
  // `underlined` modifier alone already resolves to a plain (Solid) underline
  // (loader.rs's modifiers-array match arm), matching every bundled theme's
  // own convention. Anything else needs the field to carry the difference.
  function setUnderline(style, color) {
    if (style === "line" && !color) { emit({ underline: null }); return; }
    emit({ underline: color ? { style, color } : style });
  }

  const sel = { ...INPUT, flex: 1, minWidth: 0 };

  const rFg = resolve(fgVal, palette);
  const rBg = resolve(bgVal, palette);

  function renderSelect(val, isCustom, hexVal, setCustom, setHex, onCommit) {
    // A value that isn't blank and isn't a known palette name (e.g. an imported
    // literal hex) must also render as "custom", even before the user touches it —
    // otherwise the dropdown falls back to "-- none --" while the swatch shows a color.
    const showCustom = isCustom || (val !== "" && !palNames.includes(val));
    const handleSel = e => {
      const v = e.target.value;
      if (v === "__custom__") { setCustom(true); return; }
      setCustom(false);
      setHex("");
      onCommit(v);
    };
    const handleHex = e => {
      setCustom(true);
      setHex(e.target.value);
      if (/^#[0-9a-fA-F]{6}$/.test(e.target.value)) onCommit(e.target.value);
    };
    return (
      <div style={{ flex: 1, minWidth: 0 }}>
        <select value={showCustom ? "__custom__" : val} onChange={handleSel} style={sel}>
          <option value="">{"-- none --"}</option>
          {palNames.map(n => <option key={n} value={n}>{n}</option>)}
          <option value="__custom__">{"Custom hex"}</option>
        </select>
        {showCustom && (
          <input type="text" placeholder="#ff00aa" value={isCustom ? hexVal : val} onChange={handleHex}
            style={{ ...INPUT, width: "100%", marginTop: 3, boxSizing: "border-box" }} />
        )}
      </div>
    );
  }

  return (
    <div style={{ padding: "6px 4px", borderBottom: "1px solid " + C.bgRow }}>
      <div style={{ fontSize: 11, color: C.textMuted, marginBottom: 4, fontFamily: MONO, display: "flex", alignItems: "center", gap: 6 }}>
        <Swatch color={rFg} />
        <Swatch color={rBg} />
        <span style={{ opacity: 0.7 }}>{id}</span>
      </div>
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <span style={{ fontSize: 9, color: C.textDim, width: 16, flexShrink: 0 }}>FG</span>
        {renderSelect(fgVal, fgCustom, fgHex, setFgCustom, setFgHex, v => emit({ fg: v }))}
      </div>
      <div style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 4 }}>
        <span style={{ fontSize: 9, color: C.textDim, width: 16, flexShrink: 0 }}>BG</span>
        {renderSelect(bgVal, bgCustom, bgHex, setBgCustom, setBgHex, v => emit({ bg: v }))}
      </div>
      <div style={{ display: "flex", gap: 4, flexWrap: "wrap", marginTop: 6 }}>
        {MODIFIERS.map(m => {
          const on = modifiers.includes(m);
          return (
            <button key={m} onClick={() => toggleModifier(m)}
              style={{
                background: on ? C.accent : C.bgBtn, color: on ? C.bg : C.textDim,
                border: "none", borderRadius: 8, padding: "1px 7px", fontSize: 9,
                fontFamily: MONO, cursor: "pointer", fontWeight: on ? 600 : 400,
              }}>
              {m}
            </button>
          );
        })}
      </div>
      {hasUnderline && (
        <div style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 4, paddingLeft: 22 }}>
          <select value={underlineStyle} onChange={e => setUnderline(e.target.value, underlineColorVal)} style={{ ...sel, flex: "0 0 90px" }}>
            {UNDERLINE_STYLES.map(s => <option key={s} value={s}>{s}</option>)}
          </select>
          {renderSelect(underlineColorVal, ulColorCustom, ulColorHex, setUlColorCustom, setUlColorHex, v => setUnderline(underlineStyle, v))}
        </div>
      )}
    </div>
  );
}
