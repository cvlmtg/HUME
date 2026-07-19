export const MONO = "'JetBrains Mono','Fira Code',monospace";

export const C = {
  bg: "#11111b", bgHeader: "#181825", bgChrome: "#1a1b26",
  bgPanel: "#1e1e2e", bgRow: "#1e2030", bgAccordion: "#181926",
  bgInput: "#24273a", bgBtn: "#313244", surface: "#363a4f",
  borderHeader: "#45475a", border: "#2a2d3a",
  text: "#cdd6f4", textInput: "#cad3f5", textSoft: "#c8cdd8",
  textMuted: "#8087a2", textDim: "#5b6078", textDimmer: "#5c6086",
  accent: "#89b4fa", success: "#a6e3a1", brand: "#cba6f7",
};

export const INPUT = {
  background: C.bgInput, border: "1px solid " + C.surface, color: C.textInput,
  borderRadius: 4, padding: "3px 6px", fontSize: 11, fontFamily: MONO, outline: "none",
};

export const COLOR_PICKER = {
  width: 26, height: 22, padding: 0,
  border: "1px solid " + C.surface,
  borderRadius: 3, background: "transparent", cursor: "pointer", flexShrink: 0,
};
