// tools/theme-editor/src/lib/vocabulary.generated.js — GENERATED, do not hand-edit.
//
// The theme loader's own vocabulary (hume-engine/src/theme/loader/), emitted
// so the theme editor can offer only names HUME actually accepts, and reject
// nothing HUME would. Regenerate after any change to the loader's modifier,
// underline, ANSI-colour, style-key, or cursor-ladder vocabulary:
//
//   HUME_WRITE_THEME_VOCABULARY=1 cargo test -p hume-engine theme_vocabulary_js_matches_loader
//
// hume-engine/src/theme/loader/vocabulary.rs's drift test fails the build if
// this file falls out of sync.

export const ANSI_COLORS = {
  "black": "#000000",
  "red": "#cd0000",
  "green": "#00cd00",
  "yellow": "#cdcd00",
  "blue": "#0000ee",
  "magenta": "#cd00cd",
  "cyan": "#00cdcd",
  "light-gray": "#e5e5e5",
  "gray": "#7f7f7f",
  "light-red": "#ff0000",
  "light-green": "#00ff00",
  "light-yellow": "#ffff00",
  "light-blue": "#5c5cff",
  "light-magenta": "#ff00ff",
  "light-cyan": "#00ffff",
  "white": "#ffffff",
};

export const MODIFIER_NAMES = ["bold", "italic", "crossed_out", "dim", "reversed", "hidden", "slow_blink", "rapid_blink"];

export const UNDERLINE_MODIFIER = "underlined";

export const UNDERLINE_NAMES = ["line", "curl", "dotted", "dashed", "double_line"];

export const STYLE_KEYS = ["fg", "bg", "underline", "modifiers"];

export const CURSOR_LADDERS = {
  "normal": {
    secondary: ["ui.cursor.normal", "ui.cursor", "ui.selection"],
    primary: ["ui.cursor.primary.normal", "ui.cursor.primary", "ui.cursor", "ui", "ui.selection"],
  },
  "insert": {
    secondary: ["ui.cursor.insert", "ui.cursor", "ui.selection"],
    primary: ["ui.cursor.primary.insert", "ui.cursor.primary", "ui.cursor", "ui", "ui.selection"],
  },
  "select": {
    secondary: ["ui.cursor.select", "ui.cursor", "ui.selection"],
    primary: ["ui.cursor.primary.select", "ui.cursor.primary", "ui.cursor", "ui", "ui.selection"],
  },
};
