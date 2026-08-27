# Changelog

## Unreleased

### Appearance
- Curly, dotted and dashed underlines now render on terminals that support them. Themes could already ask for them and HUME already parsed the request; it was being flattened to a plain underline on the way to the terminal.

### Configuration & options
- New `--config <FILE>` flag loads an arbitrary Steel config file instead of the default `init.scm`; `:reload-config` re-evaluates the same file. Themes and the data directory still resolve from the standard directories. Not valid with `--keys`.

### Plugins
- `picker!` gains `#:query`, which prefills the input line and filters the (still empty, until seeded) item list against it.
- `picker!` and `live-picker!` gain `#:truncate`, which end of an over-long row the panel clips: `'head` (default, unchanged) or `'tail`, for rows whose distinguishing part sits at the front (a grep match's path, say, ahead of the line preview).
- New `live-picker!` opens a picker whose query drives an external source instead of the local fuzzy filter — a live grep, say, that re-runs its search per pattern instead of only locally filtering already-fetched rows.
- New `picker-replace!`, `picker-push!`'s sibling: replaces the open picker's item list instead of appending to it.
- `picker-source-spawn!` gains `#:ok-exit-codes` (default `'(0)`): the complete set of exit codes treated as a normal outcome, e.g. to add `rg`'s "no matches" exit to the default.
- New `picker-source-stop!` stops the open picker's attached streaming source, if any, without touching the item list — the missing half of `picker-replace!` for a live requery whose new query has nothing to spawn a replacement source for.
- New `core:stdlib` commands `stdlib/git-repo?` and `stdlib/git-toplevel` (git work-tree detection / repo-root resolution).
- New `core:stdlib` commands `stdlib/selection-anchor`, `stdlib/selection-head`, `stdlib/selection-primary?`, and `stdlib/primary-selection` — accessors for a single selection triple.
- New `core:stdlib` commands `stdlib/config-integer` (takes a minimum, `#f` for no minimum) and `stdlib/config-list` (a list of strings), rounding out `stdlib/config-boolean`/`-string`/`-enum` with the two remaining common `#:config` value shapes.
- New `set-statusline-text!` writes per-buffer text for a `"steel:<name>"` statusline element — place it with `configure-statusline!` like any built-in element, then push its content from a hook, timer, or command.

## [0.11.0] - 2026-08-25

### Breaking changes
- **Breaking**: `core:pickers` and `core:vim-keybind` now require `core:stdlib` declared or
  loaded first — their `#:config` validation moved into `core:stdlib`'s new
  `stdlib/config-boolean`/`stdlib/config-string`/`stdlib/config-enum` commands, the same
  helpers `core:git-diff` uses.
- **Breaking**: `core:lsp` now requires `core:stdlib` declared or loaded first — it scans
  installed servers via `core:stdlib`'s new `stdlib/list-subdirs` at its own load time.
  New `stdlib/run` (shared subprocess spawn, used by `core:plum`/`core:pickers`) and
  `stdlib/resolve-lang-arg` (shared `:` command language-argument resolution, used by
  `core:plum`/`core:lsp`) round out this round of plugin-internal deduplication.
- **Breaking**: `core:plum`'s plugin commands are renamed `:plum-install-plugins`,
  `:plum-cleanup-plugins`, `:plum-update-plugins`, `:plum-list-plugins` (were
  `:plum-install`, `:plum-cleanup`, `:plum-update`, `:plum-list`).
- **Breaking**: `set-inline-diagnostics!` is renamed `set-eol-text!` and now takes a `source` argument first: `(set-eol-text! source bid entries)`, matching every other decoration setter's `(set-X! source bid entries)` shape.
- **Breaking**: `set-inlay-hints!` now takes a `source` argument first — `(set-inlay-hints! source bid hints)` — and each hint's position is a plain buffer char offset instead of an LSP wire `{"line" ... "character" ...}` hashmap. Convert a wire position first with the new `lsp-position->offset`/`lsp-range->offsets` builtins.
- **Breaking**: `set-virtual-lines!`'s entries are now hashmaps (`(hash 'line ... 'text ... 'scope ... 'anchor ... 'segments ...)`) instead of positional `(line text scope)` lists, and `'segments` are char offsets, not byte offsets.
- **Breaking**: `declare-plugin`'s `#:events` entries must now be symbols (e.g. `'(on-buffer-save)`), matching `register-hook!`. The string form (`'("on-buffer-save")`) that older releases accepted is now rejected.
- **Breaking**: `(viewport-range bid)` now returns `(first-line . end-line)`, 0-based end-exclusive — `end-line` was previously the last visible line, inclusive. Drop any `(+ 1 (cdr vr))` adjustment; the pair now passes straight through as `buffer-lines`' `#:start`/`#:end`. The `on-viewport-change` hook's third argument is renamed `end-line` to match.

### Editing
- `C` now honours a count prefix: `3C` duplicates each selection onto the 3 lines below in one step instead of ignoring the count and copying onto just one.
- `.` (dot-repeat) no longer replays `C`/`copy-selection-on-{next,prev}-line` as part of a selection: it duplicates whatever selection already exists rather than establishing one, so recording it could silently drop the selection step that built the real extent.
- `core:vim-keybind`'s `C` (default `'smart` config) now takes `copy-selection-on-next-line` with any count prefix, not just when a real selection is already active.
- The kill ring now dedupes its entries.
- `p`/`P` now run new `smart-paste-after`/`smart-paste-before` commands. The old `paste-after`/`paste-before` still exist for scripting but are unbound by default and no longer have any smart-paste behavior: bare, they always read the kill-ring head with no clipboard fallback, and always replace a selection outright.
- Smart-paste now decides its source by buffer state instead of the previous command's name: the kill ring while nothing has been edited since your last delete/change/yank, the clipboard once something has. Pasting text that matches what's already selected now appends alongside it instead of replacing it.
- New `:sort` command sorts each run of adjacent selected rows by their selected text, with `-r` (reverse) and `-i` (case-insensitive) flags; numeric keys are auto-detected.

### Files & buffers
- HUME now notices when an open file changes on disk and prompts to reload the next time that buffer gets focus again; Insert mode and the command line just warn instead, and prompt on the next such focus change. Controlled by the `autoread` option (default on; `#f` warns only). Answering `[k]eep` silences the prompt until the file changes again — `:checktime` still flags it regardless. `:w`/`:wa` refuse to overwrite a changed file unless forced with `!`.

### Panes & interface
- The buffer picker (`g b`) now shows each buffer's full display path instead of a `:pwd`-relative one.
- New `core:pickers` picker, `g m`, lists files with staged or unstaged git changes. Untracked-file inclusion is configurable via `#:config (hash "untracked" #t | #f)`.
- `picker!` gains a `#:pending` flag that shows a loading indicator until the first batch of results is pushed, for pickers (like `g m`) that populate asynchronously.
- Scrollable popups and menus now show a scrollbar.
- The whole statusline now tints with the current mode's color, not just the mode indicator. Opt out with the new `statusline.mode-colors` option.
- `:messages` entries are now colored by severity.

### Configuration & options
- `:reload-config` is now a full reset: keymaps, options, hooks, commands, and plugins all go back to their defaults before `init.scm` re-runs. Buffers, undo history, and running language servers are untouched.
- `mouse-enabled`/`mouse-select` and `jump-list-capacity` now apply immediately when changed with `:set`, instead of only at startup.
- `wrap-mode` is now a buffer option: set it per file type from an `on-language-set` hook, or globally. `:set global wrap-mode=…` now applies to buffers that are already open, not just ones opened afterward; `:set pane wrap-mode=…` and `:wrap` still pin a single pane above both, but now remember that pin separately for each buffer the pane shows. `:wrap` turning wrapping back on, with nothing to restore, now falls back to the configured global style instead of always hardcoding `indent`.

### Plugins & scripting
- New `core:git-diff` plugin: live, VSCode-style inline git diff. `:toggle-git-signs` renders gutter `+`/`-`/`~` signs; `:toggle-inline-diff` renders virtual deleted lines, word-level highlights, and a background tint on changed lines — both against a configurable git ref, Requires `core:stdlib` declared or loaded first.
- New `buffer-text`/`buffer-lines` scripting builtins return a buffer's live, unsaved content — the full text, or its content lines (optionally a `#:start`/`#:end` range), excluding the phantom trailing line past the buffer's structural newline.
- New `diff-words` scripting builtin computes a word-level diff between two texts, returning 0-based char-offset hunk tuples plus a flag for when the comparison was too large to refine precisely.
- New `diff-lines`/`diff-buffer-lines` scripting builtins compute a line-level diff between two texts, or between a text and a buffer's live content, returning 0-based hunk tuples ready to feed into `set-signs!`/`set-virtual-lines!`.
- New `set-line-backgrounds!` scripting builtin sets a full-row background tint on a line, the same `(set-X! source bid entries)` shape as the other decoration setters.
- New `lsp-position->offset`/`lsp-range->offsets` scripting builtins convert a raw LSP wire position/range into a buffer char offset, or `#f` if the buffer has no attached server.
- New `on-option-change` hook fires `(key value)` after a global setting is changed via `:set global`, `set-option!`, or `:theme`.
- New `on-text-changed` hook fires `(buffer-id)` when a buffer's text changes — edits, undo, redo, `:e!` reload, and read-only view refreshes (`:messages`, `:ls`, `:plugin-status`) alike, coalesced into one fire per triggering command rather than one per underlying mutation.
- New `spawn-async!`/`cancel-async!` scripting builtins run a subprocess in the background: `callback` fires once with `(stdout stderr exit-code)` when it finishes, without blocking the editor.
- `core:steel-server` no longer flags HUME's own commands and configuration functions as unknown identifiers while you edit `init.scm` or a plugin file.
- Syntax highlighting no longer requires the `core:plum` plugin to be loaded — installed grammars register automatically at startup.
- `set-option!` can now be called from a hook or command body, not just `init.scm`.
- `get-option` takes an optional buffer id to read a specific buffer's overrides.
- New `on-buffer-enter` and `on-focus-gained` hooks: the former fires whenever the focused buffer changes, the latter when the terminal regains focus.

### Terminal & compatibility
- Quitting with Ctrl-C/SIGTERM/SIGHUP/SIGQUIT now shuts down language servers gracefully and exits with the conventional `128 + signal` code.
- Tagged release builds now show a clean `--version` string, with no commit-hash suffix.

### Fixes
- Fixed `:plum-list-plugins`/`:plum-install-plugins`/`:plum-update-plugins` raising instead of skipping a stray file (e.g. `.DS_Store`) found alongside a directory it expected to walk, in
  `<data>/plugins/<user>/<repo>/`.
- Fixed `C`/`copy-selection-on-{next,prev}-line` landing a copy one column off when a tab or wide (e.g. CJK) grapheme precedes the cursor — it now targets the same display column `9j`/`9k` land on, instead of a raw char offset.
- Fixed `C`/`copy-selection-on-{next,prev}-line` on a selection spanning more than one buffer line: it used to shift the copy just one line away, which overlapped the original and merged into it instead of duplicating it. Each copy is now offset by the selection's own line span, landing cleanly above or below it.
- Fixed a bug where the fuzzy picker silently ignored Ctrl+u/Ctrl+d; they now move the selection by half a page, matching the drawer and scrollable popups.
- Fixed a bug where a closed terminal with no controlling process could leave a HUME process spinning at 100% CPU.
- Fixed a bug where opening a `.tsx`/`.jsx` file made the language server log an "Invalid languageId" warning.
- Quitting with an attached language server no longer leaves the screen frozen in the alternate screen while it shuts down: the terminal is restored first.
- Fixed a bug where `d`/`c`/`p` on a read-only buffer could still overwrite the kill ring or a named register before refusing the edit.
- Fixed a bug where a hover or diagnostic popup stayed on screen when you scrolled or clicked with the mouse; it now closes on any mouse input, the same as on any keypress.

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
