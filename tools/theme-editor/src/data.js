// The scope catalog tracks what HUME resolves by fixed name (see
// hume-engine/src/theme/mod.rs's `compute_ui`, the decoration/statusline/
// message-log call sites, and runtime/themes/sand.toml, the reference theme
// these categories are drawn from). Kept in sync by hand: nothing here is
// checked against the Rust sources, so a scope renamed there has to be
// renamed here too. Syntax/markup are tree-sitter capture names HUME has no
// fixed enum for — sand.toml is the de-facto catalog.
export const SCOPES = [
  ["UI", [
    "ui.background", "ui.text", "ui.text.focus",
    "ui.selection", "ui.selection.primary", "ui.linenr", "ui.linenr.selected",
    "ui.statusline", "ui.statusline.normal", "ui.statusline.insert",
    "ui.statusline.select", "ui.statusline.search", "ui.statusline.command",
    "ui.statusline.filter", "ui.statusline.separator",
    "ui.popup", "ui.popup.scroll", "ui.menu", "ui.menu.selected", "ui.menu.scroll",
    "ui.window", "ui.window.focused", "ui.drawer",
    "ui.cursorline", "ui.cursorline.primary",
  ]],
  ["Cursor", [
    "ui.cursor", "ui.cursor.normal", "ui.cursor.insert", "ui.cursor.select",
    "ui.cursor.primary", "ui.cursor.primary.normal", "ui.cursor.primary.insert",
    "ui.cursor.primary.select", "ui.cursor.match", "ui.cursor.match.search",
  ]],
  ["Keywords", [
    "keyword", "keyword.control", "keyword.control.conditional",
    "keyword.control.repeat", "keyword.control.import", "keyword.control.return",
    "keyword.control.exception", "keyword.operator", "keyword.directive",
    "keyword.function", "keyword.storage", "keyword.storage.type",
    "keyword.storage.modifier", "operator",
  ]],
  ["Types", [
    "type", "type.builtin", "type.parameter", "type.enum", "type.enum.variant",
    "constructor", "tag", "tag.builtin", "tag.attribute", "attribute",
    "namespace", "module",
  ]],
  ["Literals", [
    "constant", "constant.builtin", "constant.builtin.boolean",
    "constant.character", "constant.character.escape", "constant.numeric",
    "constant.numeric.integer", "constant.numeric.float",
    "string", "string.regexp", "string.special", "string.special.symbol",
    "string.special.path", "string.special.url",
    "comment", "comment.line", "comment.block", "comment.block.documentation",
  ]],
  ["Names", [
    "variable", "variable.builtin", "variable.parameter", "variable.other",
    "variable.other.member", "label",
    "punctuation", "punctuation.bracket", "punctuation.delimiter", "punctuation.special",
    "function", "function.builtin", "function.method", "function.macro", "function.special",
    "special",
  ]],
  ["Markup", [
    "markup.heading", "markup.heading.1", "markup.heading.2", "markup.heading.3",
    "markup.heading.4", "markup.heading.5", "markup.heading.6", "markup.heading.marker",
    "markup.bold", "markup.italic", "markup.strikethrough",
    "markup.link", "markup.link.url", "markup.link.text", "markup.link.label",
    "markup.quote", "markup.raw", "markup.raw.block", "markup.raw.inline",
    "markup.list", "markup.list.checked", "markup.list.unchecked",
  ]],
  ["Diff", ["diff.plus", "diff.minus", "diff.delta",
            "diff.plus.gutter", "diff.minus.gutter", "diff.delta.gutter",
            "diff.plus.word", "diff.minus.word"]],
  // HUME reads these; ui.virtual.ruler/.wrap/.jump-label and the per-kind
  // ui.virtual.inlay-hint.parameter/.type are Helix scopes HUME doesn't
  // read yet, so they're deliberately left out here — see the Themes
  // section of the user manual's Configuration page.
  ["Virtual", ["ui.virtual", "ui.virtual.indent-guide", "ui.virtual.whitespace",
               "ui.virtual.inlay-hint", "ui.virtual.invisible"]],
  ["Diagnostic", [
    "diagnostic.error", "diagnostic.warning", "diagnostic.info", "diagnostic.hint",
    "diagnostic.error.message", "diagnostic.warning.message",
    "diagnostic.info.message", "diagnostic.hint.message",
    "diagnostic.error.message-text", "diagnostic.warning.message-text",
    "diagnostic.info.message-text", "diagnostic.hint.message-text",
    // End-of-line summary, one scope per severity — same reason as the
    // gutter names below: virtual text past the end of the line must not
    // inherit the text-span squiggle's underline.
    "error.diagnostic.inline", "warning.diagnostic.inline",
    "info.diagnostic.inline", "hint.diagnostic.inline",
    // Gutter counterparts of the four "diagnostic.*" scopes above — the
    // sign column reads these bare names so a gutter glyph never inherits
    // the text-span squiggle's underline.
    "error", "warning", "info", "hint",
  ]],
];

export const ALL_SCOPES = SCOPES.flatMap(([, items]) => items);

export const DEFAULT_PAL = {
  black: "#1a1b26", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
  blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#c0caf5",
  orange: "#ff9e64", gray: "#565f89", "light-gray": "#a9b1d6", "dark-gray": "#33374c",
};

export const DEFAULT_SC = {
  "ui.background": { bg: "black" }, "ui.text": "white", "ui.cursor": { fg: "black", bg: "blue" },
  "ui.text.focus": { fg: "black", bg: "blue" },
  "ui.cursor.match": { fg: "yellow", bg: "#3a371a", modifiers: ["bold"] }, "ui.selection": { bg: "dark-gray" },
  "ui.selection.primary": { bg: "gray" }, "ui.cursor.match.search": { fg: "orange", bg: "#3a2a14" }, "ui.linenr": "gray",
  "ui.linenr.selected": "yellow",
  "ui.statusline": { fg: "white", bg: "dark-gray" },
  "ui.statusline.normal": { fg: "black", bg: "blue" },
  "ui.statusline.insert": { fg: "black", bg: "green" },
  "ui.statusline.select": { fg: "black", bg: "yellow" },
  "ui.statusline.search": { fg: "black", bg: "magenta" },
  "ui.statusline.command": { fg: "black", bg: "cyan" },
  "ui.statusline.filter": { fg: "black", bg: "orange" },
  "ui.statusline.separator": "gray",
  "ui.popup": { fg: "white", bg: "dark-gray" },
  "ui.popup.scroll": { fg: "yellow", bg: "dark-gray" },
  "ui.menu": { fg: "white", bg: "dark-gray" },
  "ui.menu.selected": { fg: "black", bg: "blue" },
  "ui.menu.scroll": { fg: "yellow", bg: "dark-gray" },
  "ui.drawer": { fg: "white", bg: "dark-gray" },
  "ui.window": "gray", "ui.window.focused": "orange",
  "ui.cursorline": { bg: "dark-gray" }, "ui.cursorline.primary": { bg: "dark-gray" },
  "ui.virtual": "gray", "ui.virtual.indent-guide": "gray",
  "ui.virtual.whitespace": "gray", "ui.virtual.inlay-hint": "gray",
  // Deliberately loud, not muted like its Virtual siblings above — this is
  // the stand-in for a character the terminal must never show as itself
  // (a bidi override among them), and it has to catch the eye.
  "ui.virtual.invisible": { fg: "black", bg: "red" },
  keyword: "magenta", "keyword.control": "magenta",
  "keyword.control.conditional": "magenta", "keyword.control.repeat": "magenta",
  "keyword.control.import": "magenta", "keyword.control.return": "magenta",
  "keyword.control.exception": "magenta", "keyword.operator": "cyan",
  "keyword.directive": "magenta", "keyword.function": "magenta",
  "keyword.storage": "magenta", "keyword.storage.type": "magenta",
  "keyword.storage.modifier": "magenta", operator: "cyan",
  type: "yellow", "type.builtin": "yellow", "type.parameter": "yellow",
  "type.enum": "yellow", "type.enum.variant": "yellow", constructor: "yellow",
  tag: "red", "tag.builtin": "red", "tag.attribute": "yellow", attribute: "yellow",
  namespace: "magenta", module: "magenta",
  constant: "orange", "constant.builtin": { fg: "orange", modifiers: ["bold"] },
  "constant.builtin.boolean": { fg: "orange", modifiers: ["bold"] },
  "constant.character": "orange", "constant.character.escape": "magenta",
  "constant.numeric": "orange", "constant.numeric.integer": "orange",
  "constant.numeric.float": "orange",
  string: "green", "string.regexp": "cyan", "string.special": "cyan",
  "string.special.symbol": "cyan", "string.special.path": "cyan",
  "string.special.url": { fg: "blue", modifiers: ["underlined"] },
  comment: { fg: "gray", modifiers: ["italic"] },
  "comment.line": { fg: "gray", modifiers: ["italic"] },
  "comment.block": { fg: "gray", modifiers: ["italic"] },
  "comment.block.documentation": { fg: "gray", modifiers: ["italic"] },
  variable: "white", "variable.builtin": "red", "variable.parameter": "orange",
  "variable.other": "white", "variable.other.member": "white",
  label: "cyan", punctuation: "light-gray",
  "punctuation.bracket": "light-gray", "punctuation.delimiter": "light-gray",
  "punctuation.special": "yellow",
  "function": "blue", "function.builtin": "blue", "function.method": "blue",
  "function.macro": "cyan", "function.special": "cyan", special: "yellow",
  "markup.heading": { fg: "blue", modifiers: ["bold"] },
  "markup.heading.1": { fg: "blue", modifiers: ["bold"] },
  "markup.heading.2": { fg: "blue", modifiers: ["bold"] },
  "markup.heading.3": "blue", "markup.heading.4": "blue", "markup.heading.5": "blue",
  "markup.heading.6": "gray", "markup.heading.marker": "gray",
  "markup.bold": { fg: "orange", modifiers: ["bold"] },
  "markup.italic": { fg: "magenta", modifiers: ["italic"] },
  "markup.strikethrough": { modifiers: ["crossed_out"] },
  "markup.link": { fg: "cyan", modifiers: ["underlined"] },
  "markup.link.url": { fg: "cyan", modifiers: ["underlined"] },
  "markup.link.text": "magenta", "markup.link.label": "magenta",
  "markup.quote": { fg: "gray", modifiers: ["italic"] },
  "markup.raw": "magenta", "markup.raw.block": "white", "markup.raw.inline": "magenta",
  "markup.list": "red", "markup.list.checked": "green", "markup.list.unchecked": "gray",
  "diff.plus": "green", "diff.minus": "red", "diff.delta": "yellow",
  "diff.plus.gutter": "green", "diff.minus.gutter": "red", "diff.delta.gutter": "cyan",
  "diff.plus.word": { fg: "white", bg: "green", modifiers: ["bold"] },
  "diff.minus.word": { fg: "white", bg: "red", modifiers: ["bold", "crossed_out"] },
  "diagnostic.error": "red", "diagnostic.warning": "yellow",
  "diagnostic.info": "blue", "diagnostic.hint": "cyan",
  "diagnostic.error.message": { fg: "black", bg: "red" },
  "diagnostic.warning.message": { fg: "black", bg: "yellow" },
  "diagnostic.info.message": { fg: "black", bg: "blue" },
  "diagnostic.hint.message": { fg: "black", bg: "cyan" },
  "diagnostic.error.message-text": "red",
  "diagnostic.warning.message-text": "yellow",
  "diagnostic.info.message-text": "blue",
  "diagnostic.hint.message-text": "cyan",
  "error.diagnostic.inline": "red",
  "warning.diagnostic.inline": "yellow",
  "info.diagnostic.inline": "blue",
  "hint.diagnostic.inline": "cyan",
};
