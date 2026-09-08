# Changelog

## Unreleased

### Breaking changes
- `}`/`{` now select the whole paragraph (plus its trailing blank line) instead of just moving the cursor, consistent with other structural motions. New `mip`/`map` text objects select just the paragraph when a plain selection is wanted.
- Case transforms moved from `gu`/`gU`/`gC` to `GL`/`GU`/`GC`.
- Several default keys moved to keep `g` reserved for goto motions: fuzzy finders move from `gf`/`gb`/`gm` to `zf`/`zb`/`zm`, rename moves from `gr` to `GR`, hover moves to bare `K`, and the viewport keys become `zk`/`zz`/`zj`.
- The vim-keybind plugin no longer binds `G`, since it was overriding HUME's own case-transform keys.
- `goto-alternate-file` is renamed `goto-alternate-buffer`.
- `:format-source` (renamed from `:lsp-fmt`) can now format several selected line ranges at once instead of falling back to the whole buffer when a server supports it.
- A handful of commands (LSP install/status, PLUM plugin/grammar management, git-diff sign toggles, `:lsp-fmt` → `:format-source`) can no longer be typed at the `:` prompt by their old bindable name — bind them to a key instead, or use their new typed command name.
- Plugins that place gutter signs (LSP diagnostics, git signs) now reserve their gutter column per buffer instead of globally. If you write your own: `(set-signs! …)` entries are `(line text scope)`, without the trailing priority — a source now declares its column with `(register-sign-source! name bid priority)` before placing or clearing signs in that buffer.
- Two more scripting renames, if you write your own plugins: `(selection-spans-full-line? bid)` is now `(selections-linewise? bid)` and checks every selection instead of only the primary one, with a new `(selections-charwise? bid)` as its counterpart; `(lsp-range-params bid)` is now `(lsp-primary-range-params bid)`, joined by a new `(lsp-linewise-ranges-params bid)` returning one range per linewise selection.
- HUME's own Select mode (the `s` regex prompt) is renamed **Sift mode**, to stop colliding with Extend mode (HUME's name for what Helix calls Select mode). The status bar shows `SIF` instead of `SEL`; the `select-within` command is renamed `sift-within`.

### Editing
- New `#` jumps between a bracket or tag and its matching partner — vim's `%`, without disturbing HUME's own `%` (select-all).
- Word motions, `miw`/`maw`, `Ctrl+W`, `*`, and quote auto-pairing now honor the new `word-chars` setting, so e.g. `foo-bar` can be treated as one word instead of three.
- New `>`/`<` indent/unindent every selected line by one level (`3>` for three levels).
- Count prefixes (`3w`, `12j`) are capped at 10,000.
- Pasting or typing unusual line-break characters no longer splits the buffer into an extra line.
- Files with old-style line endings (`\r\n`, bare `\r`) are normalized to `\n` everywhere text enters the buffer, not just on load.
- New tree-sitter based text objects for functions, types, comments, unit tests, and array/struct values (`mif`/`maf`, `mit`/`mat`, `mic`/`mac`, `miu`/`mau`, `miv`/`mav`), for any language with textobject support.
- `mia`/`maa` (argument) is now smarter about nested lists, tuples, or structs inside a call.
- New `goto-next-`/`goto-prev-` navigation for each text-object kind above, bound under `g` on the same letter (e.g. `gf`/`gF` for functions), also available from the command line.
- `}` and forward structural navigation now re-center the view on the selected object instead of leaving it at the bottom of the screen (new `object-jump-align` setting).

### Files & buffers
- New `goto-next-buffer`/`goto-prev-buffer` commands, for binding to a key.

### Appearance
- New `cursor-shape-insert` setting (`block`/`bar`/`underline`, default `bar`) picks the real terminal cursor's shape in Insert mode.
- Curly, dotted, dashed, and now double-line underlines render correctly on terminals that support them.
- Whitespace indicator glyphs (spaces, tabs, newlines) are now themable.
- Themes can now also be installed to the data directory (see `:plum-install-theme` below) alongside hand-authored ones.
- Inline diagnostic messages now use their own theme color instead of always matching the underlined squiggle.
- Replaced the bundled `dark`/`light`/`gruvbox` themes with faithful ports of Helix's `gruvbox` and `gruvbox_light`. Bundled themes are now `sand`, `gruvbox`, `gruvbox_light`.
- Themes can now use TOML section headers as well as HUME's flat dotted-key format.
- The theme loader is more Helix-compliant in several places (search-match highlighting, statusline mode colors, underline/modifier names, git-diff's gutter and line colors) — HUME's bundled themes already reflect this.
- The pane seam divider now falls back to the theme's own base text color when `ui.window` sets only a background, instead of leaving the divider's foreground unthemed.

### Configuration & options
- New `--config <FILE>` flag loads a config file other than the default `init.scm`; `:reload-config` re-evaluates the same file.
- File arguments can include a `:LINE` or `:LINE:COLUMN` suffix to open at that position (e.g. `hume src/main.rs:42:5`).
- New `word-chars` buffer option configures extra characters treated as part of a word, settable per language.

### Plugins
- New scripting support for structural text objects, live/streaming pickers, git helpers, and a per-buffer statusline element plugins can write to.
- `core:git-diff` can now show the current git branch in the statusline.
- Clearer error messages when a key or `:` command targets a command that isn't reachable that way.
- New `:plum-install-theme`, `:plum-update-themes`, `:plum-list-themes`, and `:plum-remove-theme` manage third-party themes from GitHub, the same way PLUM already manages plugins and grammars.

### Fixes
- "Nothing to do here" refusals (no search match, unknown buffer name, mistyped setting, …) now just flash a message instead of being logged as errors.
- `Ctrl+O`/`Ctrl+I` jumps now land correctly even after the buffer was edited, undone, or reloaded.
- `p` on an empty line now pastes onto that line instead of the next one.
- `:reload-config` no longer leaves plugin-driven statusline elements (e.g. the git branch) blank until the next buffer switch or save.
- The sign column no longer shifts or resizes as gutter signs come and go.
- Diagnostic gutter markers no longer render underlined.
- `.` (repeat) now correctly replays the selection step, not just the edit, for: `m/` (select all matches), `ms` (surround), `C` (copy selection to adjacent line), `,` (keep primary selection), `S` (split into lines), `_` (trim whitespace), `(`/`)` (cycle selection), `Ctrl+,` (remove primary selection), `Ctrl+e` (flip anchor/head), and `select-word-nearest-on-line`.
- The bracket-match highlight no longer treats `<`/`>` as a pair, so it no longer misfires on generics or comparisons.
- The alternate buffer (`Ctrl+6`, `#`, `:b#`) now follows visit order, so it keeps toggling with the buffer you actually came from.
- `:format-source` now correctly range-formats a multi-line selection instead of formatting the whole document.
- `:split`/`:vsplit` now resize every pane on that axis equally instead of just halving the one being split; closing a pane redistributes its space evenly.

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
- Fixed `.` after `[`/`]` (paste-ring cycling) being a permanent no-op: `.` closed the still-open paste session before replaying the cycle, so the cycle itself never ran, and every following `.` inherited the same dead state.
- Fixed `.` overwriting the last repeatable action with a no-op when the command it replayed was refused on a read-only buffer — the real action it replaced is now preserved instead.

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
