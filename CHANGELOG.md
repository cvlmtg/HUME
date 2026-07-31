# Changelog

## Unreleased
- `core:steel-server` no longer flags HUME's own commands and configuration functions as unknown identifiers while you edit `init.scm` or a plugin file. `register-lsp-server!` gained a new `#:env` keyword for passing extra environment variables to a spawned server.
- The buffer picker (`g b`) now shows each buffer's full display path instead of a `:pwd`-relative one.
- New `core:pickers` picker, `g m`, lists files with staged or unstaged git changes. Untracked-file inclusion is configurable via `#:config (hash "untracked" #t | #f)` (default on).
- HUME now notices when an open file changes on disk and prompts to reload it. Controlled by the `autoread` option (default on); `:w`/`:wa` refuse to overwrite a changed file unless forced with `!`.
- New `:sort` command sorts each run of adjacent selected rows by their selected text, with `-r` (reverse) and `-i` (case-insensitive) flags; numeric keys are auto-detected.
- Scrollable popups and menus now show a scrollbar.
- The whole statusline now tints with the current mode's color, not just the mode indicator. Opt out with the new `statusline.mode-colors` option.
- `:reload-config` is now a full reset: keymaps, options, hooks, commands, and plugins all go back to their defaults before `init.scm` re-runs. Buffers, undo history, and running language servers are untouched.
- Syntax highlighting no longer requires the `core:plum` plugin to be loaded — installed grammars register automatically at startup.
- `set-option!` can now be called from a hook or command body, not just `init.scm`.
- `get-option` takes an optional buffer id to read a specific buffer's overrides.
- Quitting with Ctrl-C/SIGTERM/SIGHUP/SIGQUIT now shuts down language servers gracefully and exits with the conventional `128 + signal` code.
- Fixed a bug where a closed terminal with no controlling process could leave a HUME process spinning at 100% CPU.
- `mouse-enabled`/`mouse-select` and `jump-list-capacity` now apply immediately when changed with `:set`, instead of only at startup.
- Fixed a bug where opening a `.tsx`/`.jsx` (or other Helix-vs-LSP-spelling-mismatched) file made the language server log an "Invalid languageId" warning and silently correct it. `define-language!` gained a `#:language-id` keyword to override the wire identifier when it differs from the language name; bundled languages whose Helix name and LSP identifier differ (`tsx`, `jsx`, `hcl`, `tfvars`, `docker-compose`, `docker-bake`, `quarto`, `robot`, `rmarkdown`) now carry the correct override.

## [0.10.0] - 2026-07-24

### Language servers
- Full LSP support: hover, goto definition, references, rename, code actions, formatting, signature help.
- In-buffer autocompletion.
- Diagnostics: underlines, gutter signs, inline messages, statusline counts, `gn`/`gp` navigation, `:diagnostics` list.
- Inlay hints.
- One-command server installation (`:lsp-install`) from a bundled catalog.
- Manual server registration from config via `register-lsp-server!`.
- `core:steel-server` plugin: a Steel language server for editing your own config and plugins.
- Hover docs and other overflow popup/drawer content are syntax-highlighted, with scroll affordance arrows shown when there's more to see.

### Editing
- After changing text with `c`, leaving Insert mode now selects the text you just typed. Controlled by the new `select-changed-text` option (default on).
- `mii` selects the last insertion.
- Case transforms: `gu`, `gU`, `gC`.
- Word motions and `mm`/`MM` now also select adjacent whitespace. Controlled by the new `word-selects-whitespace` option (default on).
- New `undo-levels` option caps the number of undo states kept per buffer (default `0`, unlimited).
- Character prompts (`r`, `t`, `f`, …) accept Enter and Tab.
- Extend-mode `o` (flip selection) moved into the `core:vim-keybind` plugin.

### Panes & interface
- Fuzzy file and buffer finders (`core:pickers` plugin): `g f` / `g b`. Files picker reads the git index when in a repo, falls back to `fd`. Any plugin can build its own picker over the same `picker!` / `picker-source-spawn!` API.
- Configurable sign column in the gutter (`signcolumn` option).
- `Diagnostics` statusline element, in the default statusline.
- Indentation guides can now be hidden via the new `indent-guides` option (default on).

### Plugins & scripting
- `manifest.scm`: a bare `(declare-plugin "name")` is enough for lazy loading.
- Expanded plugin API: LSP requests, timers, new hooks, decorations, popups, menus, drawer lists, minibuffer prompts.
- `set-buffer-option!` builtin for per-buffer setting overrides from hooks and commands.

### Terminal & compatibility
- Bracketed paste — pastes land in one step, without auto-pairing.
- Event-driven main loop: HUME sleeps when idle instead of polling.
- Switched terminal I/O from crossterm to termina. Kitty keyboard protocol now works on Windows (Windows Terminal ≥ 1.25): input decoding is now identical across platforms.

### Theming
- Theme editor rebuilt as a proper web app.
- `sand` theme refinements.

### Fixes
- `:wq` quit the whole editor instead of closing the focused pane.
- Crash when joining lines with a cursor on the last line.
- Syntax highlighting precedence for overlapping captures.
- `(set-option! ...)` from a lazily-activated plugin now takes effect immediately.
- `:set global theme=<name>` matches `:theme <name>`'s existing behavior.
- Minibuffer history recall (Up/Down in `:`, `/`, `?`) now filters to entries starting with the text typed before recalling.
- Statusline shows `*scratch*` and other synthetic buffer names instead of leaving the file-path element blank.
- Windows: statusline file paths and `:e`'s duplicate-buffer detection no longer choke on the `\\?\` canonical-path prefix.

## [0.9.0] - 2026-07-15

First tagged release. HUME has been under active development for a while; this is the point where it's considered stable enough to hand out prebuilt binaries rather than requiring a build from source.

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
