import { useState, useCallback, useMemo, useRef } from 'react';
import { C, MONO, INPUT, COLOR_PICKER, pill } from '../ui.js';
import { adjustPalette } from '../lib/color.js';
import { parseTOML, extractScopes, exportTOML, diffFromBaseline } from '../lib/toml.js';
import { SCOPES, ALL_SCOPES, DEFAULT_PAL, DEFAULT_SC } from '../data.js';
import Acc from './Acc.jsx';
import ScopeRow from './ScopeRow.jsx';
import EditorPane from '../preview/EditorPane.jsx';
import MessagesPane from '../preview/MessagesPane.jsx';

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
  // parent's palette/scopes, and that parent may itself inherit further
  // (theme -> variant -> base). We stash each imported child's overrides on a
  // stack, most-derived first, show a banner asking for the next ancestor, and
  // merge the whole stack onto the root once a non-inherits theme arrives.
  // `loadedThemeName` (the imported file's basename, matching Helix's
  // filename-is-the-theme-name convention) tracks WHICH theme is currently
  // loaded, so a later inherits-import only merges directly when it actually
  // names the loaded theme as its parent — not merely because *some*
  // non-inherits theme happens to be loaded (that could be an unrelated
  // theme, giving a child the wrong base).
  const [pendingChildren, setPendingChildren] = useState([]);
  const [inheritBanner, setInheritBanner] = useState(null);
  const [loadedThemeName, setLoadedThemeName] = useState(null);
  const [importError, setImportError] = useState(null);
  // Snapshot of the resolved parent's own palette/scopes, taken the moment a
  // child theme merges onto it — `{ name, palette, scopes }` or `null`. Lets
  // `handleExport` emit only this theme's own overrides plus `inherits`,
  // instead of the merged (parent + child) state the app actually renders
  // from. `null` covers both "this is a plain root theme" and "still waiting
  // on the banner's ancestor" — either way there's nothing to diff against,
  // so export falls back to emitting everything, as it always has.
  const [parentBaseline, setParentBaseline] = useState(null);
  const fileRef = useRef(null);

  // Merge a most-derived-first override stack onto a base, most-derived wins.
  const mergeChildStack = (children, basePalette = {}, baseScopes = {}) => {
    let palette = { ...basePalette };
    let scopes = { ...baseScopes };
    for (let i = children.length - 1; i >= 0; i--) {
      palette = { ...palette, ...children[i].palette };
      scopes = { ...scopes, ...children[i].scopes };
    }
    return { palette, scopes };
  };

  const handleImport = useCallback(e => {
    const file = e.target.files?.[0];
    if (!file) return;
    const themeName = file.name.replace(/\.toml$/i, "");
    const reader = new FileReader();
    reader.onload = ev => {
      try {
        const parsed = parseTOML(ev.target.result);
        setImportError(null);
        const newPalette = parsed.palette || {};
        const newScopes = extractScopes(parsed);
        const hasInherits = typeof parsed.inherits === "string" && parsed.inherits.length > 0;
        setHShift(0); setSShift(0); setLShift(0);

        if (hasInherits) {
          if (loadedThemeName && loadedThemeName === parsed.inherits) {
            // The loaded theme is confirmed (by name) to be this child's
            // declared parent — merge child overrides directly on top of it.
            // The pre-merge `palette`/`scopes` state *is* that parent's own
            // content, so it becomes the new export baseline. Track this
            // child as the now-loaded theme (not the parent it just merged
            // onto) so a further import naming *it* as their parent (a
            // theme -> variant -> base chain) also matches.
            setParentBaseline({ name: loadedThemeName, palette, scopes });
            setPalette(p => ({...p, ...newPalette}));
            setScopes(s => ({...s, ...newScopes}));
            setPendingChildren([]);
            setInheritBanner(null);
            setLoadedThemeName(themeName);
          } else {
            // Either nothing is loaded yet, or what's loaded isn't this
            // child's parent — stack this child's overrides (discarding
            // whatever was loaded, since it's the wrong base), apply the
            // stack for visual feedback, and point the banner at the real
            // ancestor this file names. The resolved baseline is unknown
            // until that ancestor arrives.
            const nextPending = [...pendingChildren, { palette: newPalette, scopes: newScopes }];
            setPendingChildren(nextPending);
            const merged = mergeChildStack(nextPending);
            setPalette(merged.palette);
            setScopes(merged.scopes);
            setInheritBanner({ parent: parsed.inherits });
            setParentBaseline(null);
            setLoadedThemeName(null);
          }
        } else {
          if (pendingChildren.length && inheritBanner?.parent === themeName) {
            // This is the ancestor the banner asked for — its own content,
            // before the pending stack lands on top, is the export baseline.
            setParentBaseline({ name: themeName, palette: newPalette, scopes: newScopes });
            const merged = mergeChildStack(pendingChildren, newPalette, newScopes);
            setPalette(merged.palette);
            setScopes(merged.scopes);
            setPendingChildren([]);
            setInheritBanner(null);
          } else {
            // A fresh root theme — any stashed children were waiting on a
            // different ancestor and no longer apply, and a root theme has
            // no parent to diff its export against.
            setPalette(newPalette);
            setScopes(newScopes);
            setPendingChildren([]);
            setInheritBanner(null);
            setParentBaseline(null);
          }
          setLoadedThemeName(themeName);
        }
      } catch (err) {
        setImportError(err.message || String(err));
      }
    };
    // A read that never yields a result must say so rather than looking like
    // an import that quietly did nothing.
    reader.onerror = () =>
      setImportError(`could not read ${file.name}: ${reader.error?.message ?? "read failed"}`);
    reader.readAsText(file);
    e.target.value = "";
  }, [loadedThemeName, pendingChildren, inheritBanner, palette, scopes]);

  const handleExport = useCallback(() => {
    // A resolved `parentBaseline` means `scopes`/`adjPalette` hold the
    // *merged* state (parent content plus this theme's own overrides) — diff
    // against the baseline so the export is `inherits` plus only the child's
    // own overrides, round-tripping as the small file it started as instead
    // of a flattened ~340-scope dump of everything the parent also provides.
    // Without one (a plain root theme, or an inherits chain still waiting on
    // its ancestor) export everything, unchanged from before.
    let exportPalette = adjPalette;
    let exportScopes = scopes;
    let inherits = inheritBanner?.parent;
    if (parentBaseline) {
      inherits = parentBaseline.name;
      exportPalette = diffFromBaseline(adjPalette, parentBaseline.palette);
      exportScopes = diffFromBaseline(scopes, parentBaseline.scopes);
    }

    const toml = exportTOML(exportPalette, exportScopes, inherits);
    const blob = new Blob([toml], { type: "application/toml" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = loadedThemeName ? `${loadedThemeName}.toml` : "theme.toml";
    a.click();
    URL.revokeObjectURL(url);
  }, [adjPalette, scopes, inheritBanner, parentBaseline, loadedThemeName]);

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

  return (
    <div style={{ minHeight: "100vh", background: C.bg, color: C.text, fontFamily: MONO, display: "flex", flexDirection: "column" }}>
      <div style={{ padding: "12px 20px", background: C.bgHeader, borderBottom: "1px solid " + C.border, display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: 10 }}>
        <span style={{ fontSize: 18, fontWeight: 700, color: C.brand, letterSpacing: "0.04em" }}>
          {"⬡ hume theme editor"}
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

      {importError && (
        <div style={{ padding: "8px 20px", background: "#3a1a1a", borderBottom: "1px solid #4a2a2a", color: "#f38ba8", fontFamily: MONO, fontSize: 12, display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <span>{"Import failed: " + importError}</span>
          <button onClick={() => setImportError(null)} style={{ background: "transparent", border: "1px solid #4a2a2a", color: "#f38ba8", borderRadius: 4, padding: "2px 8px", cursor: "pointer", fontFamily: MONO, fontSize: 11, flexShrink: 0 }} title="Dismiss">{"×"}</button>
        </div>
      )}

      {inheritBanner && (
        <div style={{ padding: "8px 20px", background: "#3a2f1a", borderBottom: "1px solid #4a3f2a", color: "#f9e2af", fontFamily: MONO, fontSize: 12, display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <span>
            {"This theme inherits from "}
            <code style={{ background: "#1f1a0e", padding: "1px 6px", borderRadius: 3, color: "#fab387" }}>{inheritBanner.parent + ".toml"}</code>
            {" — import the parent file to apply its scopes. Child overrides are stashed and will be re-applied on top."}
          </span>
          <button onClick={() => { setInheritBanner(null); setPendingChildren([]); setParentBaseline(null); }} style={{ background: "transparent", border: "1px solid #4a3f2a", color: "#f9e2af", borderRadius: 4, padding: "2px 8px", cursor: "pointer", fontFamily: MONO, fontSize: 11, flexShrink: 0 }} title="Dismiss and discard pending overrides">{"×"}</button>
        </div>
      )}

      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <div style={{ flex: 1, padding: 24, overflowY: "auto", display: "flex", flexDirection: "column", gap: 20 }}>
          <EditorPane pal={adjPalette} sc={scopes} />
          <MessagesPane pal={adjPalette} sc={scopes} />
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
                  <button key={c} onClick={() => setCatFilter(c)} style={pill(catFilter === c)}>{c}</button>
                ))}
              </div>
            </div>
            {filtered.map(id => (
              <ScopeRow key={id} id={id} value={scopes[id] || ""} palette={adjPalette} onChange={v => setScopes(s => {
                if (v === null) { const n = {...s}; delete n[id]; return n; }
                return {...s, [id]: v};
              })} />
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
