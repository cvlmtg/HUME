import { useState, useCallback, useMemo, useRef } from 'react';
import { C, MONO, INPUT, COLOR_PICKER } from '../ui.js';
import { adjustPalette } from '../lib/color.js';
import { parseTOML, extractScopes, exportTOML } from '../lib/toml.js';
import { fgc, bgc, tokenStyle } from '../lib/theme.js';
import { SCOPES, ALL_SCOPES, DEFAULT_PAL, DEFAULT_SC } from '../data.js';
import Acc from './Acc.jsx';
import ScopeRow from './ScopeRow.jsx';
import Preview from './Preview.jsx';

export default function HelixThemeEditor() {
  const [palette, setPalette] = useState(() => ({...DEFAULT_PAL}));
  const [scopes, setScopes] = useState(() => ({...DEFAULT_SC}));
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState("#ffffff");
  const [filter, setFilter] = useState("");
  const [catFilter, setCatFilter] = useState("All");
  const [hShift, setHShift] = useState(0);
  const [sShift, setSShift] = useState(0);
  const [lShift, setLShift] = useState(0);
  const adjPalette = useMemo(() => adjustPalette(palette, hShift, sShift, lShift), [palette, hShift, sShift, lShift]);
  const [palOpen, setPalOpen] = useState(true);
  const [scOpen, setScOpen] = useState(true);
  const [hslOpen, setHslOpen] = useState(true);
  // Inherits handling: child themes (`inherits = "..."`) only override the
  // parent's palette/scopes. We stash the child's overrides, show a banner
  // asking the user to import the parent, then merge child-on-top when it
  // arrives. `loadedParent` tracks whether a non-inherits theme is loaded so
  // the order can be reversed (parent first, then child).
  const [pendingChild, setPendingChild] = useState(null);
  const [inheritBanner, setInheritBanner] = useState(null);
  const [loadedParent, setLoadedParent] = useState(false);
  const fileRef = useRef(null);

  const handleImport = useCallback(e => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = ev => {
      try {
        const parsed = parseTOML(ev.target.result);
        const newPalette = parsed.palette || {};
        const newScopes = extractScopes(parsed);
        const hasInherits = typeof parsed.inherits === "string" && parsed.inherits.length > 0;
        setHShift(0); setSShift(0); setLShift(0);

        if (hasInherits) {
          if (loadedParent) {
            // Parent already loaded — merge child overrides on top.
            setPalette(p => ({...p, ...newPalette}));
            setScopes(s => ({...s, ...newScopes}));
            setPendingChild(null);
            setInheritBanner(null);
          } else {
            // No parent yet — apply child for visual feedback, stash for later
            // merge, and show the banner so the user knows to import the parent.
            setPalette(newPalette);
            setScopes(newScopes);
            setPendingChild({ palette: newPalette, scopes: newScopes });
            setInheritBanner({ parent: parsed.inherits });
          }
        } else {
          if (pendingChild) {
            // Child was imported first; this is the parent. Parent first, child on top.
            setPalette({...newPalette, ...pendingChild.palette});
            setScopes({...newScopes, ...pendingChild.scopes});
            setPendingChild(null);
            setInheritBanner(null);
          } else {
            setPalette(newPalette);
            setScopes(newScopes);
            setInheritBanner(null);
          }
          setLoadedParent(true);
        }
      } catch (err) {
        console.error("Parse error", err);
      }
    };
    reader.readAsText(file);
    e.target.value = "";
  }, [loadedParent, pendingChild]);

  const handleExport = useCallback(() => {
    const toml = exportTOML(adjPalette, scopes);
    const blob = new Blob([toml], { type: "application/toml" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "theme.toml";
    a.click();
    URL.revokeObjectURL(url);
  }, [adjPalette, scopes]);

  const addColor = () => {
    if (newName.trim() && /^#[0-9a-fA-F]{6}$/.test(newColor)) {
      setPalette(p => ({...p, [newName.trim()]: newColor}));
      setNewName("");
      setNewColor("#ffffff");
    }
  };

  const cats = ["All"].concat(SCOPES.map(([name]) => name));

  const filtered = useMemo(() => ALL_SCOPES.filter(id => {
    if (catFilter !== "All") {
      const cat = SCOPES.find(([, items]) => items.includes(id));
      if (cat && cat[0] !== catFilter) return false;
    }
    return !filter || id.toLowerCase().includes(filter.toLowerCase());
  }), [filter, catFilter]);

  const markupBg = bgc("ui.background", scopes, adjPalette, "#1a1b26");

  return (
    <div style={{ minHeight: "100vh", background: C.bg, color: C.text, fontFamily: MONO, display: "flex", flexDirection: "column" }}>
      <div style={{ padding: "12px 20px", background: C.bgHeader, borderBottom: "1px solid " + C.border, display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: 10 }}>
        <span style={{ fontSize: 18, fontWeight: 700, color: C.brand, letterSpacing: "0.04em" }}>
          {"⬡ helix theme editor"}
        </span>
        <div style={{ display: "flex", gap: 8 }}>
          <input type="file" ref={fileRef} accept=".toml" onChange={handleImport} style={{ display: "none" }} />
          <button onClick={() => fileRef.current?.click()} style={{ background: C.bgBtn, color: C.text, border: "1px solid " + C.borderHeader, borderRadius: 6, padding: "6px 14px", cursor: "pointer", fontFamily: MONO, fontSize: 12 }}>
            {"Import TOML"}
          </button>
          <button onClick={handleExport} style={{ background: C.accent, color: C.bg, border: "none", borderRadius: 6, padding: "6px 14px", cursor: "pointer", fontFamily: MONO, fontSize: 12, fontWeight: 600 }}>
            {"Export TOML"}
          </button>
        </div>
      </div>

      {inheritBanner && (
        <div style={{ padding: "8px 20px", background: "#3a2f1a", borderBottom: "1px solid #4a3f2a", color: "#f9e2af", fontFamily: MONO, fontSize: 12, display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <span>
            {"This theme inherits from "}
            <code style={{ background: "#1f1a0e", padding: "1px 6px", borderRadius: 3, color: "#fab387" }}>{inheritBanner.parent + ".toml"}</code>
            {" — import the parent file to apply its scopes. Child overrides are stashed and will be re-applied on top."}
          </span>
          <button onClick={() => { setInheritBanner(null); setPendingChild(null); }} style={{ background: "transparent", border: "1px solid #4a3f2a", color: "#f9e2af", borderRadius: 4, padding: "2px 8px", cursor: "pointer", fontFamily: MONO, fontSize: 11, flexShrink: 0 }} title="Dismiss and discard pending overrides">{"×"}</button>
        </div>
      )}

      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <div style={{ flex: 1, padding: 24, overflowY: "auto", display: "flex", flexDirection: "column", gap: 20 }}>
          <Preview pal={adjPalette} sc={scopes} />

          <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
            {["diagnostic.error", "diagnostic.warning", "diagnostic.info", "diagnostic.hint"].map(d => {
              const c = fgc(d, scopes, adjPalette, "#888");
              return (
                <div key={d} style={{ padding: "6px 14px", borderRadius: 6, background: c + "18", borderLeft: "3px solid " + c, color: c, fontSize: 11, fontFamily: MONO }}>
                  {d.split(".")[1]}
                </div>
              );
            })}
          </div>

          <div style={{ borderRadius: 6, overflow: "hidden", border: "1px solid " + C.border, fontFamily: MONO, fontSize: 12 }}>
            <div style={{ padding: "4px 12px", background: C.bgChrome, color: C.textDimmer, fontSize: 10, borderBottom: "1px solid " + C.border }}>diff preview</div>
            {[["diff.plus", "+ added line"], ["diff.minus", "- removed line"], ["diff.delta", "~ changed line"]].map(([scope, label]) => {
              const c = fgc(scope, scopes, adjPalette, "#888");
              return <div key={scope} style={{ padding: "4px 12px", background: c + "15", color: c }}>{label}</div>;
            })}
          </div>

          <div style={{ borderRadius: 6, padding: "12px 16px", background: markupBg, border: "1px solid " + C.border, fontFamily: MONO, fontSize: 12, lineHeight: "22px" }}>
            <div style={{ ...tokenStyle("markup.heading", scopes, adjPalette, "#7aa2f7", markupBg), fontSize: 14, marginBottom: 4 }}>{"# Markup Preview"}</div>
            <div style={{ color: fgc("ui.foreground", scopes, adjPalette, "#c0caf5") }}>
              {"Normal text with "}
              <span style={tokenStyle("markup.bold", scopes, adjPalette, "#ff9e64", markupBg)}>{"**bold**"}</span>
              {" and "}
              <span style={tokenStyle("markup.italic", scopes, adjPalette, "#bb9af7", markupBg)}>{"_italic_"}</span>
              {" and "}
              <span style={tokenStyle("markup.link", scopes, adjPalette, "#7dcfff", markupBg)}>{"[link](url)"}</span>
            </div>
            <div style={tokenStyle("markup.list", scopes, adjPalette, "#f7768e", markupBg)}>{"- list item"}</div>
            <span style={{ ...tokenStyle("markup.raw", scopes, adjPalette, "#9ece6a", markupBg), background: fgc("markup.raw", scopes, adjPalette, "#9ece6a") + "15", padding: "1px 5px", borderRadius: 3 }}>{"`code`"}</span>
          </div>
        </div>

        <div style={{ width: 340, flexShrink: 0, background: C.bgPanel, borderLeft: "1px solid " + C.border, overflowY: "auto", display: "flex", flexDirection: "column" }}>
          <Acc title="Palette" open={palOpen} onToggle={() => setPalOpen(!palOpen)} count={Object.keys(palette).length}>
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              {Object.entries(palette).map(([name, color]) => (
                <div key={name} style={{ display: "flex", alignItems: "center", gap: 6, padding: "4px 2px", borderBottom: "1px solid " + C.bgRow }}>
                  <input type="color" value={color} onChange={e => setPalette(p => ({...p, [name]: e.target.value}))} style={COLOR_PICKER} />
                  <span style={{ flex: 1, fontSize: 12, color: C.textInput, fontFamily: MONO, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{name}</span>
                  <span style={{ fontSize: 10, color: C.textDim, fontFamily: MONO, flexShrink: 0 }}>{color}</span>
                  <button onClick={() => setPalette(p => { const n = {...p}; delete n[name]; return n; })} style={{ background: "none", border: "none", color: C.textDim, cursor: "pointer", padding: "0 2px", fontSize: 14, lineHeight: 1, flexShrink: 0 }} title="Remove">{"×"}</button>
                </div>
              ))}
            </div>
            <div style={{ display: "flex", gap: 6, marginTop: 8, alignItems: "center", paddingTop: 6, borderTop: "1px solid " + C.border }}>
              <input type="color" value={newColor} onChange={e => setNewColor(e.target.value)} style={COLOR_PICKER} />
              <input type="text" value={newName} onChange={e => setNewName(e.target.value)} placeholder="name" onKeyDown={e => { if (e.key === "Enter") addColor(); }} style={{ ...INPUT, flex: 1, minWidth: 0 }} />
              <button onClick={addColor} style={{ background: C.success, color: C.bg, border: "none", borderRadius: 4, padding: "3px 10px", cursor: "pointer", fontFamily: MONO, fontSize: 11, fontWeight: 600, flexShrink: 0 }}>{"+"}</button>
            </div>
          </Acc>

          <Acc title="Scopes" open={scOpen} onToggle={() => setScOpen(!scOpen)} count={ALL_SCOPES.length}>
            <div style={{ marginBottom: 8 }}>
              <input type="text" value={filter} onChange={e => setFilter(e.target.value)} placeholder="Filter scopes..." style={{ ...INPUT, width: "100%", boxSizing: "border-box", padding: "5px 8px", marginBottom: 6 }} />
              <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                {cats.map(c => (
                  <button key={c} onClick={() => setCatFilter(c)} style={{ background: catFilter === c ? C.accent : C.bgBtn, color: catFilter === c ? C.bg : C.textMuted, border: "none", borderRadius: 10, padding: "2px 9px", fontSize: 10, fontFamily: MONO, cursor: "pointer", fontWeight: catFilter === c ? 600 : 400 }}>{c}</button>
                ))}
              </div>
            </div>
            {filtered.map(id => (
              <ScopeRow key={id} id={id} value={scopes[id] || ""} palette={adjPalette} onChange={v => setScopes(s => ({...s, [id]: v}))} />
            ))}
          </Acc>

          <Acc title="Global HSL" open={hslOpen} onToggle={() => setHslOpen(!hslOpen)}>
            {[
              { label: "Hue", value: hShift, set: setHShift, min: -180, max: 180, unit: "°" },
              { label: "Saturation", value: sShift, set: setSShift, min: -100, max: 100, unit: "%" },
              { label: "Lightness", value: lShift, set: setLShift, min: -100, max: 100, unit: "%" },
            ].map(sl => (
              <div key={sl.label} style={{ marginBottom: 14 }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 5 }}>
                  <span style={{ fontSize: 11, color: C.textMuted, fontFamily: MONO }}>{sl.label}</span>
                  <span style={{ fontSize: 11, color: C.textInput, fontFamily: MONO, minWidth: 50, textAlign: "right" }}>
                    {(sl.value > 0 ? "+" : "") + sl.value + sl.unit}
                  </span>
                </div>
                <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                  <input type="range" min={sl.min} max={sl.max} value={sl.value} onChange={e => sl.set(Number(e.target.value))}
                    style={{ flex: 1, accentColor: C.accent, height: 4, cursor: "pointer" }} />
                  <button onClick={() => sl.set(0)}
                    style={{ background: "none", border: "1px solid " + C.surface, color: C.textDim, borderRadius: 3, padding: "1px 6px", cursor: "pointer", fontFamily: MONO, fontSize: 10, flexShrink: 0 }}
                    title="Reset">
                    {"↺"}
                  </button>
                </div>
              </div>
            ))}
            <div style={{ display: "flex", gap: 6, marginTop: 4, borderTop: "1px solid " + C.border, paddingTop: 8 }}>
              <button onClick={() => { setHShift(0); setSShift(0); setLShift(0); }}
                style={{ flex: 1, background: C.bgBtn, color: C.textMuted, border: "1px solid " + C.surface, borderRadius: 4, padding: "4px 8px", cursor: "pointer", fontFamily: MONO, fontSize: 10 }}>
                {"Reset All"}
              </button>
              <button onClick={() => { setPalette({...adjPalette}); setHShift(0); setSShift(0); setLShift(0); }}
                style={{ flex: 1, background: C.accent, color: C.bg, border: "none", borderRadius: 4, padding: "4px 8px", cursor: "pointer", fontFamily: MONO, fontSize: 10, fontWeight: 600 }}>
                {"Apply to Palette"}
              </button>
            </div>
          </Acc>
        </div>
      </div>
    </div>
  );
}
