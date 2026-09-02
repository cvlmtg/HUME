# Changelog

## Unreleased

### Breaking changes
- **Breaking**: `(set-signs! source bid signs)` entries are now `(line text scope)` — the
  trailing `priority` is gone. A sign source's gutter column now comes from a new
  `(register-sign-source! name bid priority)` call instead, scoped to that one buffer and
  made every time the source is about to place or clear signs there (idempotent, so
  repeating it is cheap) rather than once globally; `set-signs!` for a source unregistered
  for that buffer now errors instead of silently rendering nothing. A source's gutter slot
  is per-buffer — a buffer neither `core:lsp` nor `core:git-diff` (nor any other plugin)
  ever registers a sign source for never reserves a gutter column for one.
- **Breaking**: `(selection-spans-full-line? bid)` is now `(selections-linewise? bid)`: it
  checks every selection in the buffer instead of just the primary one, and each selection
  may now span any number of whole lines instead of exactly one. New `(selections-charwise?
  bid)` complements it — `#t` when none of `bid`'s selections is linewise — so `:lsp-fmt`'s
  three-way verdict (all linewise / none linewise / mixed) is two plain predicates rather than
  one boolean plus an inferred third state. A collapsed cursor sitting alone on a blank line
  doesn't count toward either predicate — it neither breaks an otherwise-linewise set into
  "mixed" nor counts as a deliberate whole-line selection on its own — and `#f` from
  `selections-linewise?` (`#t` from `selections-charwise?`) when every selection is such a
  cursor, matching a bare cursor's usual behavior.
- **Breaking**: `(lsp-range-params bid)` is now `(lsp-primary-range-params bid)` (same shape,
  from `bid`'s primary selection alone), plus a new `(lsp-linewise-ranges-params bid)` that
  returns one wire range per linewise selection in `bid`'s buffer (touching selections
  coalesced into one range apiece), empty if none are linewise. `:lsp-fmt` now formats a set of
  disjoint linewise selections as several ranges instead of falling back to the whole buffer:
  one `textDocument/rangesFormatting` request (LSP 3.18) when the server advertises
  `rangesSupport`, otherwise one `rangeFormatting` request per range — capped at the new
  `lsp.format-max-ranges` setting (default 16), past which nothing is formatted, with a
  warning naming the cap. A mix of whole-line and partial-line selections now warns and
  formats nothing, rather than silently reformatting the whole buffer.

### Editing
- New `goto-matching-pair` (`#`) jumps between a bracket and its partner (`(` `)` `[` `]` `{` `}`), or between an HTML/XML/JSX tag and its partner — vim's `%`, without disturbing HUME's own `%` (select-all). For single line selections, it scans for brackets against the whole selection, not just the character the cursor sits on — so `#` still jumps after a motion like `w` leaves the cursor on the whitespace past a bracket rather than on the bracket itself.
- `w`/`b`, `mm`, `miw`/`maw`, `select-word-nearest-on-line`, `Ctrl+W`, `*`, quote auto-pairing, the identifier under the cursor used by plugin commands (e.g. rename), and the LSP completion fallback replace span now honor a buffer's configured `word-chars` (see the new setting below) — e.g. with `-` configured, `foo-bar` is one word instead of three. Bracket pairs are unaffected: only pairs whose opening and closing character are the same (`'`, `"`, `` ` ``) skip auto-pairing after a word character. `W`/`B`/`MM` are unaffected: they already treat punctuation and word characters as one class. With `word-chars` configured, `*` can now still bleed into a longer run sharing the same edge character (e.g. searching `foo-bar` inside `foo-bar-baz` also matches there).
- New `indent`/`unindent` (`>`/`<`) shift every line touched by a selection by one indent level (a count shifts by that many, e.g. `3>`), in the buffer's `tab-width`/`tab-style`. Blank and whitespace-only lines are left alone. Each touched line's whole indent is re-rendered to the new width rather than just prepended to or trimmed from, so `<` immediately after `>` restores the previous indent width exactly (re-rendered in the buffer's `tab-style`, so a mixed tabs-and-spaces indent normalizes as a side effect rather than coming back byte-identical). `<` on an indent narrower than one level flattens it to the left margin rather than going negative, so `>` afterwards lands on a full level, not back where `<` started.
- A numeric count prefix (`3w`, `12j`) is now capped at 10,000, whether typed or supplied by a script's `(call! "cmd" count)`. A large count no longer risks an overflow, and no longer slows down motions like `w`/`h`/`l` past their buffer clamp — they now stop as soon as the motion stops moving instead of repeating the full count.
- Pasting or inserting a vertical tab, form feed, NEL, or Unicode line/paragraph separator character no longer splits the buffer into an extra editor line — it's ordinary content now, rendered like any other control character, matching every other editor and the line-break definition language servers use.
- Every line-ending convention now normalizes to `\n` wherever text enters a buffer, not just at load: pasting, `p`/`P` register paste, a language server's edit or completion, and a plugin's own insertion all collapse a `\r\n` or a bare `\r` (old Mac) the way file load already did for `\r\n`. A buffer's lines always end in `\n` regardless of where the text came from. A file written with bare `\r` line endings is still read correctly, but that convention is not preserved on save: it loads as `LF` and saves with `\n`.
- New tree-sitter structural text objects, for a language whose grammar ships a `textobjects.scm`
  (PLUM installs one alongside highlights where the upstream grammar has one): `m i f`/`m a f`
  (function), `m i t`/`m a t` (class/type), `m i c`/`m a c` (comment), `m i T`/`m a T` (test),
  `m i e`/`m a e` (array/tuple/struct entry). Each is a silent no-op without a matching grammar.
- `m i a`/`m a a` (argument) is now structure-aware: where the grammar defines a `parameter` object
  it's used in preference to the lexical scan, which still covers everything the grammar doesn't (a
  region under a syntax error, a language with no grammar at all). This changes what counts as "the
  argument" inside a call: a nested list, tuple, or struct literal is now one argument rather than
  the lexical scan's innermost comma-delimited fragment — reach its members with `m i e`/`m a e`.
- New unbound `goto-next-<kind>`/`goto-prev-<kind>` commands, one pair per structural kind above
  plus `goto-next-argument`/`goto-prev-argument`, select the next/previous object of that kind as a
  whole selection with the cursor on its start; they don't wrap past either end of the buffer and
  each records a jump-list entry (`Ctrl+o` returns). Bind them yourself, e.g. `(bind-key! 'normal
  "g f" "goto-next-function")`; they also run unbound from the command line, e.g.
  `:goto-next-function`.

### Appearance
- Curly, dotted and dashed underlines now render on terminals that support them. Themes could already ask for them and HUME already parsed the request; it was being flattened to a plain underline on the way to the terminal.

### Configuration & options
- New `--config <FILE>` flag loads an arbitrary Steel config file instead of the default `init.scm`; `:reload-config` re-evaluates the same file. Themes and the data directory still resolve from the standard directories. Not valid with `--keys`.
- Command-line file arguments now accept a trailing `:LINE` or `:LINE:COLUMN` position — `hume src/main.rs:42:5` opens the file with the cursor placed there, matching the `file:line:col` shape most tools print in diagnostics. A path that exists on disk exactly as typed always opens as-is, so a file genuinely named with a colon is unaffected.
- New buffer option `word-chars` (Vim's `iskeyword`, minus the range syntax): extra characters counted as part of a word. Ships with no default set — configure it per language from an `on-language-set` hook (see the manual's [Configuration](https://cvlmtg.github.io/HUME/configuration.html) page).

### Plugins
- `register-grammar!` gains an optional 6th positional argument, a `textobjects.scm` path. A
  language that defines textobjects but nothing embedded passes `#f` for the 5th argument
  (`injections-path`) to reach it.
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
- `core:git-diff` now also drives a `"steel:git-branch"` statusline element for the focused buffer — place it with `configure-statusline!` and it shows the current branch, e.g. `(main)`, with no extra Steel required.

### Fixes
- `Ctrl+O`/`Ctrl+I` now land on the text you jumped from rather than on whatever happens to sit at the old offset after you edit, undo, or reload the file (`:e!`). Re-running `:messages`/`:ls` drops that view's stale jump stops instead of leaving them pointing into regenerated content.
- `p` on an empty line now pastes charwise register content onto that line instead of at the start of the next one.
- `:reload-config` no longer leaves an `on-buffer-enter`-driven `"steel:<name>"` statusline element (e.g. `core:git-diff`'s `steel:git-branch`) blank until the next buffer switch or save — it now re-fires `on-buffer-enter` for the focused buffer as part of its usual buffer-lifecycle replay.
- The sign column no longer shuffles a channel's marker sideways depending on what else shares its line, and no longer changes width as a channel's individual signs come and go. `signcolumn` (bare `always`/`auto`, no `:N`) now sizes the column to one slot per registered sign source (see `register-sign-source!` above), reserved the moment a plugin registers rather than derived from whatever priorities happen to be present in the buffer right now; pin `signcolumn=always:N`/`auto:N` for a fixed width, as before.
- Diagnostic gutter markers no longer render underlined. They were interning the same scope as the text-span squiggle, which every bundled theme underlines; the gutter now reads its own scope (`error`/`warning`/`info`/`hint`).
- `m/` (select all search matches) and `ms` (surround selection, e.g. `ms(`) followed by an edit are now replayable with `.` — they previously replayed only the edit against whatever selection happened to remain, instead of re-running the selection step (every current search match, or the next surrounding delimiter pair) before repeating the edit.
- `C` (`copy-selection-on-next-line`/`-prev-line`) preceding an edit is now replayable with `.` — it previously replayed only the edit against whatever selection happened to remain, instead of re-duplicating the selection onto the adjacent line first.
- The bracket-match cursor highlight (`ui.cursor.match`) no longer treats `<`/`>` as a pair — a cursor on either in `Vec<String>` or `a < b` no longer highlights the other as if they matched.
- `,` (keep only the primary selection), `S` (split a selection into one per line), `_` (trim whitespace from a selection), `(`/`)` (cycle the primary selection), `Ctrl+,` (remove the primary selection), and `Ctrl+e` (flip anchor/head) preceding an edit are now replayable with `.` — each previously replayed only the edit against whatever selection happened to remain, instead of re-running the step that narrowed or reshaped the selection first.
- `select-word-nearest-on-line` (bound by plugins, not a default key) is now replayable with `.` when it precedes an edit.
- `m/` with a search pattern matching nothing, or `ms`/`ma`/`mi` finding no surrounding pair, no longer discards a selection step an earlier command in the same sequence had already built before an edit — `.` now re-runs that earlier step instead of replaying the edit against whatever selection happens to remain.
- The alternate buffer (`Ctrl+6`/`goto-alternate-file`, `#`/`:b#`) now follows the order buffers were last visited rather than the order they were opened, so it keeps toggling with the buffer you actually came from after jumping around with a picker or another pane instead of falling back to whichever buffer opened just before the current one.
- `:lsp-fmt` now range-formats a multi-line selection that spans only complete lines, instead of silently formatting the whole document.
- `:split`/`:vsplit` now resize every pane sharing that split axis to an equal size, instead of halving whatever pane was split (three `:vsplit`s in a row now gives three equal columns, not 50/25/25). Closing a pane redistributes its space equally between the survivors rather than handing it all to one neighbour.

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
