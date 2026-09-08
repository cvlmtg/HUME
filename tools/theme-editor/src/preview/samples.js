// Preview content as plain data, so `tests/coverage.test.js` can walk every
// scope reference without importing JSX. Each buffer sample is a list of
// lines; each line is `{ n, t }` where `t` is a list of tokens.
// A token is `[text, scope]` (scope "" means unstyled) or
// `[text, scope, tag]`, where `tag` overlays a cursor/selection/match/diag
// highlight on top of the token's own scope — see `EditorPane.jsx`. `"diag"`
// marks the specific span a line's own diagnostic (below) targets, styled
// with its `diagnostic.<sev>` scope — the squiggle a real diagnostic draws
// under the offending code, distinct from the gutter dot and the
// end-of-line summary this pane also shows.

import { SCOPES } from '../data.js';

export const RUST_SAMPLE = {
  name: "theme.rs",
  cursorLine: 7,
  diagnostics: [
    { line: 16, sev: "warning", msg: "unused variable: `count`" },
    { line: 29, sev: "error", msg: "value moved here, in previous iteration of loop" },
  ],
  lines: [
    { n: 1, t: [["#[derive(", "punctuation.bracket"], ["Debug", "attribute"], [", ", "punctuation.delimiter"], ["Clone", "attribute"], [")]", "punctuation.bracket"]] },
    { n: 2, t: [["use ", "keyword.control.import"], ["std", "namespace"], ["::", "punctuation.delimiter"], ["collections", "module"], ["::", "punctuation.delimiter"], ["HashMap", "type"], [";", "punctuation.delimiter"]] },
    { n: 3, t: [] },
    { n: 4, t: [["/// A theme palette for the editor", "comment.block.documentation"]] },
    { n: 5, t: [["// see also: constant.rs", "comment.line"]] },
    { n: 6, t: [["pub ", "keyword.storage.modifier"], ["struct ", "keyword.storage.type"], ["Theme", "type", "search"], ["<", "punctuation.bracket"], ["T", "type.parameter"], [">", "punctuation.bracket"], [" {", "punctuation.bracket"]] },
    { n: 7, t: [["    ", ""], ["n", "variable", "cursor"], ["ame", "variable", "sel"], [":", "punctuation.delimiter"], [" String", "type.builtin"], [",", "punctuation.delimiter"]] },
    { n: 8, t: [["    ", ""], ["c", "variable", "cursor2"], ["olors", "variable"], [":", "punctuation.delimiter"], [" HashMap", "type"], ["<", "punctuation.bracket"], ["String", "type.builtin"], [", ", "punctuation.delimiter"], ["Color", "type"], [">", "punctuation.bracket"], [",", "punctuation.delimiter"]] },
    { n: 9, t: [["}", "punctuation.bracket"]] },
    { n: 10, t: [] },
    { n: 11, t: [["#[repr(", "punctuation.bracket"], ["u8", "type.builtin"], [")]", "punctuation.bracket"]] },
    { n: 12, t: [["enum ", "keyword.storage.type"], ["Kind", "type.enum"], [" { ", "punctuation.bracket"], ["Named", "type.enum.variant"], [", ", "punctuation.delimiter"], ["Indexed", "type.enum.variant"], [" }", "punctuation.bracket"]] },
    { n: 13, t: [] },
    { n: 14, t: [["impl", "keyword"], ["<", "punctuation.bracket"], ["T", "type.parameter"], [">", "punctuation.bracket"], [" Theme", "type", "search"], ["<", "punctuation.bracket"], ["T", "type.parameter"], [">", "punctuation.bracket"], [" {", "punctuation.bracket"]] },
    { n: 15, t: [["    pub ", "keyword.storage.modifier"], ["fn ", "keyword.function"], ["new", "function", "match"], ["(", "punctuation.bracket", "match"], ["name", "variable.parameter"], [": ", "punctuation.delimiter"], ["&", "operator"], ["str", "type.builtin"], [")", "punctuation.bracket", "match"], [" -> ", "operator"], ["Self", "constructor"], [" {", "punctuation.bracket"]] },
    { n: 16, t: [["        let ", "keyword"], ["mut ", "keyword.storage.modifier"], ["count", "variable", "diag"], [": u32", "ui.virtual.inlay-hint"], [" = ", "operator"], ["0x2A", "constant.numeric.integer"], [";", "punctuation.delimiter"]] },
    { n: 17, t: [["        let ", "keyword"], ["ratio", "variable"], [" = ", "operator"], ["4.2", "constant.numeric.float"], [";", "punctuation.delimiter"]] },
    { n: 18, t: [["        let ", "keyword"], ["found", "variable"], [" = ", "operator"], ["true", "constant.builtin.boolean"], [";", "punctuation.delimiter"]] },
    { n: 19, t: [["        let ", "keyword"], ["esc", "variable"], [" = ", "operator"], ["'", "string"], ["\\n", "constant.character.escape"], ["'", "string"], [";", "punctuation.delimiter"]] },
    { n: 20, t: [["        let ", "keyword"], ["sep", "variable"], [" = ", "operator"], ["':'", "constant.character"], [";", "punctuation.delimiter"]] },
    { n: 21, t: [["        let ", "keyword"], ["re", "variable"], [" = ", "operator"], ["Regex", "type"], ["::", "punctuation.delimiter"], ["new", "function.builtin"], ["(", "punctuation.bracket"], ['"^[a-z]+$"', "string.regexp"], [")", "punctuation.bracket"], [";", "punctuation.delimiter"]] },
    { n: 22, t: [["        let ", "keyword"], ["path", "variable"], [" = ", "operator"], ["Path", "type.builtin"], ["::", "punctuation.delimiter"], ["new", "function.builtin"], ["(", "punctuation.bracket"], ['"~/.hume"', "string.special.path"], [")", "punctuation.bracket"], [";", "punctuation.delimiter"]] },
    { n: 23, t: [["        let ", "keyword"], ["url", "variable"], [" = ", "operator"], ['"https://hume.dev"', "string.special.url"], [";", "punctuation.delimiter"]] },
    { n: 24, t: [["        if ", "keyword.control.conditional"], ["count", "variable"], [" > ", "keyword.operator"], ["0", "constant.numeric"], [" {", "punctuation.bracket"]] },
    { n: 25, t: [["            for ", "keyword.control.repeat"], ["c", "variable"], [" in ", "keyword.operator"], ["colors", "variable"], [".", "punctuation.delimiter"], ["iter", "function.method"], ["()", "punctuation.bracket"], [" {", "punctuation.bracket"]] },
    { n: 26, t: [["                println!", "function.macro"], ["(", "punctuation.bracket"], ['"{c:?}"', "string"], [")", "punctuation.bracket"], [";", "punctuation.delimiter"]] },
    { n: 27, t: [["            }", "punctuation.bracket"]] },
    { n: 28, t: [["        } else if ", "keyword.control.conditional"], ["let ", "keyword"], ["Err", "variable.builtin"], ["(", "punctuation.bracket"], ["e", "variable"], [")", "punctuation.bracket"], [" = ", "operator"], ["load", "function"], ["()", "punctuation.bracket"], [" {", "punctuation.bracket"]] },
    { n: 29, t: [["            return ", "keyword.control.return"], ["Err", "variable.builtin"], ["(", "punctuation.bracket"], ["e", "variable", "diag"], [")", "punctuation.bracket"], [";", "punctuation.delimiter"]] },
    { n: 30, t: [["        }", "punctuation.bracket"]] },
    { n: 31, t: [["        ", ""], ["Self", "constructor", "sel2"], [" { ", "punctuation.bracket"], ["name", "variable.parameter", "sel2"], [": ", "punctuation.delimiter"], ["name", "variable.parameter"], [".", "punctuation.delimiter"], ["to_string", "function.method"], ["()", "punctuation.bracket"], [", ", "punctuation.delimiter"], ["colors", "variable.other.member"], [" }", "punctuation.bracket"]] },
    { n: 32, t: [["    }", "punctuation.bracket"]] },
    { n: 33, t: [["}", "punctuation.bracket"]] },
    { n: 34, t: [] },
    { n: 35, t: [["#![", "punctuation.special"], ["allow", "keyword.directive"], ["(dead_code)]", "punctuation.special"]] },
    { n: 36, t: [["const ", "keyword.storage"], ["VERSION", "constant"], [": ", "punctuation.delimiter"], ["&str", "type.builtin"], [" = ", "operator"], ["env!", "function.special"], ["(", "punctuation.bracket"], ['"CARGO_PKG_VERSION"', "string.special.symbol"], [")", "punctuation.bracket"], [";", "punctuation.delimiter"]] },
    { n: 37, t: [["const ", "keyword.storage"], ["MAX", "constant"], [": ", "punctuation.delimiter"], ["u8", "type.builtin"], [" = ", "operator"], ["u8", "constant.builtin"], ["::", "punctuation.delimiter"], ["MAX", "constant.builtin"], [";", "punctuation.delimiter"]] },
    { n: 38, t: [["    ", ""], ["'outer", "label"], [": ", "punctuation"], ["loop ", "keyword.control.repeat"], ["{", "punctuation.bracket"]] },
    { n: 39, t: [["        let ", "keyword"], ["state", "variable.other"], [" = ", "operator"], ["read_theme", "function"], ["()", "punctuation.bracket"], [";", "punctuation.delimiter"]] },
    { n: 40, t: [["        match ", "keyword.control"], ["state", "variable.other"], [" {", "punctuation.bracket"]] },
    { n: 41, t: [["            ", ""], ["Err", "variable.builtin"], ["(", "punctuation.bracket"], ["_", "special"], [")", "punctuation.bracket"], [" => ", "operator"], ["panic!", "function.macro"], ["(", "punctuation.bracket"], ['"unreadable"', "string.special"], [")", "punctuation.bracket"], [",", "punctuation.delimiter"]] },
    { n: 42, t: [["            _ ", ""], ["=> ", "operator"], ["break ", "keyword.control.exception"], ["'outer", "label"], [",", "punctuation.delimiter"]] },
    { n: 43, t: [["        }", "punctuation.bracket"]] },
    { n: 44, t: [["    }", "punctuation.bracket"]] },
    { n: 45, t: [["}", "punctuation.bracket"]] },
    { n: 46, t: [["/* general note */", "comment"]] },
    { n: 47, t: [["/* a block comment", "comment.block"]] },
    { n: 48, t: [["   spanning two lines */", "comment.block"]] },
    { n: 49, t: [["// contains a stray ", "comment.line"], ["<200b>", "ui.virtual.invisible"], [" char", "comment.line"]] },
  ],
};

export const HTML_SAMPLE = {
  name: "page.html",
  cursorLine: 8,
  diagnostics: [{ line: 7, sev: "info", msg: "missing alt text on nested image" }],
  lines: [
    { n: 1, t: [["<!-- landing page -->", "comment"]] },
    { n: 2, t: [["<", "punctuation.bracket"], ["html", "tag.builtin"], [" ", ""], ["lang", "tag.attribute"], ["=", "operator"], ['"', "string"], ["e", "string", "cursor2"], ['n"', "string", "sel2"], [">", "punctuation.bracket"]] },
    { n: 3, t: [["  <", "punctuation.bracket"], ["head", "tag.builtin"], [">", "punctuation.bracket"]] },
    { n: 4, t: [["    <", "punctuation.bracket"], ["meta", "tag.builtin"], [" ", ""], ["charset", "tag.attribute"], ["=", "operator"], ['"utf-8"', "string"], [" />", "punctuation.bracket"]] },
    { n: 5, t: [["  </", "punctuation.bracket"], ["head", "tag.builtin"], [">", "punctuation.bracket"]] },
    { n: 6, t: [["  <", "punctuation.bracket"], ["body", "tag.builtin"], [">", "punctuation.bracket"]] },
    { n: 7, t: [["    <", "punctuation.bracket"], ["article", "tag"], [" ", ""], ["class", "tag.attribute"], ["=", "operator"], ['"hero"', "string"], [" ", ""], ["data-theme", "attribute"], ["=", "operator"], ['"sand"', "string"], [">", "punctuation.bracket"]] },
    { n: 8, t: [["      <", "punctuation.bracket"], ["h1", "tag"], [">", "punctuation.bracket"], ["H", "variable", "cursor"], ["UME", "variable", "sel"], ["</", "punctuation.bracket"], ["h1", "tag"], [">", "punctuation.bracket"]] },
    { n: 9, t: [["    </", "punctuation.bracket"], ["article", "tag"], [">", "punctuation.bracket"]] },
    { n: 10, t: [["  </", "punctuation.bracket"], ["body", "tag.builtin"], [">", "punctuation.bracket"]] },
    { n: 11, t: [["</", "punctuation.bracket"], ["html", "tag.builtin"], [">", "punctuation.bracket"]] },
  ],
};

export const MARKDOWN_SAMPLE = {
  name: "README.md",
  cursorLine: 1,
  diagnostics: [{ line: 8, sev: "hint", msg: "consider a table of contents" }],
  lines: [
    { n: 1, t: [["# ", "markup.heading.marker"], ["G", "markup.heading.1", "cursor"], ["etting started", "markup.heading.1", "sel"]] },
    { n: 2, t: [["## ", "markup.heading.marker"], ["I", "markup.heading.2", "cursor2"], ["nstallation", "markup.heading.2", "sel2"]] },
    { n: 3, t: [["### ", "markup.heading.marker"], ["Configuration", "markup.heading.3"]] },
    { n: 4, t: [["#### ", "markup.heading.marker"], ["Themes", "markup.heading.4"]] },
    { n: 5, t: [["##### ", "markup.heading.marker"], ["Scopes", "markup.heading.5"]] },
    { n: 6, t: [["###### ", "markup.heading.marker"], ["Notes", "markup.heading.6"]] },
    { n: 7, t: [] },
    { n: 8, t: [["A theme is ", ""], ["**bold**", "markup.bold"], [", ", ""], ["_italic_", "markup.italic"], [", or ", ""], ["~~struck~~", "markup.strikethrough"], [". Edit ", ""], ["`config.toml`", "markup.raw"], [" to change it.", ""]] },
    { n: 9, t: [["See ", ""], ["[the guide]", "markup.link.text"], ["(", ""], ["https://hume.dev/themes", "markup.link.url"], [")", ""], [" or ", ""], ["[helix]", "markup.link.label"], [", or just ", ""], ["<https://hume.dev>", "markup.link"], [".", ""]] },
    { n: 10, t: [["> ", ""], ["A theme is just a TOML file.", "markup.quote"]] },
    { n: 11, t: [["- ", "markup.list"], ["run ", ""], ["`hume --version`", "markup.raw.inline"]] },
    { n: 12, t: [["- [x] ", "markup.list.checked"], ["ship the default theme", ""]] },
    { n: 13, t: [["- [ ] ", "markup.list.unchecked"], ["ship a light theme", ""]] },
    { n: 14, t: [["```toml", "markup.raw.block"]] },
    { n: 15, t: [['"ui.cursor" = { bg = "blue" }', "markup.raw.block"]] },
    { n: 16, t: [["```", "markup.raw.block"]] },
    { n: 17, t: [["Table of Contents", "markup.heading"]] },
  ],
};

// The diff buffer, laid out the way HUME's own git-diff plugin renders one
// (`runtime/plugins/core/git-diff/render.scm`): a sign-column glyph per
// changed line from the bare `diff.plus`/`.minus`/`.delta` scope (Helix's own
// gutter-marker names), a row-wide `diff.plus.line`/`.minus.line`/`.delta.line`
// background (HUME's own addition — Helix reads no background for these), and
// `diff.plus.word`/`.minus.word` spans for the word-level change within it.
export const DIFF_SAMPLE = {
  name: "theme.rs (diff)",
  rows: [
    { n: 1, sign: null, rowScope: null, t: [["pub struct Theme {", ""]] },
    { n: 2, sign: { glyph: "-", scope: "diff.minus" }, rowScope: "diff.minus.line", t: [["    name: ", ""], ["C", "diff.minus.word", "sel2"], ["ow<str>", "diff.minus.word"], [",", ""]] },
    { n: 3, sign: { glyph: "+", scope: "diff.plus" }, rowScope: "diff.plus.line", t: [["    name: ", ""], ["S", "diff.plus.word", "cursor"], ["tring", "diff.plus.word"], [",", ""]] },
    { n: 4, sign: { glyph: "~", scope: "diff.delta" }, rowScope: "diff.delta.line", t: [["    colors: HashMap<String, ", ""], ["Rgb", "diff.minus.word"], [">,", ""]] },
    { n: 5, sign: null, rowScope: null, t: [["}", ""]] },
  ],
};

// Pane-level scopes rendered as chrome around whichever buffer is active —
// background, gutter, seams, cursor/selection, the statusline, virtual-text
// indicators, and diagnostics — rather than as inline token scopes. Derived
// from the matching `SCOPES` categories in `data.js` rather than
// hand-copied, so the two can't drift apart; `tests/coverage.test.js` still
// catches a scope that's editable but rendered nowhere in these samples.
const CHROME_CATEGORIES = ["UI", "Cursor", "Virtual", "Diagnostic"];
export const CHROME_SCOPES = SCOPES
  .filter(([category]) => CHROME_CATEGORIES.includes(category))
  .flatMap(([, ids]) => ids);

// Mode -> (statusline scope, cursor chain) — mirrors HUME's own mapping:
// `hume-editor/src/ui/theme.rs`'s `mode_scope`, and `cursor_cell_style` in
// `hume-engine/src/style/mod.rs`, whose match picks the chain by document
// mode: Insert -> "insert"; Extend (HUME's name for Helix's Select mode) ->
// "select"; everything else, including HUME's own Command/Search/Select
// prompt modes (which have no Helix equivalent — Helix keeps the underlying
// document mode while a prompt is open, and HUME's prompts have no
// cursor-shape option of their own) -> "normal".
//
// `barPrimary` marks Insert alone: HUME's `cursor-shape-insert` default is
// `bar`, so the buffer's primary head shows no theme color at all there — the
// real terminal bar is the sole indicator (`cursorColors` in lib/theme.js).
// Every other mode is hardwired `Block` (Normal and Extend by design; the
// three prompt modes because they render like Normal while a prompt is
// open), so their primary head always paints from its chain, the same as
// Normal — even for Command/Search, whose *own* bar cursor blinks in the
// minibuf/statusline instead (`hume-editor/src/editor/lifecycle.rs`), a fact
// with no effect on how the buffer itself renders. A secondary head is
// always painted regardless of `barPrimary` — it has no real terminal cursor
// to fall back on, matching Helix's own unconditional secondary painting.
export const MODES = [
  { scope: "ui.statusline.normal", label: "NOR", chain: "normal" },
  { scope: "ui.statusline.insert", label: "INS", chain: "insert", barPrimary: true },
  { scope: "ui.statusline.select", label: "EXT", chain: "select" },
  { scope: "ui.statusline.search", label: "SRC", chain: "normal" },
  { scope: "ui.statusline.command", label: "CMD", chain: "normal" },
  { scope: "ui.statusline.filter", label: "SEL", chain: "normal" },
];

// Overlay surfaces the pane can show on top of the buffer. Each one's scopes
// are already covered by CHROME_SCOPES above; nothing here reads them per
// overlay, so this list carries only what EditorPane actually needs to
// render the toggle and pick the right overlay body.
export const OVERLAYS = [
  { key: "none", label: "None" },
  { key: "menu", label: "Menu" },
  { key: "popup", label: "Popup" },
  { key: "picker", label: "Picker" },
  { key: "drawer", label: "Drawer" },
];

// Fuzzy-picker file list — enough rows to fill the panel's
// `min(80%, 100 cols) x min(60%, 30 rows)` footprint (`picker_panel.rs`)
// instead of leaving most of it as bare background.
export const PICKER_ROWS = [
  "hume-engine/src/theme/mod.rs",
  "hume-editor/src/ui/theme.rs",
  "hume-editor/src/ui/picker_panel.rs",
  "hume-editor/src/ui/drawer.rs",
  "hume-editor/src/ui/menu_box.rs",
  "hume-engine/src/pipeline/mod.rs",
  "hume-engine/src/style/mod.rs",
  "runtime/themes/sand.toml",
  "runtime/themes/gruvbox_light.toml",
  "runtime/plugins/core/git-diff/render.scm",
  "docs/LSP.md",
  "tools/theme-editor/src/preview/EditorPane.jsx",
];

// Drawer row list — shaped like the goto/references drawer's own rows
// (`runtime/plugins/core/lsp/lib.scm`): a `path:line:col` locator followed
// by the matched line's text.
export const DRAWER_ROWS = [
  "hume-editor/src/ui/theme.rs:42:5    pub fn mode_scope(mode: Mode) -> Scope {",
  "hume-editor/src/ui/drawer.rs:61:1   pub(crate) struct DrawerWidget {",
  "hume-editor/src/ui/picker_panel.rs:109:1  fn panel_geometry(pane_area: Rect)",
  "hume-engine/src/style/mod.rs:286:1  fn cursor_cell_style(theme, mode, is_primary)",
  "hume-engine/src/pipeline/mod.rs:344:5   let bottom_edge = area.bottom() - 1;",
  "runtime/themes/sand.toml:11:1       \"ui.drawer\" = { fg = \"text\", bg = \"surface\" }",
];

// The two panes shown in the neighbor column, in the same `{ n, t }` line
// shape as the main buffer samples. A `"head"` tag (distinct from the
// cursor/cursor2/sel/sel2 vocabulary above — a neighbor pane has no notion
// of "primary" among panes, only within itself) marks the one grapheme that
// gets that pane's own block cursor; see `NeighborPane` in `EditorPane.jsx`.
export const NEIGHBOR_TOP = [
  { n: 1, t: [["s", "variable", "head"], ["and", "variable"], [" = ", "operator"], ['"#D4A373"', "string"]] },
  { n: 2, t: [["plum", "variable"], [" = ", "operator"], ['"#ad7ca8"', "string"]] },
  { n: 3, t: [["fg", "variable"], [" = ", "operator"], ['"#F4EADC"', "string"]] },
];

export const NEIGHBOR_BOTTOM = [
  { n: 1, t: [["# ", "markup.heading.marker"], ["N", "markup.heading.1", "head"], ["otes", "markup.heading.1"]] },
  { n: 2, t: [["- ", "markup.list"], ["sand theme, dusk palette", ""]] },
];

export const BUFFERS = [RUST_SAMPLE, HTML_SAMPLE, MARKDOWN_SAMPLE, DIFF_SAMPLE];
