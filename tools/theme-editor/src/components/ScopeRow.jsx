import { useState } from 'react';
import { C, INPUT, MONO } from '../ui.js';
import { resolve } from '../lib/theme.js';
import Swatch from './Swatch.jsx';

export default function ScopeRow({ id, value, palette, onChange }) {
  const isObj = typeof value === "object" && value !== null;
  const fgVal = isObj ? (value.fg || "") : (value || "");
  const bgVal = isObj ? (value.bg || "") : "";
  const hasBg = isObj || id.startsWith("ui.");
  const palNames = Object.keys(palette);

  const [fgCustom, setFgCustom] = useState(false);
  const [bgCustom, setBgCustom] = useState(false);
  const [fgHex, setFgHex] = useState("");
  const [bgHex, setBgHex] = useState("");

  // Rows are keyed by a fixed `id` (see App.jsx), so a row is never remounted
  // when `value` changes out from under it (e.g. a theme import overwrites
  // `scopes` while this row is showing a typed-but-uncommitted custom hex).
  // Reset the local "custom hex" state whenever the resolved fg/bg actually
  // changes — this is the React-documented "adjust state during render"
  // pattern, not an effect, so it applies before paint with no extra render.
  // A self-triggered `emit()` also changes `value` (and so re-fires this),
  // but harmlessly: `showCustom`'s `val`-based fallback keeps the custom
  // input visible and in sync even with `fgCustom`/`bgCustom` reset to false.
  const [prevFgVal, setPrevFgVal] = useState(fgVal);
  const [prevBgVal, setPrevBgVal] = useState(bgVal);
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

  const emit = (newFg, newBg) => {
    const extra = (typeof value === "object" && value !== null)
      ? Object.fromEntries(Object.entries(value).filter(([k]) => k !== "fg" && k !== "bg"))
      : {};
    const hasExtra = Object.keys(extra).length > 0;
    if (!hasBg) {
      // extra is always {} here: hasBg is false only when value isn't an object.
      if (newFg === "") { onChange(null); return; }
      onChange(newFg);
      return;
    }
    if (newFg === "" && newBg === "") {
      if (!hasExtra) { onChange(null); return; }
      onChange({ ...extra });
    }
    else if (newBg === "")            onChange(hasExtra ? { ...extra, fg: newFg } : newFg);
    else if (newFg === "")            onChange({ ...extra, bg: newBg });
    else                              onChange({ ...extra, fg: newFg, bg: newBg });
  };

  const sel = { ...INPUT, flex: 1, minWidth: 0 };

  const rFg = resolve(fgVal, palette);
  const rBg = resolve(bgVal, palette);

  function renderSelect(val, isCustom, hexVal, setCustom, setHex, isBgField) {
    // A value that isn't blank and isn't a known palette name (e.g. an imported
    // literal hex) must also render as "custom", even before the user touches it —
    // otherwise the dropdown falls back to "-- none --" while the swatch shows a color.
    const showCustom = isCustom || (val !== "" && !palNames.includes(val));
    const handleSel = e => {
      const v = e.target.value;
      if (v === "__custom__") { setCustom(true); return; }
      setCustom(false);
      setHex("");
      if (isBgField) emit(fgVal, v); else emit(v, bgVal);
    };
    const handleHex = e => {
      setCustom(true);
      setHex(e.target.value);
      if (/^#[0-9a-fA-F]{6}$/.test(e.target.value)) {
        if (isBgField) emit(fgVal, e.target.value); else emit(e.target.value, bgVal);
      }
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
        {hasBg && <Swatch color={rBg} />}
        <span style={{ opacity: 0.7 }}>{id}</span>
      </div>
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <span style={{ fontSize: 9, color: C.textDim, width: 16, flexShrink: 0 }}>FG</span>
        {renderSelect(fgVal, fgCustom, fgHex, setFgCustom, setFgHex, false)}
      </div>
      {hasBg && (
        <div style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 4 }}>
          <span style={{ fontSize: 9, color: C.textDim, width: 16, flexShrink: 0 }}>BG</span>
          {renderSelect(bgVal, bgCustom, bgHex, setBgCustom, setBgHex, true)}
        </div>
      )}
    </div>
  );
}
