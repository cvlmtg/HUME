import { C, MONO } from '../ui.js';

export default function Acc({ title, open, onToggle, count, children }) {
  return (
    <div style={{ borderBottom: "1px solid " + C.border }}>
      <button onClick={onToggle} style={{
        width: "100%", display: "flex", alignItems: "center", justifyContent: "space-between",
        padding: "10px 14px", background: open ? C.bgRow : "transparent", border: "none",
        cursor: "pointer", color: C.textSoft, fontSize: 12, fontFamily: MONO,
        fontWeight: 600, letterSpacing: "0.05em", textTransform: "uppercase",
      }}>
        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ display: "inline-block", transform: open ? "rotate(90deg)" : "rotate(0)", transition: "transform 0.2s", fontSize: 10 }}>
            {"▶"}
          </span>
          {title}
        </span>
        {count != null && (
          <span style={{ background: C.surface, color: C.textMuted, borderRadius: 8, padding: "1px 7px", fontSize: 10 }}>{count}</span>
        )}
      </button>
      {open && (
        <div style={{ padding: "6px 10px 10px", background: C.bgAccordion, maxHeight: "50vh", overflowY: "auto" }}>
          {children}
        </div>
      )}
    </div>
  );
}
