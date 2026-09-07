import { C, MONO } from '../ui.js';
import { fgc, bgc, tokenStyle } from '../lib/theme.js';

// The `:messages` log — matches `hume-editor/src/editor/message_log.rs`'s
// severity -> scope mapping: a `diagnostic.<sev>.message` badge next to
// `diagnostic.<sev>.message-text` body text. Distinct from `EditorPane`'s
// gutter signs (bare `error`/`warning`/`info`/`hint`) and inline diagnostic
// underlines (`diagnostic.<sev>`) — this is the log, not the buffer.
const ENTRIES = [
  { sev: "error", label: "ERR", text: "failed to load runtime/themes/sand.toml: BadUnderline" },
  { sev: "warning", label: "WARN", text: "lsp server 'rust-analyzer' exited unexpectedly" },
  { sev: "info", label: "INFO", text: "wrote runtime/themes/sand.toml" },
  { sev: "hint", label: "HINT", text: "3 unsaved buffers" },
];

export default function MessagesPane({ pal, sc }) {
  const BG = bgc("ui.background", sc, pal, "#1a1b26");
  const FG = fgc("ui.text", sc, pal, "#c0caf5");

  return (
    <div style={{ borderRadius: 6, overflow: "hidden", border: "1px solid " + C.border, fontFamily: MONO, fontSize: 12, background: BG }}>
      <div style={{ padding: "4px 12px", background: C.bgChrome, color: C.textDimmer, fontSize: 10, borderBottom: "1px solid " + C.border }}>{":messages"}</div>
      {ENTRIES.map(e => (
        <div key={e.sev} style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 12px" }}>
          <span style={{ ...tokenStyle("diagnostic." + e.sev + ".message", sc, pal, FG, BG, null), borderRadius: 3, padding: "1px 6px", fontSize: 10, fontWeight: 700 }}>
            {e.label}
          </span>
          <span style={tokenStyle("diagnostic." + e.sev + ".message-text", sc, pal, FG, BG, null)}>{e.text}</span>
        </div>
      ))}
    </div>
  );
}
