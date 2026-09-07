import { useState } from 'react';
import { C, MONO, pill } from '../ui.js';
import { fgc, bgc, fullStyle, cursorColors, diagnosticStyle, tokenStyle } from '../lib/theme.js';
import { BUFFERS, DIFF_SAMPLE, MODES, OVERLAYS, PICKER_ROWS, DRAWER_ROWS, NEIGHBOR_TOP, NEIGHBOR_BOTTOM } from './samples.js';

// Floor under the pane's content height so an overlay (the picker's centered
// panel, the drawer's docked band) always has room to render — the sample
// buffers range from 5 rows (the diff) to ~50, but a terminal pane doesn't
// shrink to fit its content. Not a claim about any real terminal row count.
const PANE_MIN_H = 440;

// One overlay box (completion menu or hover popup) drawn on top of the
// buffer — mirrors `menu_box.rs`'s pairing of a root scope with its
// `.selected`/`.scroll` leaves. The picker and drawer are separate real
// render paths (`picker_panel.rs`, `drawer.rs`) with their own geometry —
// see `PickerPanel`/`DrawerBand` below, not this shared box.
function OverlayBox({ kind, sc, pal }) {
  const boxStyle = (root, selected, scroll) => ({
    bg: bgc(root, sc, pal, "#33374c"),
    fg: fgc(root, sc, pal, "#c0caf5"),
    selBg: selected ? bgc(selected, sc, pal, "#7aa2f7") : null,
    selFg: selected ? fgc(selected, sc, pal, "#1a1b26") : null,
    scroll: fgc(scroll, sc, pal, "#e0af68"),
  });

  let title, rows, style;
  if (kind === "menu") {
    title = "Completion";
    rows = ["build_default_theme", "bake_if_stale", "background"];
    style = boxStyle("ui.menu", "ui.menu.selected", "ui.menu.scroll");
  } else {
    title = "Hover";
    rows = ["fn bake(&mut self, registry: &ScopeRegistry)", "Pre-resolve every provider scope."];
    style = boxStyle("ui.popup", null, "ui.popup.scroll");
  }

  return (
    <div style={{
      position: "absolute", top: 24, right: 16, width: 220, borderRadius: 6,
      background: style.bg, color: style.fg, boxShadow: "0 6px 20px rgba(0,0,0,0.45)",
      overflow: "hidden", fontSize: 11, zIndex: 2,
    }}>
      <div style={{ padding: "4px 10px", opacity: 0.7, fontSize: 10 }}>{title}</div>
      {rows.map((r, i) => (
        <div key={r} style={{
          padding: "3px 10px",
          background: i === 0 && style.selBg ? style.selBg : "transparent",
          color: i === 0 && style.selFg ? style.selFg : style.fg,
        }}>
          {r}
        </div>
      ))}
      {style.scroll && (
        <div style={{ position: "absolute", top: 4, right: 2, width: 3, height: 24, borderRadius: 2, background: style.scroll }} />
      )}
    </div>
  );
}

// The fuzzy picker — mirrors `panel_geometry` + `draw_picker_panel`
// (`picker_panel.rs`): centered over the pane at `80% x 60%` (clamped to
// 100 cols / 30 rows in the real terminal grid; approximated here as a
// pixel clamp), a square 1-cell border in `ui.text` (no drop shadow — a
// terminal has none), a prompt/query input row with a block cursor and a
// right-aligned `matched/total` counter, and a full-width `ui.text.focus`
// fill on the selected row. Unlike `OverlayBox`'s menu/popup, the picker
// draws no title and no scrollbar thumb — the real panel has neither.
function PickerPanel({ sc, pal }) {
  const bg = bgc("ui.background", sc, pal, "#1a1b26");
  const fg = fgc("ui.text", sc, pal, "#c0caf5");
  const selBg = bgc("ui.text.focus", sc, pal, "#7aa2f7");
  const selFg = fgc("ui.text.focus", sc, pal, "#1a1b26");
  const cursorBg = bgc("ui.cursor.primary", sc, pal, fg);
  const cursorFg = fgc("ui.cursor.primary", sc, pal, bg);

  return (
    <div style={{
      position: "absolute", inset: 0, display: "flex",
      alignItems: "center", justifyContent: "center", zIndex: 2,
    }}>
      <div style={{
        width: "80%", maxWidth: 780, height: "60%", maxHeight: 600,
        background: bg, color: fg, border: "1px solid " + fg,
        display: "flex", flexDirection: "column", overflow: "hidden", fontSize: 13,
      }}>
        <div style={{ display: "flex", alignItems: "center", padding: "0 6px", minHeight: 20 }}>
          <span>{"> theme"}</span>
          <span style={{ background: cursorBg, color: cursorFg }}>{" "}</span>
          <span style={{ flex: 1 }} />
          <span>{"12/340"}</span>
        </div>
        {PICKER_ROWS.map((r, i) => (
          <div key={r} style={{
            padding: "0 6px", minHeight: 20,
            background: i === 0 ? selBg : "transparent",
            color: i === 0 ? selFg : fg,
          }}>
            {r}
          </div>
        ))}
      </div>
    </div>
  );
}

// The drawer — mirrors `DrawerWidget::render` (`drawer.rs`): a full-width
// band docked directly above the statusline (rendered here as the pane's
// last flex child, so it shrinks the buffer area above it rather than
// floating over it), filled `ui.drawer` with no border and no title, a
// blank row 0 (the real drawer's only separation from the pane above), and
// a full-width `ui.menu.selected` fill on the selected row.
function DrawerBand({ sc, pal }) {
  const bg = bgc("ui.drawer", sc, pal, "#33374c");
  const fg = fgc("ui.drawer", sc, pal, "#c0caf5");
  const selBg = bgc("ui.menu.selected", sc, pal, "#7aa2f7");
  const selFg = fgc("ui.menu.selected", sc, pal, "#1a1b26");

  return (
    <div style={{ background: bg, color: fg, fontSize: 13, flexShrink: 0 }}>
      <div style={{ minHeight: 20 }} />
      {DRAWER_ROWS.map((r, i) => (
        <div key={r} style={{
          padding: "0 12px", minHeight: 20, whiteSpace: "pre",
          background: i === 0 ? selBg : "transparent",
          color: i === 0 ? selFg : fg,
        }}>
          {r}
        </div>
      ))}
    </div>
  );
}

// Render one `[text, scope, tag?]` token, handling the one tag shape
// `tokenStyle` can't express on its own: a bar-cursor head with no theme
// entry for its chain, where the real terminal cursor would show through.
// Approximated with a thin foreground-colored caret rather than a filled
// cell. Shared by the plain-buffer lines and `DiffRows` so the two don't
// drift on how a head/selection tag turns into a rendered span.
function renderToken(tok, i, tag, sc, pal, fallbackFg, editorBg) {
  if (tag?.bar) {
    return (
      <span key={i}>
        <span style={{ borderLeft: "2px solid " + tag.fg }} />
        <span style={tokenStyle(tok[1], sc, pal, fallbackFg, editorBg, null)}>{tok[0]}</span>
      </span>
    );
  }
  return <span key={i} style={tokenStyle(tok[1], sc, pal, fallbackFg, editorBg, tag)}>{tok[0]}</span>;
}

// The diff buffer: a git-diff-shaped view. Each changed row gets a full-row
// `diff.plus`/`.minus`/`.delta` background and a sign-column glyph from the
// matching `.gutter` scope; the row's own word-level change is a nested
// `diff.plus.word`/`.minus.word` span — same layering as
// `runtime/plugins/core/git-diff/render.scm`.
function DiffRows({ sc, pal, BG, FG, lnr, tagStyle }) {
  return DIFF_SAMPLE.rows.map(row => {
    const rowBg = row.rowScope ? bgc(row.rowScope, sc, pal, BG) : BG;
    const rowFg = row.rowScope ? fgc(row.rowScope, sc, pal, FG) : FG;
    return (
      <div key={row.n} style={{ display: "flex", background: rowBg, minHeight: 20 }}>
        <span style={{ width: 16, textAlign: "center", color: row.sign ? fgc(row.sign.scope, sc, pal, rowFg) : "transparent", flexShrink: 0, fontSize: 12 }}>
          {row.sign ? row.sign.glyph : "·"}
        </span>
        <span style={{ display: "inline-block", width: 32, textAlign: "right", paddingRight: 12, color: lnr, userSelect: "none", flexShrink: 0, fontSize: 12 }}>{row.n}</span>
        <span style={{ whiteSpace: "pre", color: rowFg }}>
          {row.t.map((tok, i) => renderToken(tok, i, tagStyle(tok[2]), sc, pal, rowFg, rowBg))}
        </span>
      </div>
    );
  });
}

// One pane in the neighbor column — its own line-number gutter (gutters are
// per-pane, hume-engine/src/pane.rs) and a few syntax-highlighted lines.
// Dimmed via CSS opacity: a 0.5 lerp of fg/bg toward `ui.background`
// (`PANE_DIM_FACTOR`, hume-engine/src/pipeline/mod.rs:200-202) is exactly
// what 50% opacity over an `ui.background`-filled ancestor already produces.
// Every unfocused pane is forced to Normal mode regardless of the mode
// buttons above (hume-editor/src/editor/frame.rs:64-68), so its head always
// takes the plain block-cursor chain, never the active mode's.
function NeighborPane({ lines, sc, pal, BG, FG, lnr, virtualFg }) {
  const headStyle = cursorColors("normal", true, sc, pal);
  return (
    <div style={{ flex: 1, minHeight: 0, opacity: 0.5, overflow: "hidden", padding: "4px 0" }}>
      {lines.map(line => (
        <div key={line.n} style={{ display: "flex", padding: "0 8px", minHeight: 20 }}>
          <span style={{ display: "inline-block", width: 18, textAlign: "right", paddingRight: 8, color: lnr, userSelect: "none", flexShrink: 0, fontSize: 12 }}>{line.n}</span>
          <span style={{ whiteSpace: "pre" }}>
            {line.t.map((tok, i) => (
              <span key={i} style={tokenStyle(tok[1], sc, pal, FG, BG, tok[2] === "head" ? headStyle : null)}>{tok[0]}</span>
            ))}
          </span>
        </div>
      ))}
      <div style={{ padding: "0 8px", color: virtualFg }}>{"~"}</div>
    </div>
  );
}

// The seam between the focused pane and the neighbor column — one character
// wide, a `│` run rather than a filled bar (`draw_box_border`'s vertical
// edge is exactly this: one glyph per row). In a plain two/three-pane split
// the seam is adjacent to the focused pane along its whole height, so it's
// `ui.window.focused` top to bottom; the `├` marks where the neighbor
// column's own horizontal seam (below) meets it.
function VSeam({ fg, bg }) {
  const half = Array.from({ length: 11 });
  return (
    <div style={{ width: "1ch", flexShrink: 0, display: "flex", flexDirection: "column", background: bg, color: fg, fontSize: 13, lineHeight: "20px", overflow: "hidden" }}>
      <div style={{ flex: 1, overflow: "hidden" }}>
        {half.map((_, i) => <div key={i}>{"│"}</div>)}
      </div>
      <div>{"├"}</div>
      <div style={{ flex: 1, overflow: "hidden" }}>
        {half.map((_, i) => <div key={i}>{"│"}</div>)}
      </div>
    </div>
  );
}

// The seam between the two stacked neighbor panes — a `─` run in `ui.window`
// (muted), not `ui.window.focused`: it isn't adjacent to the focused pane,
// so `focused_seam_segment` (hume-engine/src/pipeline/layout.rs:481-505)
// never lights it up. This is the only place in the preview `ui.window`
// (as opposed to `ui.window.focused`) is actually demonstrated.
function HSeam({ fg, bg }) {
  return (
    <div style={{ minHeight: 20, background: bg, color: fg, whiteSpace: "pre", overflow: "hidden" }}>
      {"─".repeat(60)}
    </div>
  );
}

export default function EditorPane({ pal, sc }) {
  const [bufIdx, setBufIdx] = useState(0);
  const [modeIdx, setModeIdx] = useState(0);
  const [overlay, setOverlay] = useState("none");
  const mode = MODES[modeIdx];
  const buf = BUFFERS[bufIdx];
  const isDiff = buf === DIFF_SAMPLE;

  const BG = bgc("ui.background", sc, pal, "#1a1b26");
  const FG = fgc("ui.text", sc, pal, "#c0caf5");
  const lnr = fgc("ui.linenr", sc, pal, "#565f89");
  const lnrS = fgc("ui.linenr.selected", sc, pal, "#e0af68");
  const rowFg = fgc(mode.scope, sc, pal, fgc("ui.statusline", sc, pal, "#c0caf5"));
  const rowBg = bgc(mode.scope, sc, pal, bgc("ui.statusline", sc, pal, "#33374c"));
  const sepFg = fgc("ui.statusline.separator", sc, pal, rowFg);
  const brdFocused = fgc("ui.window.focused", sc, pal, "#7aa2f7");
  const brd = fgc("ui.window", sc, pal, "#565f89");
  const statusBg = rowBg !== "transparent" ? rowBg : BG;

  // `isCur` below only ever matches when `buf.cursorLine` names a real line,
  // so no separate guard is needed here for buffers that omit it.
  const cursorlineBg = bgc("ui.cursorline.primary", sc, pal, "transparent");
  const indentGuideFg = fgc("ui.virtual.indent-guide", sc, pal, "#565f89");
  const whitespaceFg = fgc("ui.virtual.whitespace", sc, pal, "#565f89");
  const virtualFg = fgc("ui.virtual", sc, pal, "#565f89");

  // Returns the tagged scope's own style (fg/bg/mods), or null for "nothing
  // to layer" — `tokenStyle`'s `tag?.fg ?? s?.fg ?? fallbackFg` already
  // resolves that exactly like `ResolvedStyle::layer` does: an unset field
  // inherits what's underneath rather than being overridden.
  // `d` (the current line's own diagnostic, when it has one) is only ever
  // needed for the "diag" tag — every other caller omits it.
  function tagStyle(tag, d) {
    if (!tag) return null;
    if (tag === "cursor" || tag === "cursor2") {
      const primary = tag === "cursor";
      // `barPrimary` (Insert only, HUME's default `cursor-shape-insert`) means
      // the primary head has no configured Block shape: the real terminal bar
      // is the sole indicator, so nothing from the theme is layered over it.
      // A secondary head has no real terminal cursor to fall back on, so
      // HUME always paints it regardless of shape — matching Helix's own
      // unconditional secondary-cursor painting.
      if (primary && mode.barPrimary) {
        return { fg: FG, bg: null, bar: true };
      }
      return cursorColors(mode.chain, primary, sc, pal);
    }
    if (tag === "sel") return fullStyle("ui.selection.primary", sc, pal);
    if (tag === "sel2") return fullStyle("ui.selection", sc, pal);
    if (tag === "match") return fullStyle("ui.cursor.match", sc, pal);
    if (tag === "search") return fullStyle("ui.cursor.match.search", sc, pal);
    if (tag === "diag") return d ? diagnosticStyle(d.sev, sc, pal) : null;
    return null;
  }

  const diag = buf.diagnostics ?? [];

  return (
    <div style={{ background: BG, borderRadius: 8, overflow: "hidden", border: "1px solid " + C.border, fontFamily: MONO, fontSize: 13, lineHeight: "20px", boxShadow: "0 8px 32px rgba(0,0,0,0.4)" }}>
      <div style={{ display: "flex", alignItems: "center", padding: "6px 14px", borderBottom: "1px solid " + C.border, background: C.bgChrome, gap: 8 }}>
        <div style={{ display: "flex", gap: 6 }}>
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "#ff5f56" }} />
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "#ffbd2e" }} />
          <span style={{ width: 10, height: 10, borderRadius: "50%", background: "#27c93f" }} />
        </div>
        <span style={{ color: C.textDimmer, fontSize: 11 }}>{buf.name}</span>
      </div>

      {/* PANE_MIN_H floors this column, not the pane root, so the drawer
          band below can shrink it (as `EngineView::pane_area` shrinks the
          real viewport) instead of growing the frame. */}
      <div style={{ minHeight: PANE_MIN_H, display: "flex", flexDirection: "column" }}>
        <div style={{ flex: 1, minHeight: 0, display: "flex", position: "relative" }}>
          <div style={{ flex: 1, padding: "4px 0", overflowX: "auto", overflowY: "hidden", position: "relative" }}>
            {(overlay === "menu" || overlay === "popup") && <OverlayBox kind={overlay} sc={sc} pal={pal} />}
            {overlay === "picker" && <PickerPanel sc={sc} pal={pal} />}

            {isDiff ? (
              <DiffRows sc={sc} pal={pal} BG={BG} FG={FG} lnr={lnr} tagStyle={tagStyle} />
            ) : (
              <>
                {buf.lines.map(line => {
                  const isCur = line.n === buf.cursorLine;
                  const d = diag.find(x => x.line === line.n);
                  return (
                    <div key={line.n} style={{ display: "flex", padding: "0 12px 0 0", background: isCur ? cursorlineBg : "transparent", minHeight: 20 }}>
                      <span style={{ width: 16, textAlign: "center", color: d ? fgc(d.sev, sc, pal, "#888") : "transparent", flexShrink: 0, fontSize: 12 }}>
                        {d ? "●" : ""}
                      </span>
                      <span style={{ display: "inline-block", width: 36, textAlign: "right", paddingRight: 12, color: isCur ? lnrS : lnr, userSelect: "none", flexShrink: 0, fontSize: 12 }}>{line.n}</span>
                      <span style={{ whiteSpace: "pre" }}>
                        {line.t.length === 0 && <span>{" "}</span>}
                        {line.t.map((tok, i) => renderToken(tok, i, tagStyle(tok[2], d), sc, pal, FG, BG))}
                      </span>
                      {d && (
                        <span style={{ marginLeft: 12, whiteSpace: "pre", ...tokenStyle(d.sev + ".diagnostic.inline", sc, pal, "#888", BG, null) }}>
                          {"■ " + d.msg}
                        </span>
                      )}
                    </div>
                  );
                })}
                {/* ui.virtual: end-of-buffer filler rows, Helix's tilde convention. */}
                {[0, 1].map(i => (
                  <div key={"eob" + i} style={{ padding: "0 12px", color: virtualFg }}>{"~"}</div>
                ))}
                {/* ui.virtual.indent-guide / ui.virtual.whitespace: the indicators
                    shown when indent-guides / whitespace rendering are on. */}
                <div style={{ padding: "0 12px", display: "flex", gap: 18, fontSize: 11 }}>
                  <span style={{ color: C.textDim }}>{"indent "}<span style={{ color: indentGuideFg }}>{"│ │ │"}</span></span>
                  <span style={{ color: C.textDim }}>{"space "}<span style={{ color: whitespaceFg }}>{"· · ·"}</span></span>
                </div>
              </>
            )}
          </div>

          <VSeam fg={brdFocused} bg={BG} />
          <div style={{ width: 200, flexShrink: 0, display: "flex", flexDirection: "column" }}>
            <NeighborPane lines={NEIGHBOR_TOP} sc={sc} pal={pal} BG={BG} FG={FG} lnr={lnr} virtualFg={virtualFg} />
            <HSeam fg={brd} bg={BG} />
            <NeighborPane lines={NEIGHBOR_BOTTOM} sc={sc} pal={pal} BG={BG} FG={FG} lnr={lnr} virtualFg={virtualFg} />
          </div>
        </div>
        {overlay === "drawer" && <DrawerBand sc={sc} pal={pal} />}
      </div>

      <div style={{ display: "flex", alignItems: "center", background: statusBg, borderTop: "1px solid " + brd, fontSize: 11, fontFamily: MONO }}>
        <span style={{ padding: "3px 10px", color: rowFg }}>{(buf.cursorLine ?? 1) + ":1 " + buf.name + " [+]"}</span>
        <span style={{ flex: 1 }} />
        <span style={{ padding: "3px 10px", color: sepFg }}>{"│"}</span>
        <span style={{ padding: "3px 10px", color: rowFg, fontWeight: 700 }}>{mode.label}</span>
      </div>

      <div style={{ display: "flex", gap: 4, padding: "6px 10px", background: C.bgChrome, borderTop: "1px solid " + C.border, flexWrap: "wrap" }}>
        {BUFFERS.map((b, i) => (
          <button key={b.name} onClick={() => setBufIdx(i)} style={pill(i === bufIdx)}>
            {b.name}
          </button>
        ))}
      </div>
      <div style={{ display: "flex", gap: 4, padding: "0 10px 6px", background: C.bgChrome, flexWrap: "wrap" }}>
        {MODES.map((m, i) => (
          <button key={m.scope} onClick={() => setModeIdx(i)} style={pill(i === modeIdx)}
            title={"Preview " + m.scope}
          >
            {m.label}
          </button>
        ))}
        <span style={{ width: 1, background: C.border, margin: "2px 4px" }} />
        {OVERLAYS.map(o => (
          <button key={o.key} onClick={() => setOverlay(o.key)} style={pill(overlay === o.key, { activeBg: C.brand })}>
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
