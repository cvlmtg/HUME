# Changelog

## Unreleased

### Language servers
- Full LSP support: hover, goto definition, references, rename, code
  actions, formatting, signature help.
- In-buffer autocompletion.
- Diagnostics: underlines, gutter signs, inline messages, statusline
  counts, `gn`/`gp` navigation, `:diagnostics` list.
- Inlay hints.
- One-command server installation (`:lsp-install`) from a bundled catalog.
- Manual server registration from config via `register-lsp-server!`.
- `core:steel-server` plugin: a Steel language server for editing your own
  config and plugins.

### Editing
- After changing text with `c`, leaving Insert mode now selects the text
  you just typed. Controlled by the new `select-changed-text` option
  (default on).
- `mii` selects the last insertion.
- Case transforms: `gu`, `gU`, `gC`.
- Word motions and `mm`/`MM` now also select adjacent whitespace.
  Controlled by the new `word-selects-whitespace` option (default on).
- New `undo-levels` option caps the number of undo states kept per buffer
  (default `0`, unlimited).
- Character prompts (`r`, `t`, `f`, …) accept Enter and Tab.
- Extend-mode `o` (flip selection) moved into the `core:vim-keybind`
  plugin.

### Panes & interface
- Fuzzy file and buffer finders (`core:pickers` plugin): `g f` / `g b`.
  Files picker reads the git index when in a repo, falls back to `fd`.
  Any plugin can build its own picker over the same `picker!` /
  `picker-source-spawn!` API.
- Configurable sign column in the gutter (`signcolumn` option).
- `Diagnostics` statusline element, in the default statusline.
- Indentation guides can now be hidden via the new `indent-guides` option
  (default on).

### Plugins & scripting
- Full-trust plugin model: no more sandbox; plugins use Steel's standard
  library for process and file access.
- `manifest.scm`: a bare `(declare-plugin "name")` is enough for lazy
  loading.
- Expanded plugin API: LSP requests, timers, new hooks, decorations,
  popups, menus, drawer lists, minibuffer prompts.

### Terminal & compatibility
- Bracketed paste — pastes land in one step, without auto-pairing.
- Event-driven main loop: HUME sleeps when idle instead of polling.
- Switched terminal I/O from crossterm to termina. Kitty keyboard protocol
  now works on Windows (Windows Terminal ≥ 1.25): input decoding is now
  identical across platforms, so a probe reporting kitty support means it
  actually works, not just that the terminal answered a query. Windows
  requires 10 1809+ and a VT-capable console (no legacy-conhost fallback);
  raw mintty without winpty is unsupported, unchanged from before.
- Fixed: held-key autorepeat under the kitty protocol (`REPORT_EVENT_TYPES`)
  never matched a keymap binding — the trie only ever recorded `Press`.

### Theming
- Theme editor rebuilt as a proper web app.
- `sand` theme refinements.

### Fixes
- `:wq` quit the whole editor instead of closing the focused pane.
- Crash when joining lines with a cursor on the last line.
- Syntax highlighting precedence for overlapping captures.
- Busy-loop when the terminal hung up.
- `(set-option! ...)` from a lazily-activated plugin now takes effect
  immediately instead of silently doing nothing until the next `:set`.
- `:set global theme=<name>` no longer keeps a theme name that failed to
  load — it's rolled back, matching `:theme <name>`'s existing behavior.

## [0.9.0] - 2026-07-15

First tagged release. HUME has been under active development for a while;
this is the point where it's considered stable enough to hand out prebuilt
binaries rather than requiring a build from source.

### Editing model
- Multiple selections and multi-cursor editing.
- Full Unicode correctness.
- Registers and kill ring, with system-clipboard integration.
- Keyboard macros, count prefixes, dot-repeat, undo/redo tree.
- Incremental search, search-based multi-cursor selection, jump list.

### Syntax & language awareness
- Tree-sitter-powered syntax highlighting.
- Multi-layer language injections.
- Grammar installation and management built in.

### Plugins & scripting
- Steel (Scheme) scripting for configuration and plugins.
- Built-in plugin manager (PLUM) for installing and managing plugins.
- Lazy plugin loading.

### Panes & interface
- Split panes with directional focus movement, seam dividers, and focus dimming.
- Tab completion in the command line.
- Multi-buffer workflow.
- Configurable status line.

### Theming
- Hierarchical, scope-based theming compatible with Helix themes.

### Terminal & compatibility
- True color and synchronized output by default.
- Kitty keyboard protocol support with automatic fallback to legacy key encoding.
- Runs on macOS, Linux, and Windows.
