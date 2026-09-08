import { useState } from 'react';
import { C, INPUT, MONO, pill } from '../ui.js';
import { resolve, STYLE_KEYS, MODIFIERS, UNDERLINE_STYLES } from '../lib/theme.js';
import Swatch from './Swatch.jsx';

const UNDERLINE_STYLE_NAMES = Object.keys(UNDERLINE_STYLES);

export default function ScopeRow({ id, value, palette, onChange }) {
  const isObj = typeof value === "object" && value !== null;
  const fgVal = isObj ? (value.fg || "") : (value || "");
  const bgVal = isObj ? (value.bg || "") : "";
  const modifiers = isObj && Array.isArray(value.modifiers) ? value.modifiers : [];
  const underlineRaw = isObj ? (value.underline ?? null) : null;
  const underlineStyle = typeof underlineRaw === "string" ? underlineRaw
    : (underlineRaw && typeof underlineRaw === "object" ? (underlineRaw.style || "line") : "line");
  const underlineColorVal = underlineRaw && typeof underlineRaw === "object" ? (underlineRaw.color || "") : "";
  // The loader reads `underline` unconditionally — it is not gated on the
  // `underlined` modifier (which is just a shorthand for a solid one). So an
  // imported `{ fg = "red", underline = "curl" }` has a real, rendered
  // underline with no modifier set, and must be editable here.
  const hasUnderline = modifiers.includes("underlined") || underlineRaw != null;
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
    // No caller ever patches a key to `undefined` (only a real value or
    // `null`), so a plain spread over the current fg/bg/modifiers/underline
    // is exactly the "patch wins where given" merge this needs.
    const next = { fg: fgVal, bg: bgVal, modifiers, underline: underlineRaw, ...patch };
    // Fields Helix themes carry that this editor doesn't author (e.g. `style`)
    // survive an import/export round-trip untouched — see README's Known
    // limitations. Computed here, not in the render body: `emit` is the only
    // reader, and it only runs on a user edit, not every render.
    const extra = isObj
      ? Object.fromEntries(Object.entries(value).filter(([k]) => !STYLE_KEYS.includes(k)))
      : {};
    const hasExtra = Object.keys(extra).length > 0;
    const hasBg = next.bg !== "";
    const hasMods = next.modifiers.length > 0;
    const hasUl = next.underline != null;
    if (!hasBg && !hasMods && !hasUl && !hasExtra) {
      onChange(next.fg === "" ? null : next.fg);
      return;
    }
    const def = { ...extra };
    if (next.fg !== "") def.fg = next.fg;
    if (hasBg) def.bg = next.bg;
    if (hasMods) def.modifiers = next.modifiers;
    if (hasUl) def.underline = next.underline;
    onChange(def);
  };

  // Turning `underlined` off leaves any explicit `underline` field alone: the
  // loader reads that field whether or not the modifier is set, so dropping it
  // here would silently delete a style the modifier never controlled.
  function toggleModifier(name) {
    const has = modifiers.includes(name);
    const nextMods = has ? modifiers.filter(m => m !== name) : [...modifiers, name];
    emit({ modifiers: nextMods });
  }

  // "line" + no color needs no explicit `underline` field *while the
  // `underlined` modifier is set* — that modifier alone already resolves to a
  // plain (Solid) underline (loader.rs's modifiers-array match arm), which is
  // every bundled theme's own convention. Without it the field is the only
  // thing carrying the underline, so it has to be written out.
  function setUnderline(style, color) {
    if (style === "line" && !color && modifiers.includes("underlined")) {
      emit({ underline: null });
      return;
    }
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
              style={pill(on, { inactiveColor: C.textDim, radius: 8, padding: "1px 7px", fontSize: 9 })}>
              {m}
            </button>
          );
        })}
      </div>
      {hasUnderline && (
        <div style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 4, paddingLeft: 22 }}>
          <select value={underlineStyle} onChange={e => setUnderline(e.target.value, underlineColorVal)} style={{ ...sel, flex: "0 0 90px" }}>
            {UNDERLINE_STYLE_NAMES.map(s => <option key={s} value={s}>{s}</option>)}
          </select>
          {renderSelect(underlineColorVal, ulColorCustom, ulColorHex, setUlColorCustom, setUlColorHex, v => setUnderline(underlineStyle, v))}
        </div>
      )}
    </div>
  );
}
