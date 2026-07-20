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
- Character prompts (`r`, `t`, `f`, …) accept Enter and Tab.
- Extend-mode `o` (flip selection) moved into the `core:vim-keybind`
  plugin.

### Panes & interface
- Configurable sign column in the gutter (`signcolumn` option).
- `Diagnostics` statusline element, in the default statusline.

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

### Theming
- Theme editor rebuilt as a proper web app.
- `sand` theme refinements.

### Fixes
- `:wq` quit the whole editor instead of closing the focused pane.
- Crash when joining lines with a cursor on the last line.
- Syntax highlighting precedence for overlapping captures.
- Busy-loop when the terminal hung up.
- Kitty keyboard protocol disabled on Windows, where support is
  unreliable.

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
