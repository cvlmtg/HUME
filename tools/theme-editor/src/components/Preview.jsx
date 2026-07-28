import { useState } from 'react';
import { C, MONO } from '../ui.js';
import { fgc, bgc, fullStyle, tokenStyle } from '../lib/theme.js';
import { CODE } from '../data.js';

// Mode -> (statusline scope, label) — mirrors HUME's own mapping
// (hume-editor/src/ui/theme.rs's `mode_scope` and each mode element's label).
const MODES = [
  { scope: "ui.statusline.normal", label: "NOR" },
  { scope: "ui.statusline.insert", label: "INS" },
  { scope: "ui.statusline.extend", label: "EXT" },
  { scope: "ui.statusline.search", label: "SRC" },
  { scope: "ui.statusline.command", label: "CMD" },
  { scope: "ui.statusline.select", label: "SEL" },
];

export default function Preview({ pal, sc }) {
  const [modeIdx, setModeIdx] = useState(0);
  const mode = MODES[modeIdx];

  const BG = bgc("ui.background", sc, pal, "#1a1b26");
  const FG = fgc("ui.foreground", sc, pal, fgc("ui.text", sc, pal, "#c0caf5"));
  const lnr = fgc("ui.linenr", sc, pal, "#565f89");
  const lnrS = fgc("ui.linenr.selected", sc, pal, "#e0af68");
  // The whole row resolves from the active mode's scope, falling back to the
  // base `ui.statusline` — same dotted-fallback chain HUME's engine uses, so
  // a theme that omits a mode scope previews exactly as it will render.
  const rowFg = fgc(mode.scope, sc, pal, fgc("ui.statusline", sc, pal, "#c0caf5"));
  const rowBg = bgc(mode.scope, sc, pal, bgc("ui.statusline", sc, pal, "#33374c"));
  const sepFg = fgc("ui.statusline.separator", sc, pal, rowFg);
  const brd = fgc("ui.window", sc, pal, "#565f89");
  const statusBg = rowBg !== "transparent" ? rowBg : BG;

  const selPrimBg = bgc("ui.selection.primary", sc, pal, "#565f89");
  const selSecBg  = bgc("ui.selection", sc, pal, "#33374c");

  const searchFg = fgc("ui.selection.search", sc, pal, "#ff9e64");
  const searchBg = bgc("ui.selection.search", sc, pal, "#3a2a14");

  const cursorBg = bgc("ui.cursor.primary", sc, pal, bgc("ui.cursor", sc, pal, "#888888"));
  const cursorlineBg = bgc("ui.cursorline.primary", sc, pal, bgc("ui.cursorline", sc, pal, "transparent"));

  function tagStyle(tag) {
    if (!tag) return null;
    if (tag === "cursor") {
      const c = fullStyle("ui.cursor.primary", sc, pal);
      return { fg: c?.fg ?? FG, bg: c?.bg ?? cursorBg };
    }
    if (tag === "sel") {
      const c = fullStyle("ui.selection.primary", sc, pal);
      return { fg: c?.fg ?? FG, bg: c?.bg ?? selPrimBg };
    }
    if (tag === "sel2") {
      const c = fullStyle("ui.selection", sc, pal);
      return { fg: c?.fg ?? FG, bg: c?.bg ?? selSecBg };
    }
    if (tag === "match") {
      const m = fullStyle("ui.cursor.match", sc, pal);
      return { fg: m?.fg, bg: m?.bg, mods: m?.mods ?? [] };
    }
    if (tag === "search") return { fg: searchFg, bg: searchBg };
    return null;
  }

  return (
    <div style={{ background: BG, borderRadius: 8, overflow: "hidden", border: "1px solid " + C.border, fontFamily: MONO, fontSize: 13, lineHeight: "20px", boxShadow: "0 8px 32px rgba(0,0,0,0.4)" }}>
      <div style={{ display: "flex", alignItems: "center", padding: "6px 14px", borderBottom: "1px solid " + C.border, background: C.bgChrome, gap: 8 }}>
        <div style={{ display: "flex", gap: 6 }}>
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "#ff5f56" }} />
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "#ffbd2e" }} />
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "#27c93f" }} />
        </div>
        <span style={{ color: C.textDimmer, fontSize: 11 }}>theme.rs</span>
      </div>
      <div style={{ padding: "4px 0", overflowX: "auto" }}>
        {CODE.map(line => {
          const isCur = line.n === 5;
          return (
            <div key={line.n} style={{ display: "flex", padding: "0 12px 0 0", background: isCur ? cursorlineBg : "transparent", minHeight: 20 }}>
              <span style={{ display: "inline-block", width: 44, textAlign: "right", paddingRight: 12, color: isCur ? lnrS : lnr, userSelect: "none", flexShrink: 0, fontSize: 12 }}>{line.n}</span>
              <span style={{ whiteSpace: "pre" }}>
                {line.t.length === 0 && <span>{" "}</span>}
                {line.t.map((tok, i) => (
                  <span key={i} style={tokenStyle(tok[1], sc, pal, FG, BG, tagStyle(tok[2]))}>{tok[0]}</span>
                ))}
              </span>
            </div>
          );
        })}
      </div>
      <div style={{ display: "flex", alignItems: "center", background: statusBg, borderTop: "1px solid " + brd, fontSize: 11, fontFamily: MONO }}>
        <span style={{ padding: "3px 10px", color: rowFg }}>5:1 theme.rs [rust] [+]</span>
        <span style={{ flex: 1 }} />
        <span style={{ padding: "3px 10px", color: sepFg }}>{"│"}</span>
        <span style={{ padding: "3px 10px", color: rowFg, fontWeight: 700 }}>{mode.label}</span>
      </div>
      <div style={{ display: "flex", gap: 4, padding: "6px 10px", background: C.bgChrome, borderTop: "1px solid " + C.border }}>
        {MODES.map((m, i) => (
          <button key={m.scope} onClick={() => setModeIdx(i)}
            style={{
              background: i === modeIdx ? C.accent : C.bgBtn,
              color: i === modeIdx ? C.bg : C.textMuted,
              border: "none", borderRadius: 10, padding: "2px 9px",
              fontSize: 10, fontFamily: MONO, cursor: "pointer",
              fontWeight: i === modeIdx ? 600 : 400,
            }}
            title={"Preview " + m.scope}
          >
            {m.label}
          </button>
        ))}
      </div>
    </div>
  );
}
